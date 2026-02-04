// Generate microphone icon PNG files
// Run with: cargo run --example generate_icon

use image::{ImageBuffer, Rgba};
use std::path::Path;

const SIZE: u32 = 64;

fn main() {
    // Create assets directory if it doesn't exist
    std::fs::create_dir_all("assets").unwrap();

    // Generate white microphone icon (will be tinted at runtime)
    let icon = generate_microphone_icon([255, 255, 255, 255]);
    icon.save("assets/microphone.png").unwrap();
    println!("Generated assets/microphone.png");

    // Also generate colored versions for reference
    let gray = generate_microphone_icon([128, 128, 128, 255]);
    gray.save("assets/mic_gray.png").unwrap();
    println!("Generated assets/mic_gray.png");

    let red = generate_microphone_icon([255, 80, 80, 255]);
    red.save("assets/mic_red.png").unwrap();
    println!("Generated assets/mic_red.png");

    let yellow = generate_microphone_icon([255, 200, 50, 255]);
    yellow.save("assets/mic_yellow.png").unwrap();
    println!("Generated assets/mic_yellow.png");

    let green = generate_microphone_icon([80, 200, 80, 255]);
    green.save("assets/mic_green.png").unwrap();
    println!("Generated assets/mic_green.png");

    println!("\nDone! Icons saved to assets/");
}

fn generate_microphone_icon(color: [u8; 4]) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    let mut img = ImageBuffer::from_pixel(SIZE, SIZE, Rgba([0, 0, 0, 0]));

    let cx = SIZE as f32 / 2.0;

    for y in 0..SIZE {
        for x in 0..SIZE {
            let px = x as f32;
            let py = y as f32;

            if is_microphone_pixel(px, py, cx) {
                img.put_pixel(x, y, Rgba(color));
            }
        }
    }

    img
}

fn is_microphone_pixel(px: f32, py: f32, cx: f32) -> bool {
    // Microphone head (capsule/rounded rectangle)
    let head_top = 4.0;
    let head_bottom = 36.0;
    let head_width = 22.0;
    let head_left = cx - head_width / 2.0;
    let head_right = cx + head_width / 2.0;
    let corner_radius = 11.0;

    let in_head = if py >= head_top && py <= head_bottom {
        if py < head_top + corner_radius {
            // Top rounded part (semicircle)
            let center_y = head_top + corner_radius;
            let dy = center_y - py;
            let max_dx = (corner_radius * corner_radius - dy * dy).max(0.0).sqrt();
            px >= cx - max_dx && px <= cx + max_dx
        } else if py > head_bottom - corner_radius {
            // Bottom rounded part (semicircle)
            let center_y = head_bottom - corner_radius;
            let dy = py - center_y;
            let max_dx = (corner_radius * corner_radius - dy * dy).max(0.0).sqrt();
            px >= cx - max_dx && px <= cx + max_dx
        } else {
            // Middle straight part
            px >= head_left && px <= head_right
        }
    } else {
        false
    };

    // Stand (vertical line)
    let stand_top = 38.0;
    let stand_bottom = 50.0;
    let stand_width = 5.0;
    let in_stand = py >= stand_top && py <= stand_bottom
        && px >= cx - stand_width / 2.0 && px <= cx + stand_width / 2.0;

    // Base (horizontal line with rounded ends)
    let base_y = 50.0;
    let base_height = 5.0;
    let base_width = 26.0;
    let in_base = py >= base_y && py <= base_y + base_height
        && px >= cx - base_width / 2.0 && px <= cx + base_width / 2.0;

    // U-shaped holder around the mic head
    let holder_outer_radius = 17.0;
    let holder_inner_radius = 13.0;
    let holder_center_y = 28.0;
    let dist = ((px - cx).powi(2) + (py - holder_center_y).powi(2)).sqrt();
    let in_holder = py > holder_center_y + 2.0
        && dist >= holder_inner_radius
        && dist <= holder_outer_radius;

    // Holder arms connecting to stand
    let arm_width = 4.0;
    let arm_top = holder_center_y + holder_outer_radius - 5.0;
    let arm_bottom = stand_top;
    let in_left_arm = py >= arm_top && py <= arm_bottom
        && px >= cx - holder_outer_radius && px <= cx - holder_outer_radius + arm_width;
    let in_right_arm = py >= arm_top && py <= arm_bottom
        && px >= cx + holder_outer_radius - arm_width && px <= cx + holder_outer_radius;

    in_head || in_stand || in_base || in_holder || in_left_arm || in_right_arm
}
