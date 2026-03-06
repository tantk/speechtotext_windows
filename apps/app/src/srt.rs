//! SRT subtitle file generation and timestamp parsing
//!
//! Parses ct2rs timestamp tokens like `<|0.00|> text <|2.50|>` and
//! generates SRT formatted subtitle files.

use regex::Regex;

/// Parse timestamped text output from ct2rs Whisper.
///
/// Input format: `<|0.00|> hello world <|2.50|> how are you <|5.00|>`
/// Returns: Vec of (start_seconds, end_seconds, text) tuples with chunk_offset added.
pub fn parse_timestamped_text(text: &str, chunk_offset: f64) -> Vec<(f64, f64, String)> {
    let re = Regex::new(r"<\|(\d+\.?\d*)\|>").unwrap();
    let mut segments = Vec::new();

    // Collect all timestamp positions and values
    let timestamps: Vec<(usize, f64)> = re
        .captures_iter(text)
        .filter_map(|cap| {
            let full_match = cap.get(0)?;
            let value: f64 = cap[1].parse().ok()?;
            Some((full_match.end(), value))
        })
        .collect();

    // Extract text between consecutive timestamps
    for window in timestamps.windows(2) {
        let (end_pos, start_time) = window[0];
        let next_match = re.find_at(text, end_pos);
        let text_end = next_match.map(|m| m.start()).unwrap_or(text.len());

        let segment_text = text[end_pos..text_end].trim().to_string();
        if !segment_text.is_empty() {
            let end_time = window[1].1;
            segments.push((
                start_time + chunk_offset,
                end_time + chunk_offset,
                segment_text,
            ));
        }
    }

    segments
}

/// Format seconds as SRT timestamp: HH:MM:SS,mmm
fn format_srt_time(seconds: f64) -> String {
    let total_ms = (seconds * 1000.0).round() as u64;
    let ms = total_ms % 1000;
    let total_s = total_ms / 1000;
    let s = total_s % 60;
    let total_m = total_s / 60;
    let m = total_m % 60;
    let h = total_m / 60;
    format!("{:02}:{:02}:{:02},{:03}", h, m, s, ms)
}

/// Generate an SRT formatted string from segments.
///
/// Each segment is (start_seconds, end_seconds, text).
pub fn generate_srt(segments: &[(f64, f64, String)]) -> String {
    let mut output = String::new();
    for (i, (start, end, text)) in segments.iter().enumerate() {
        if i > 0 {
            output.push('\n');
        }
        output.push_str(&format!(
            "{}\n{} --> {}\n{}\n",
            i + 1,
            format_srt_time(*start),
            format_srt_time(*end),
            text
        ));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_timestamped_text_basic() {
        let text = "<|0.00|> hello world <|2.50|> how are you <|5.00|>";
        let segments = parse_timestamped_text(text, 0.0);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0], (0.0, 2.5, "hello world".to_string()));
        assert_eq!(segments[1], (2.5, 5.0, "how are you".to_string()));
    }

    #[test]
    fn test_parse_timestamped_text_with_offset() {
        let text = "<|0.00|> hello <|3.00|>";
        let segments = parse_timestamped_text(text, 30.0);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0], (30.0, 33.0, "hello".to_string()));
    }

    #[test]
    fn test_parse_timestamped_text_empty() {
        let text = "<|0.00|><|2.50|>";
        let segments = parse_timestamped_text(text, 0.0);
        assert_eq!(segments.len(), 0);
    }

    #[test]
    fn test_parse_no_timestamps() {
        let text = "just plain text";
        let segments = parse_timestamped_text(text, 0.0);
        assert_eq!(segments.len(), 0);
    }

    #[test]
    fn test_format_srt_time() {
        assert_eq!(format_srt_time(0.0), "00:00:00,000");
        assert_eq!(format_srt_time(2.5), "00:00:02,500");
        assert_eq!(format_srt_time(61.123), "00:01:01,123");
        assert_eq!(format_srt_time(3661.5), "01:01:01,500");
    }

    #[test]
    fn test_generate_srt() {
        let segments = vec![
            (0.0, 2.5, "hello world".to_string()),
            (2.5, 5.0, "how are you".to_string()),
        ];
        let srt = generate_srt(&segments);
        let expected = "1\n00:00:00,000 --> 00:00:02,500\nhello world\n\n2\n00:00:02,500 --> 00:00:05,000\nhow are you\n";
        assert_eq!(srt, expected);
    }

    #[test]
    fn test_generate_srt_empty() {
        let segments: Vec<(f64, f64, String)> = vec![];
        let srt = generate_srt(&segments);
        assert_eq!(srt, "");
    }

    #[test]
    fn test_generate_srt_long_timestamps() {
        let segments = vec![(3661.5, 3665.0, "one hour in".to_string())];
        let srt = generate_srt(&segments);
        assert!(srt.contains("01:01:01,500 --> 01:01:05,000"));
    }
}
