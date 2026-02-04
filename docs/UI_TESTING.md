# UI Testing Guide

This document describes how to test the app UI components including the Setup Wizard and Overlay.

## Overview

The app has two main UI components:

1. **Setup Wizard** (`setup.rs`) - Initial configuration window (500x500px)
2. **Overlay** (`overlay.rs`) - Status indicator during runtime (120x50px)

Both use `tao` for windowing and `softbuffer` for software rendering.

---

## Automated Unit Tests

### Running UI Tests

```bash
cargo test -p app setup::tests      # Setup wizard tests
cargo test -p app overlay::tests    # Overlay tests
cargo test -p app                   # All tests
```

### Current Test Coverage

| Component | Tests | Description |
|-----------|-------|-------------|
| **Setup Wizard** | 13 tests | Helper functions, button geometry, page navigation |
| **Overlay** | 5 tests | Status colors, dimensions, state transitions |

---

## Setup Wizard Tests

### Test Categories

#### 1. Helper Function Tests

```bash
cargo test -p app setup::tests::test_format_hotkey_display
cargo test -p app setup::tests::test_keycode_to_string
cargo test -p app setup::tests::test_is_modifier_key
```

These verify:
- Hotkey display formatting (`Control+KeyA` → `Ctrl+A`)
- Key code to string conversion
- Modifier key detection

#### 2. Drawing Function Tests

```bash
cargo test -p app setup::tests::test_draw_rect
cargo test -p app setup::tests::test_draw_text
cargo test -p app setup::tests::test_get_char_bitmap_coverage
```

These verify:
- Rectangle drawing doesn't go out of bounds
- Text rendering writes pixels
- All printable ASCII characters have bitmaps

#### 3. Button Geometry Tests

```bash
cargo test -p app setup::tests::test_is_inside
cargo test -p app setup::tests::test_button_rect_consistency
```

These verify:
- Hit detection for mouse clicks
- Button rectangles are valid (non-zero size)

#### 4. State Management Tests

```bash
cargo test -p app setup::tests::test_hotkey_target_variants
cargo test -p app setup::tests::test_hotkey_capture_states
cargo test -p app setup::tests::test_setup_page_navigation
```

These verify:
- All hotkey targets exist and are distinct
- Hotkey capture states work
- Setup pages are distinct

#### 5. Visual Constants Tests

```bash
cargo test -p app setup::tests::test_color_format
cargo test -p app setup::tests::test_window_dimensions
cargo test -p app setup::tests::test_visible_models_constant
```

These verify:
- Colors have proper alpha channel
- Window dimensions are reasonable
- Model list constants are valid

---

## Overlay Tests

### Test Categories

```bash
cargo test -p app overlay::tests
```

#### Status Color Tests

- **test_app_status_variants** - All 4 status types exist with unique colors
- **test_status_color_contrast** - Colors are visually distinguishable
- **test_status_title_mapping** - Window titles match status

#### Dimension Tests

- **test_overlay_dimensions** - Overlay size is appropriate (120x50px)

#### State Tests

- **test_overlay_state_transitions** - Can transition between all states

---

## Manual UI Testing

Since the UI uses direct framebuffer rendering, automated visual testing is limited. Manual testing is required for:

### Setup Wizard Manual Test Checklist

#### Visual Layout Tests

1. **Home Page Layout**
   ```bash
   cargo run --release
   # Delete config.json to trigger setup wizard
   ```
   - [ ] Window size is 500x500px
   - [ ] Title bar shows "Speech-to-Text Setup"
   - [ ] All fields are aligned properly
   - [ ] Status text is visible at bottom

2. **Model Selection Page**
   - [ ] Click "Select" button opens model page
   - [ ] Model list shows up to 6 items
   - [ ] Scroll indicators (^ v) appear when needed
   - [ ] Selected model is highlighted
   - [ ] Download button is visible
   - [ ] Progress bar appears during download

3. **Hotkey Config Page**
   - [ ] Click "Configure" opens hotkey page
   - [ ] Current hotkey is displayed
   - [ ] "Set Hotkey" button is clickable
   - [ ] Pressing keys updates display
   - [ ] Confirm/Clear buttons work

4. **CUDA Config Page**
   - [ ] Click "Setup" (when GPU enabled) opens CUDA page
   - [ ] Auto-Detect button finds CUDA paths
   - [ ] Browse buttons open file dialogs
   - [ ] Status indicators show [OK] or [!]

#### Interaction Tests

1. **Mouse Interactions**
   - [ ] Hover over buttons changes color
   - [ ] Click buttons triggers action
   - [ ] Scroll wheel scrolls model list

2. **Keyboard Interactions**
   - [ ] Tab/shift+tab navigation (if implemented)
   - [ ] Enter key activates focused button
   - [ ] Escape key goes back

3. **Hotkey Capture**
   - [ ] Click "Set Hotkey" enters capture mode
   - [ ] Display shows "Press any key..."
   - [ ] Pressing Ctrl+A shows "Ctrl+A"
   - [ ] Clicking Confirm saves hotkey
   - [ ] Clicking Clear removes hotkey

#### Functional Tests

1. **Model Download**
   - [ ] Select a model
   - [ ] Click Download starts download
   - [ ] Progress bar updates
   - [ ] Status shows download info
   - [ ] Completion updates status

