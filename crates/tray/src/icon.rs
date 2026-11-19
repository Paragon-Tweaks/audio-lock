use tray_icon::Icon;

const ICON_SIZE: u32 = 32;
const SUPERSAMPLE: u32 = 4;
const CENTER: f32 = 16.0;
const STROKE_HALF: f32 = 1.25;

const CAPSULE_TOP_Y: f32 = 7.5;
const CAPSULE_BOT_Y: f32 = 14.5;
const CAPSULE_RADIUS: f32 = 4.75;
const CRADLE_TOP_Y: f32 = 12.75;
const CRADLE_RADIUS: f32 = 9.5;
const BASE_Y: f32 = 29.25;
const BASE_HALF_W: f32 = 5.75;

const WHITE: [f32; 3] = [255.0, 255.0, 255.0];
const RED: [f32; 3] = [239.0, 68.0, 68.0];

pub fn unmuted_mic_icon() -> Icon {
    render_icon(false)
}

pub fn muted_mic_icon() -> Icon {
    render_icon(true)
}

fn render_icon(muted: bool) -> Icon {
    let mut rgba = vec![0u8; (ICON_SIZE * ICON_SIZE * 4) as usize];
    let samples_total = (SUPERSAMPLE * SUPERSAMPLE) as f32;

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
                    if let Some([r, g, b]) = sample_pixel(x, y, muted) {
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

    Icon::from_rgba(rgba, ICON_SIZE, ICON_SIZE).expect("32x32 RGBA icon data is always valid")
}

fn sample_pixel(x: f32, y: f32, muted: bool) -> Option<[f32; 3]> {
    let cap_signed = capsule_sdf(x, y);
    if mic_outline_sdf(x, y, cap_signed) <= STROKE_HALF {
        return Some(WHITE);
    }
    if muted && cap_signed < -STROKE_HALF {
        return Some(RED);
    }
    None
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
