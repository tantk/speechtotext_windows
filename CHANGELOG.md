# Changelog

## 0.1.5
- Replaced multi-process setup wizard with sequential process handoff — eliminates race conditions, mutex contention, and "Already Running" errors.
- Removed Voxtral (mistral.rs) backend — model loading issues were unresolved. Only Whisper and Faster Whisper backends remain.
- Switched text output from keystroke simulation to clipboard paste (Ctrl+V) for reliability.
- Added `arboard` crate for clipboard access.

## 0.1.4
- Pseudo-streaming partials for Toggle Listen with stability gating and URL/symbol filtering.
- Exposed streaming interval setting in the setup wizard (200–3000 ms).
- Hotkey capture/registration fixes (including punctuation + Enter) and reliable hotkey persistence on restart.
- Single-instance setup wizard per exe name.
- Layout fixes for Toggle Listening config page to prevent overlapping controls.