2. **Configuration Flow**
   - [ ] Select model
   - [ ] Configure hotkeys
   - [ ] Toggle GPU on/off
   - [ ] Configure CUDA (if GPU on)
   - [ ] Click Start creates config.json
   - [ ] Application restarts

3. **Validation**
   - [ ] Cannot start without model selected
   - [ ] Cannot start without model downloaded
   - [ ] Status message shows appropriate warnings

---

### Overlay Manual Test Checklist

#### Visual Tests

```bash
# Start the application
cargo run --release
```

1. **Initial State**
   - [ ] Overlay appears (120x50px colored rectangle)
   - [ ] Position is bottom-left of screen
   - [ ] Color is dark gray (Idle state)
   - [ ] Title bar shows "Idle"

2. **Status Colors**
   - [ ] **Idle**: Dark gray (0xFF505050)
   - [ ] **Recording**: Red (0xFFDD3333) - Press and hold PTT
   - [ ] **Processing**: Yellow (0xFFDDAA00) - Release PTT
   - [ ] **Always Listening**: Green (0xFF33AA33) - Toggle always-listen

3. **Window Behavior**
   - [ ] No window decorations (borderless)
   - [ ] Always on top of other windows
   - [ ] Not resizable
   - [ ] Can be dragged by clicking and holding
   - [ ] Position persists (saved to config)

#### Functional Tests

1. **Dragging**
   - [ ] Click and drag to move overlay
   - [ ] Release to drop
   - [ ] Position remembered on restart

2. **Toggle Visibility**
   - [ ] Can be hidden/shown via tray menu
   - [ ] State persists correctly

---

## Visual Regression Testing

For detecting UI changes, consider:

### Screenshot Comparison

```rust
// Add to integration tests (requires display)
#[test]
#[ignore = "Requires display - manual only"]
fn test_setup_wizard_screenshot() {
    // Run setup wizard
    // Take screenshot
    // Compare with reference
}
```

### Manual Screenshot Checklist

Take screenshots of:
1. Home page (all states: initial, model selected, ready)
2. Model selection page (empty, with models, downloading)
3. Hotkey config page (idle, capturing)
4. CUDA config page (not detected, detected)
5. Overlay (all 4 status colors)

Store in `screenshots/` directory for reference.

---

## Accessibility Testing

### Current Limitations

- No screen reader support (custom softbuffer rendering)
- No high contrast mode
- No keyboard-only navigation
- Fixed font size

### Workarounds

1. **High DPI Displays**
   - Test on 4K monitors
   - Verify window scales appropriately

2. **Color Blindness**
   - Overlay uses both color AND text (window title)
   - Status in tray icon also indicates state

---

## Performance Testing

### Rendering Performance

```bash
# Profile the application
cargo run --release
# Watch for frame drops during:
# - Page transitions
# - Download progress updates
# - Hotkey capture animation
```

### Target Metrics

- Setup wizard: 60 FPS rendering
- Overlay: 30 FPS is sufficient (static mostly)
- Model list scroll: Smooth at 60 FPS

---

## Platform Testing

Test on different Windows versions:

| OS | Priority | Notes |
|----|----------|-------|
| Windows 11 | High | Primary target |
| Windows 10 (22H2) | High | Widely used |
| Windows 10 (older) | Medium | May have issues |

### DPI Testing

- 100% scaling (96 DPI)
- 125% scaling (120 DPI)
- 150% scaling (144 DPI)
- 200% scaling (192 DPI)

---

## CI/CD Considerations

### Automated Tests (Run in CI)

```bash
# These don't need a display
cargo test -p app setup::tests
cargo test -p app overlay::tests
```

### Manual Tests (Pre-release only)

- Visual layout verification
- Color accuracy on real monitors
- Drag and drop behavior
- File dialog interactions

### Why No Headless UI Tests?

The UI uses `tao` + `softbuffer` which requires:
- A display server (Windows desktop)
- GPU/graphics driver for softbuffer
- User input for meaningful testing

Alternatives considered:
- Mock rendering (doesn't test actual drawing)
- Screenshot comparison (fragile, OS-dependent)
- Record/replay (complex to implement)

---

## Debugging UI Issues

### Enable Debug Logging

```rust
// In main.rs or setup.rs
println!("Button clicked: {:?}", button);
println!("Mouse position: {:?}", state.mouse_pos);
println!("Current page: {:?}", state.current_page);
```

### Common Issues

1. **Black screen in setup wizard**
   - Check softbuffer context creation
   - Verify window size is non-zero

2. **Buttons not responding**
   - Check `is_inside()` calculations
   - Verify `get_button_rects()` returns correct rects

3. **Text not rendering**
   - Check bitmap font contains character
   - Verify buffer bounds in `draw_char()`

4. **Overlay not visible**
   - Check `with_always_on_top(true)`
   - Verify position is on-screen

---

## Summary

| Test Type | Automated | Manual | Priority |
|-----------|-----------|--------|----------|
| Helper functions | ✅ Yes | No | High |
| Button geometry | ✅ Yes | No | High |
| State management | ✅ Yes | No | High |
| Visual layout | No | ✅ Yes | High |
| Color accuracy | Partial | ✅ Yes | Medium |
| Interactions | No | ✅ Yes | High |
| Performance | No | ✅ Yes | Medium |

**Total Automated Tests:** 18 UI-specific tests (76 total across project)
