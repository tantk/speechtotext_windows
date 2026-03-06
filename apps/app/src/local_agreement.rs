//! LocalAgreement-based text confirmation for streaming transcription/translation.
//!
//! Based on the approach from ufal/whisper_streaming:
//! - Compare consecutive Whisper outputs at the word level
//! - Confirm the longest common prefix across n consecutive passes
//! - Track timestamps for buffer trimming

use tracing::{debug, info};

/// A word with its start and end timestamps (in seconds relative to audio buffer start).
#[derive(Debug, Clone)]
pub struct TimestampedWord {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

/// Parse Whisper's timestamped output into words with timestamps.
///
/// Whisper with timestamps enabled produces text like:
///   `<|0.00|> Hello world<|2.50|><|2.50|> how are you<|5.00|>`
///
/// Each `<|X.XX|>` is a timestamp token. Text between two timestamps
/// belongs to that time range.
pub fn parse_timestamped_text(text: &str) -> Vec<TimestampedWord> {
    let mut words = Vec::new();
    let mut pos = 0;
    let bytes = text.as_bytes();
    let len = bytes.len();

    // State: look for pairs of timestamps with text between them
    let mut last_ts: Option<f64> = None;

    while pos < len {
        // Try to parse a timestamp token <|X.XX|>
        if pos + 5 < len && &text[pos..pos + 2] == "<|" {
            if let Some(end_pipe) = text[pos + 2..].find("|>") {
                let ts_str = &text[pos + 2..pos + 2 + end_pipe];
                if let Ok(ts) = ts_str.parse::<f64>() {
                    if let Some(start) = last_ts {
                        // We have a start timestamp and now an end timestamp
                        // Any text accumulated between them is a segment
                        // (text was already captured below)
                        last_ts = Some(ts);
                    } else {
                        last_ts = Some(ts);
                    }
                    pos = pos + 2 + end_pipe + 2; // skip past |>
                    continue;
                }
            }
        }

        // Accumulate text between timestamps
        if let Some(start_ts) = last_ts {
            // Find the next timestamp or end of string
            let text_start = pos;
            while pos < len {
                if pos + 5 < len && &text[pos..pos + 2] == "<|" {
                    if let Some(end_pipe) = text[pos + 2..].find("|>") {
                        let ts_str = &text[pos + 2..pos + 2 + end_pipe];
                        if ts_str.parse::<f64>().is_ok() {
                            break; // Found next timestamp
                        }
                    }
                }
                pos += 1;
            }

            let segment_text = text[text_start..pos].trim();
            if !segment_text.is_empty() {
                // Now parse the end timestamp
                let end_ts = if pos + 5 < len && &text[pos..pos + 2] == "<|" {
                    if let Some(end_pipe) = text[pos + 2..].find("|>") {
                        let ts_str = &text[pos + 2..pos + 2 + end_pipe];
                        if let Ok(ts) = ts_str.parse::<f64>() {
                            pos = pos + 2 + end_pipe + 2;
                            last_ts = Some(ts);
                            ts
                        } else {
                            start_ts + 1.0 // fallback
                        }
                    } else {
                        start_ts + 1.0
                    }
                } else {
                    start_ts + 1.0
                };

                // Split segment into individual words, distributing timestamps evenly
                let word_texts: Vec<&str> =
                    segment_text.split_whitespace().collect();
                if !word_texts.is_empty() {
                    let duration = end_ts - start_ts;
                    let per_word = duration / word_texts.len() as f64;
                    for (i, w) in word_texts.iter().enumerate() {
                        words.push(TimestampedWord {
                            text: w.to_string(),
                            start: start_ts + i as f64 * per_word,
                            end: start_ts + (i + 1) as f64 * per_word,
                        });
                    }
                }
            }
        } else {
            pos += 1;
        }
    }

    words
}

/// Tracks confirmed vs unconfirmed words across consecutive Whisper passes.
pub struct LocalAgreement {
    /// Words confirmed by n consecutive agreeing passes
    confirmed: Vec<TimestampedWord>,
    /// Previous pass output (for comparison)
    prev_words: Vec<TimestampedWord>,
    /// Number of agreeing passes required (default: 2)
    n: usize,
    /// How many consecutive passes have agreed on current prefix
    agree_count: usize,
    /// Text that has been emitted (typed/displayed)
    emitted_text: String,
    /// Timestamp of the last confirmed word (for buffer trimming)
    last_confirmed_end: f64,
}

impl LocalAgreement {
    pub fn new() -> Self {
        Self {
            confirmed: Vec::new(),
            prev_words: Vec::new(),
            n: 2,
            agree_count: 0,
            emitted_text: String::new(),
            last_confirmed_end: 0.0,
        }
    }

