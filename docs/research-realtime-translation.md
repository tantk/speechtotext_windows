# Real-Time Speech Translation: Research & Approaches

## Problem Statement

Our app uses Whisper with an always-listen mode (VAD-based segmentation) for real-time
translation. The current approach waits for 2 seconds of silence to finalize an utterance,
which rarely happens in continuous speech (e.g., translating a video). This causes either:
- Partials that keep re-translating the same growing audio buffer (repeated text)
- Long delays before any output appears

This document surveys open-source projects that solve this problem.

---

## 1. whisper_streaming (ufal) -- The Gold Standard

**Repo:** https://github.com/ufal/whisper_streaming
**Paper:** Machacek et al., "Turning Whisper into Real-Time Transcription System" (2023)

### Architecture

The system processes audio in a growing buffer but uses two key mechanisms to keep it bounded:

1. **LocalAgreement-n** for text confirmation
2. **Buffer trimming** at sentence/segment boundaries

### LocalAgreement-n Policy

Instead of waiting for silence, this compares consecutive Whisper outputs to determine what
text is stable:

- Each time new audio arrives, run Whisper on the full buffer
- Compare the new word-level output with the previous output
- Find the **longest common prefix** of words that match across n consecutive passes
- Only those matching words are "confirmed" and emitted to the user
- Default: n=2 (two consecutive agreeing passes)

**Key implementation detail:** The comparison works at the **word level with timestamps**,
not just string matching. The `HypothesisBuffer` class maintains:
- `commited_in_buffer`: words already confirmed
- `new`: words from the latest transcription pass
- It searches for matching n-grams to align old and new outputs

```python
# Simplified LocalAgreement logic
def flush(self):
    # Find longest common prefix between consecutive outputs
    committed = []
    for word in new_output:
        if word matches previous_output at same position:
            committed.append(word)
        else:
            break  # Divergence point -- stop confirming
    return committed
```

### Buffer Trimming

This is the critical innovation. After confirming text, the system **trims the audio buffer**:

1. Find the timestamp of the last confirmed complete sentence
2. Remove all audio samples before that timestamp
3. Adjust `buffer_time_offset` to track the global position
4. Next Whisper pass only processes audio from the trim point onward

Two trimming strategies:

- **"segment"** (default): Trim at Whisper's own segment boundaries (the `segments_end_ts`
  returned by Whisper). Simpler, no external dependencies.
- **"sentence"**: Trim at punctuation-detected sentence endings. Uses language-specific
  sentence segmenters (e.g., `wtpsplit`). More precise but requires additional libraries.

```python
# Segment-based trimming (simplified)
def chunk_completed_segment(self, res):
    ends = self.asr.segments_end_ts(res)
    if len(ends) > 1:
        # Trim at second-to-last segment end (keep last segment as context)
        trim_at = ends[-2] + self.buffer_time_offset
        self.chunk_at(trim_at)

def chunk_at(self, timestamp):
    # Remove audio before timestamp
    samples_to_remove = int(timestamp * sample_rate) - self.buffer_offset
    self.audio_buffer = self.audio_buffer[samples_to_remove:]
    self.buffer_time_offset = timestamp
```

### Translation Mode

Translation uses the exact same pipeline -- just sets `task="translate"` on Whisper.
LocalAgreement and buffer trimming work identically because they operate on the **output
text**, not the source language. When translating, sentence segmentation uses English
tokenizer regardless of source language.

### Performance

- ~3.3 seconds latency on unsegmented long-form speech
- Works with any Whisper backend (openai, faster-whisper, MLX, etc.)

---

## 2. SimulStreaming (ufal) -- Next Generation

**Repo:** https://github.com/ufal/SimulStreaming
**Paper:** Machacek & Polak, IWSLT 2025 Simultaneous Speech Translation Shared Task (winner)

