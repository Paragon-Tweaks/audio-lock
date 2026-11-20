use tray_icon::Icon;

const ICON_SIZE: u32 = 32;
const SUPERSAMPLE: u32 = 4;
const CENTER: f32 = 16.0;

// Rounded-square tile matching assets/app.ico
const TILE_MARGIN: f32 = 1.0;
const TILE_HALF_EXTENT: f32 = CENTER - TILE_MARGIN;
const TILE_RADIUS: f32 = 6.6;
const TILE_BORDER_WIDTH: f32 = 1.4;

// Centered microphone glyph inside the dark tile
const STROKE_HALF: f32 = 0.95;
const CAPSULE_TOP_Y: f32 = 10.2;
const CAPSULE_BOT_Y: f32 = 14.8;
const CAPSULE_RADIUS: f32 = 3.25;
const CRADLE_TOP_Y: f32 = 13.7;
const CRADLE_RADIUS: f32 = 6.35;
const BASE_Y: f32 = 24.8;
const BASE_HALF_W: f32 = 4.0;

const TILE_TOP: [f32; 3] = [28.0, 28.0, 28.0]; // #1C1C1C
const TILE_BOT: [f32; 3] = [20.0, 20.0, 20.0]; // #141414
const BORDER_DARK_TASKBAR: [f32; 3] = [74.0, 74.0, 74.0]; // #4A4A4A
const BORDER_LIGHT_TASKBAR: [f32; 3] = [44.0, 44.0, 44.0]; // #2C2C2C
const MIC_WHITE: [f32; 3] = [245.0, 245.0, 245.0]; // #F5F5F5
const RED: [f32; 3] = [239.0, 68.0, 68.0]; // #EF4444

pub fn unmuted_mic_icon(light_taskbar: bool) -> Icon {
    render_icon(false, light_taskbar)
}

pub fn muted_mic_icon(light_taskbar: bool) -> Icon {
    render_icon(true, light_taskbar)
}

fn render_icon(muted: bool, light_taskbar: bool) -> Icon {
    let rgba = render_icon_rgba(muted, light_taskbar);
    Icon::from_rgba(rgba, ICON_SIZE, ICON_SIZE).expect("32x32 RGBA icon data is always valid")
}

fn render_icon_rgba(muted: bool, light_taskbar: bool) -> Vec<u8> {
    let mut rgba = vec![0u8; (ICON_SIZE * ICON_SIZE * 4) as usize];
    let samples_total = (SUPERSAMPLE * SUPERSAMPLE) as f32;
    let border_color = if light_taskbar {
        BORDER_LIGHT_TASKBAR
    } else {
        BORDER_DARK_TASKBAR
    };

    for py in 0..ICON_SIZE {
        for px in 0..ICON_SIZE {
            let mut r_sum = 0.0f32;
            let mut g_sum = 0.0f32;
            let mut b_sum = 0.0f32;
            let mut a_sum = 0.0f32;

            for sy in 0..SUPERSAMPLE {
                for sx in 0..SUPERSAMPLE {
                    let x = px as f32 + (sx as f32 + 0.5) / SUPERSAMPLE as f32;
                    let y = py as f32 + (sy as f32 + 0.5) / SUPERSAMPLE as f32;
                    if let Some([r, g, b]) = sample_pixel(x, y, muted, border_color) {
                        r_sum += r;
                        g_sum += g;
                        b_sum += b;
                        a_sum += 1.0;
                    }
                }
            }

            if a_sum > 0.0 {
                let idx = ((py * ICON_SIZE + px) * 4) as usize;
                rgba[idx] = (r_sum / a_sum).round() as u8;
                rgba[idx + 1] = (g_sum / a_sum).round() as u8;
                rgba[idx + 2] = (b_sum / a_sum).round() as u8;
                rgba[idx + 3] = ((a_sum / samples_total) * 255.0).round() as u8;
            }
        }
    }

    rgba
}