    /// Process a new Whisper output. Returns:
    /// - `confirmed_new`: newly confirmed text to emit (type into window)
    /// - `unconfirmed`: text that's not yet stable (show on subtitle)
    /// - `trim_to`: if Some, the audio buffer can be trimmed up to this timestamp
    pub fn process(&mut self, text: &str) -> AgreementResult {
        let words = parse_timestamped_text(text);

        if words.is_empty() {
            return AgreementResult {
                confirmed_new: String::new(),
                full_text: String::new(),
                trim_to: None,
            };
        }

        // Find longest common prefix between this output and previous
        let common_len = self.common_prefix_len(&words);

        if common_len > 0 && !self.prev_words.is_empty() {
            self.agree_count += 1;
        } else {
            self.agree_count = 1;
        }

        // Build full text from this pass
        let full_text: String = words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");

        let mut result = AgreementResult {
            confirmed_new: String::new(),
            full_text,
            trim_to: None,
        };

        // If enough passes agree, confirm the common prefix
        if self.agree_count >= self.n && common_len > self.confirmed.len() {
            let new_confirmed = &words[self.confirmed.len()..common_len];

            // Find sentence boundaries in newly confirmed text for trimming
            let new_text: String = new_confirmed
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");

            if !new_text.is_empty() {
                // Check if there's a complete sentence in the new confirmed text
                let sentence_end = find_sentence_end(&new_text);

                if sentence_end > 0 {
                    // We have a complete sentence — emit it and set trim point
                    let sentence_text = &new_text[..sentence_end];
                    result.confirmed_new = sentence_text.to_string();

                    // Find the timestamp of the last word in the confirmed sentence
                    let sentence_word_count = sentence_text.split_whitespace().count();
                    let trim_word_idx = self.confirmed.len() + sentence_word_count;
                    if trim_word_idx > 0 && trim_word_idx <= words.len() {
                        let trim_ts = words[trim_word_idx - 1].end;
                        result.trim_to = Some(trim_ts);
                        self.last_confirmed_end = trim_ts;
                    }

                    // Update confirmed to include the sentence words
                    self.confirmed
                        .extend_from_slice(&words[self.confirmed.len()..trim_word_idx.min(words.len())]);

                    info!(
                        "LocalAgreement: confirmed sentence \"{}\" (trim_to={:.2}s)",
                        sentence_text,
                        result.trim_to.unwrap_or(0.0)
                    );
                } else {
                    // Words confirmed but no complete sentence yet — don't trim
                    debug!(
                        "LocalAgreement: {} words agreed but no sentence boundary yet",
                        common_len
                    );
                }
            }
        }

        self.prev_words = words;
        result
    }

    /// Reset state (call when always-listen recording restarts)
    pub fn reset(&mut self) {
        self.confirmed.clear();
        self.prev_words.clear();
        self.agree_count = 0;
        self.emitted_text.clear();
        self.last_confirmed_end = 0.0;
    }