SimulStreaming is the successor to whisper_streaming, merging it with Simul-Whisper. It adds
a two-stage pipeline: Whisper for ASR + EuroLLM for translation (instead of relying on
Whisper's built-in translation). ~5x faster than whisper_streaming.

### AlignAtt Policy (Best-Performing, 2025)

Uses **encoder-decoder attention weights** to determine how much of the source audio has been
"consumed" at each decoding step:

- During Whisper's autoregressive decoding, inspect attention weights
- If attention reaches a "dangerous zone" near the end of the audio buffer, **pause decoding**
- Wait for more audio to arrive before continuing
- `--frame_threshold` controls how many frames from the buffer end trigger the pause
  (1 frame = 0.02s for large-v3)

This is more sophisticated than LocalAgreement because it uses the model's own internal
signals rather than comparing output text across passes. However, it requires access to
attention weights, making it harder to implement with opaque APIs.

### Voice Activity Controller (VAC)

Optional component that **detects and skips unvoiced segments**:

- Uses Silero VAD (torch-based) for voice detection
- Skips silence segments entirely, reducing unnecessary Whisper computation
- Configurable chunk size (`--vac-chunk-size`)
- Reduces latency by not processing silence

### Two-Stage Translation Pipeline

Instead of Whisper's built-in `task="translate"`:

1. **Stage 1 (Whisper):** Transcribe source language with timestamps (JSONL output)
2. **Stage 2 (EuroLLM 9B):** Translate transcribed text to target language

Advantages:
- Better translation quality (dedicated LLM vs Whisper's translation head)
- Supports 200 language pairs (not just to-English)
- Can inject domain terminology via prompts (RAG support)

Architecture:
```
Microphone -> TCP Server (Whisper ASR) -> TCP Server (EuroLLM) -> Output
```

### Key Parameters

| Parameter | Description | Default |
|-----------|-------------|---------|
| `--min-chunk-size` | Min seconds before processing | varies |
| `--frame_threshold` | Frames from end to pause decoding (AlignAtt) | varies |
| `--buffer_trimming` | "segments" or "sentences" | segments |
| `--beams` | Beam search width (1=greedy) | 1 |
| `--vac` | Enable Voice Activity Controller | off |
| `--comp_unaware` | Fixed chunk size, ignore compute time | off |

---

## 3. WhisperLive (Collabora)

**Repo:** https://github.com/collabora/WhisperLive

Client-server architecture for near-live transcription:
- Client captures audio and sends chunks via WebSocket
- Server runs faster-whisper, TensorRT, or OpenVINO backend
- Optional Silero VAD for voice activity detection
- Supports microphone, audio files, RTSP, and HLS streams

Less documented internals compared to ufal projects, but widely used in production.

---

## 4. WhisperLiveKit (QuentinFuxa)

**Repo:** https://github.com/QuentinFuxa/WhisperLiveKit

Built on top of whisper_streaming, adds:
- Web UI with server component
- Speaker diarization
- Simultaneous translation from/to 200 languages
- Uses the same LocalAgreement + buffer trimming approach

---

## 5. Other Notable Projects

### Speech-Translate (Dadangdut33)
**Repo:** https://github.com/Dadangdut33/Speech-Translate
- Desktop app (Tkinter) combining Whisper + free translation APIs
- Practical but simpler approach -- fixed-interval transcription

### whisper_real_time_translation (mldljyh)
**Repo:** https://github.com/mldljyh/whisper_real_time_translation
- Real-time subtitles displayed as pop-ups
- Uses Faster-Whisper + TranslatePy for translation

### Realtime-Speech-to-Speech-Translation (kensonhui)
**Repo:** https://github.com/kensonhui/Realtime-Speech-to-Speech-Translation
- Audio-to-audio translation (Whisper ASR + Microsoft SpeechT5 TTS)
- Virtual microphone integration for video conferencing

---

## Key Takeaways for Our Implementation

### What we currently do (and its limitations)
1. VAD detects speech, accumulates audio in a growing buffer
2. Send partial snapshots (entire buffer) to Whisper every 1.2s
3. Wait for 2s silence to finalize -- **too long for continuous speech**
4. Translation partials cause repeated text due to non-deterministic rephrasing

### What we should adopt from whisper_streaming

#### Priority 1: Buffer Trimming at Sentence Boundaries
Instead of keeping the entire audio buffer and re-transcribing everything:
- After confirming a sentence, **trim the audio buffer** at that sentence's timestamp
- Next Whisper pass only processes new audio (+ small overlap for context)
- This bounds memory usage and processing time
- Requires: enabling Whisper's timestamp output (`timestamps=true`)

#### Priority 2: LocalAgreement for Text Confirmation
Replace our candidate_count approach with proper word-level comparison:
- Compare consecutive Whisper outputs at the word level
- Confirm the longest matching prefix
- More robust than our current string-prefix matching, especially for translation

#### Priority 3: VAD-Based Buffer Skipping (Already Have)
We already use energy-based VAD. Could upgrade to Silero VAD for better accuracy,
but our current approach works reasonably well.

### Implementation Approach

The buffer trimming requires changes to the always-listen controller:

```
Current flow:
  [audio grows] -> [snapshot entire buffer] -> [Whisper] -> [compare text] -> [wait for silence]

Proposed flow:
  [audio grows] -> [Whisper with timestamps] -> [LocalAgreement on words]
       -> [confirmed sentence?] -> YES: trim buffer at timestamp, emit text
                                -> NO:  wait for next chunk
```

Key changes needed:
1. Enable timestamps in Whisper output (already supported in backend, just hardcoded off)
2. Parse timestamp tokens from Whisper output to get word/segment boundaries
3. Implement buffer trimming in `AudioBufferManager` -- remove samples before a given timestamp
4. Replace string-prefix dedup with word-level LocalAgreement
5. Emit confirmed sentences immediately without waiting for silence

### Trade-offs

| Approach | Latency | Quality | Complexity |
|----------|---------|---------|------------|
| Our current (silence-based) | High (2s+ silence needed) | Good per-utterance | Low |
| LocalAgreement + trim | Medium (~3s) | Good streaming | Medium |
| AlignAtt (SimulStreaming) | Low (~1-2s) | Best | High (needs attention weights) |

**Recommendation:** Start with LocalAgreement + buffer trimming (whisper_streaming approach).
It's well-proven, doesn't require attention weight access, and directly solves our repeated
translation problem. AlignAtt can be explored later if lower latency is needed.

---

## Future Improvements (Not Yet Portable)

### AlignAtt (from SimulStreaming)
Uses Whisper's encoder-decoder attention weights to determine how much source audio has been
consumed at each decoding step. When attention reaches a "dangerous zone" near the buffer end,
decoding pauses until more audio arrives. This is the best-performing policy (IWSLT 2025
winner) with ~1-2s latency.

**Portability challenge:** Requires access to Whisper's internal attention weights during
decoding. CTranslate2's C++ API may not expose these. Would need to either:
- Modify `ct2rs` Rust bindings to expose attention weights from CTranslate2
- Or switch to a backend that provides this (e.g., direct ONNX inference with `ort` crate)

**When to revisit:** If LocalAgreement latency (~3s) proves insufficient for the use case.

### Silero VAD (from SimulStreaming / WhisperLive)
Neural network-based Voice Activity Detection, significantly more accurate than energy-based
VAD (our current approach). Silero VAD is a small PyTorch model that detects speech vs
non-speech with high accuracy even in noisy environments.

**Portability challenge:** Silero VAD is a PyTorch model. To run in Rust, would need:
- Export to ONNX format
- Use `ort` crate (ONNX Runtime Rust bindings) for inference
- ~5MB model, minimal latency overhead

**When to revisit:** If our energy-based VAD produces too many false positives/negatives,
especially with system audio loopback (music, sound effects triggering false speech detection).