fn sample_pixel(x: f32, y: f32, muted: bool, border_color: [f32; 3]) -> Option<[f32; 3]> {
    let tile_d = rounded_box_sdf(
        x - CENTER,
        y - CENTER,
        TILE_HALF_EXTENT,
        TILE_HALF_EXTENT,
        TILE_RADIUS,
    );
    if tile_d > 0.0 {
        return None;
    }

    let cap_signed = capsule_sdf(x, y);
    if mic_outline_sdf(x, y, cap_signed) <= STROKE_HALF {
        return Some(MIC_WHITE);
    }
    if muted && cap_signed < -STROKE_HALF {
        return Some(RED);
    }

    if tile_d > -TILE_BORDER_WIDTH {
        return Some(border_color);
    }

    let t = ((y - TILE_MARGIN) / (TILE_HALF_EXTENT * 2.0)).clamp(0.0, 1.0);
    Some([
        TILE_TOP[0] + (TILE_BOT[0] - TILE_TOP[0]) * t,
        TILE_TOP[1] + (TILE_BOT[1] - TILE_TOP[1]) * t,
        TILE_TOP[2] + (TILE_BOT[2] - TILE_TOP[2]) * t,
    ])
}

fn rounded_box_sdf(px: f32, py: f32, bx: f32, by: f32, radius: f32) -> f32 {
    let qx = px.abs() - (bx - radius);
    let qy = py.abs() - (by - radius);
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius
}

fn capsule_sdf(x: f32, y: f32) -> f32 {
    dist_to_segment(x, y, CENTER, CAPSULE_TOP_Y, CENTER, CAPSULE_BOT_Y) - CAPSULE_RADIUS
}

fn mic_outline_sdf(x: f32, y: f32, cap_signed: f32) -> f32 {
    let d_capsule = cap_signed.abs();

    let d_arc = if y >= CAPSULE_BOT_Y {
        ((x - CENTER).hypot(y - CAPSULE_BOT_Y) - CRADLE_RADIUS).abs()
    } else {
        f32::INFINITY
    };
    let d_left_leg = dist_to_segment(
        x,
        y,
        CENTER - CRADLE_RADIUS,
        CRADLE_TOP_Y,
        CENTER - CRADLE_RADIUS,
        CAPSULE_BOT_Y,
    );
    let d_right_leg = dist_to_segment(
        x,
        y,
        CENTER + CRADLE_RADIUS,
        CRADLE_TOP_Y,
        CENTER + CRADLE_RADIUS,
        CAPSULE_BOT_Y,
    );
    let d_cradle = d_arc.min(d_left_leg).min(d_right_leg);

    let d_stem = dist_to_segment(
        x,
        y,
        CENTER,
        CAPSULE_BOT_Y + CRADLE_RADIUS,
        CENTER,
        BASE_Y,
    );
    let d_base = dist_to_segment(
        x,
        y,
        CENTER - BASE_HALF_W,
        BASE_Y,
        CENTER + BASE_HALF_W,
        BASE_Y,
    );

    d_capsule.min(d_cradle).min(d_stem).min(d_base)
}

fn dist_to_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let abx = bx - ax;
    let aby = by - ay;
    let len_sq = abx * abx + aby * aby;
    let t = if len_sq > 0.0 {
        ((px - ax) * abx + (py - ay) * aby) / len_sq
    } else {
        0.0
    }
    .clamp(0.0, 1.0);
    (px - (ax + t * abx)).hypot(py - (ay + t * aby))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_dark_app_tile_with_red_muted_center() {
        let live = render_icon_rgba(false, false);
        let muted = render_icon_rgba(true, false);

        // Corner outside rounded tile stays transparent
        assert_eq!(live[3], 0);
        // Center of tile inside capsule is dark when live, red when muted
        let center_idx = ((12 * ICON_SIZE + 16) * 4) as usize;
        assert!(live[center_idx] < 40 && live[center_idx + 3] == 255);
        assert!(muted[center_idx] > 200 && muted[center_idx + 1] < 90 && muted[center_idx + 3] == 255);
    }
}