    /// Get currently confirmed text
    pub fn confirmed_text(&self) -> String {
        self.confirmed
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Count words in common prefix between new output and previous output
    fn common_prefix_len(&self, new_words: &[TimestampedWord]) -> usize {
        let mut count = 0;
        let max = new_words.len().min(self.prev_words.len());
        for i in 0..max {
            if normalize_word(&new_words[i].text) == normalize_word(&self.prev_words[i].text) {
                count = i + 1;
            } else {
                break;
            }
        }
        count
    }
}

/// Result from processing a new Whisper output
pub struct AgreementResult {
    /// Newly confirmed text to emit (empty if nothing new confirmed)
    pub confirmed_new: String,
    /// Full text from this pass (confirmed + unconfirmed, for subtitle display)
    pub full_text: String,
    /// If Some, audio buffer can be trimmed before this timestamp (seconds)
    pub trim_to: Option<f64>,
}

/// Find the byte index just past the last complete sentence in text.
/// A sentence ends with '.', '!', or '?' followed by a space or end-of-string,
/// but only if there is more text after it (to avoid cutting mid-sentence).
fn find_sentence_end(text: &str) -> usize {
    let mut last_end = 0;
    let chars: Vec<char> = text.chars().collect();
    let mut byte_pos = 0;

    for (i, &ch) in chars.iter().enumerate() {
        let char_len = ch.len_utf8();
        byte_pos += char_len;

        if (ch == '.' || ch == '!' || ch == '?') && i + 1 < chars.len() {
            // Check next char is space or another sentence-ender
            let next = chars[i + 1];
            if next == ' ' || next == '.' || next == '!' || next == '?' {
                // Find end of whitespace after punctuation
                let mut end = byte_pos;
                let mut j = i + 1;
                while j < chars.len() && chars[j] == ' ' {
                    end += chars[j].len_utf8();
                    j += 1;
                }
                if j < chars.len() {
                    // There's more text after — this is a confirmed sentence boundary
                    last_end = end;
                }
            }
        }
    }

    last_end
}

/// Normalize a word for comparison (lowercase, strip trailing punctuation)
fn normalize_word(word: &str) -> String {
    word.to_lowercase()
        .trim_end_matches(|c: char| c.is_ascii_punctuation())
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_timestamped_text() {
        let text = "<|0.00|> Hello world<|2.50|><|2.50|> how are you<|5.00|>";
        let words = parse_timestamped_text(text);
        assert_eq!(words.len(), 5);
        assert_eq!(words[0].text, "Hello");
        assert!((words[0].start - 0.0).abs() < 0.01);
        assert_eq!(words[1].text, "world");
        assert_eq!(words[2].text, "how");
        assert!((words[2].start - 2.5).abs() < 0.01);
    }

    #[test]
    fn test_local_agreement_basic() {
        let mut la = LocalAgreement::new();

        // First pass — nothing confirmed yet (need 2 agreeing passes)
        let r1 = la.process("<|0.00|> Hello world.<|2.00|><|2.00|> How are you?<|4.00|>");
        assert!(r1.confirmed_new.is_empty());

        // Second pass — same prefix, should confirm
        let r2 = la.process("<|0.00|> Hello world.<|2.00|><|2.00|> How are you? I am fine.<|6.00|>");
        // "Hello world." is a complete sentence that's stable
        assert!(!r2.confirmed_new.is_empty());
        assert!(r2.confirmed_new.contains("Hello world."));
    }

    #[test]
    fn test_find_sentence_end() {
        assert_eq!(find_sentence_end("Hello world. How are you?"), 13);
        assert_eq!(find_sentence_end("Hello world."), 0); // No text after — not confirmed
        assert_eq!(find_sentence_end("Hello world"), 0); // No sentence boundary
        assert_eq!(find_sentence_end("A. B. C."), 6); // "A. B. " — last complete sentence before "C."
    }

    #[test]
    fn test_normalize_word() {
        assert_eq!(normalize_word("Hello,"), "hello");
        assert_eq!(normalize_word("world."), "world");
        assert_eq!(normalize_word("you?"), "you");
        assert_eq!(normalize_word("HELLO"), "hello");
    }
}
