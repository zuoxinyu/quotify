use crate::provider::UsageData;
use windows::Win32::Foundation::TRUE;
use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO};

pub struct TrayIcon {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl TrayIcon {
    pub fn to_hicon(&self) -> windows::core::Result<HICON> {
        let mut bgra = vec![0u8; self.rgba.len()];
        for i in (0..self.rgba.len()).step_by(4) {
            bgra[i] = self.rgba[i + 2]; // B
            bgra[i + 1] = self.rgba[i + 1]; // G
            bgra[i + 2] = self.rgba[i]; // R
            bgra[i + 3] = self.rgba[i + 3]; // A
        }

        unsafe {
            let hbm_color = CreateBitmap(
                self.width as i32,
                self.height as i32,
                1,
                32,
                Some(bgra.as_ptr() as *const _),
            );

            // For a 32bpp alpha icon, a 1bpp monochrome mask of all 0s is required.
            // A monochrome mask of 32x32 is 32 * 32 / 8 = 128 bytes of zeros.
            let mask_bits = [0u8; 128];
            let hbm_mask = CreateBitmap(
                self.width as i32,
                self.height as i32,
                1,
                1,
                Some(mask_bits.as_ptr() as *const _),
            );

            let info = ICONINFO {
                fIcon: TRUE,
                xHotspot: 0,
                yHotspot: 0,
                hbmMask: hbm_mask,
                hbmColor: hbm_color,
            };

            let hicon = CreateIconIndirect(&info);

            let _ = DeleteObject(hbm_color.into());
            let _ = DeleteObject(hbm_mask.into());

            hicon
        }
    }
}

fn percent_color(pct: f64) -> [u8; 3] {
    if pct < 50.0 {
        let t = pct / 50.0;
        [(80.0 * t) as u8, (180.0 - 80.0 * t) as u8, 30]
    } else if pct < 80.0 {
        let t = (pct - 50.0) / 30.0;
        [220, (180.0 - 140.0 * t) as u8, 30]
    } else {
        let t = ((pct - 80.0) / 20.0).min(1.0);
        [220, (40.0 - 40.0 * t) as u8, 30]
    }
}

pub fn generate_icon(data: &[UsageData]) -> TrayIcon {
    let size = 32u32;
    let mut rgba = vec![0u8; (size * size * 4) as usize];

    let pct = aggregate_percent(data);
    let color = percent_color(pct);
    let label = format_pct(pct);

    let cx = size as f64 / 2.0;
    let cy = size as f64 / 2.0;
    let outer_r = size as f64 / 2.0 - 2.0;
    let ring_width = 4.0;
    let inner_r = outer_r - ring_width;

    let fill_angle = pct / 100.0 * 2.0 * std::f64::consts::PI;

    for y in 0..size {
        for x in 0..size {
            let dx = x as f64 - cx;
            let dy = y as f64 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            let angle = dy.atan2(dx);
            let norm_angle = ((angle + std::f64::consts::FRAC_PI_2) % (2.0 * std::f64::consts::PI)
                + 2.0 * std::f64::consts::PI)
                % (2.0 * std::f64::consts::PI);

            let in_ring = dist >= inner_r && dist <= outer_r;
            let idx = ((y * size + x) * 4) as usize;

            if in_ring {
                let filled = norm_angle <= fill_angle;
                let px_color = if filled { color } else { [55, 55, 55] };
                let d_inner = dist - inner_r;
                let d_outer = outer_r - dist;
                let alpha = if d_inner < 1.0 {
                    d_inner
                } else if d_outer < 1.0 {
                    d_outer
                } else {
                    1.0
                };
                rgba[idx] = px_color[0];
                rgba[idx + 1] = px_color[1];
                rgba[idx + 2] = px_color[2];
                rgba[idx + 3] = (alpha.min(1.0) * 255.0) as u8;
            } else if dist < inner_r {
                rgba[idx] = 20;
                rgba[idx + 1] = 20;
                rgba[idx + 2] = 25;
                rgba[idx + 3] = 240;
            }
        }
    }

    draw_text_centered(&mut rgba, size, &label);
    TrayIcon {
        rgba,
        width: size,
        height: size,
    }
}

fn aggregate_percent(data: &[UsageData]) -> f64 {
    let valid: Vec<f64> = data
        .iter()
        .filter(|d| d.error.is_none() && d.has_data())
        .filter_map(|d| {
            let percents: Vec<f64> = d
                .windows
                .iter()
                .filter(|w| w.label != "No data" && w.label != "Error" && w.label != "Connected")
                .filter(|w| w.used_percent > 0.0)
                .map(|w| w.used_percent)
                .collect();
            if percents.is_empty() {
                None
            } else {
                Some(percents.into_iter().fold(0.0f64, f64::max))
            }
        })
        .collect();

    if valid.is_empty() {
        return 0.0;
    }
    valid.iter().sum::<f64>() / valid.len() as f64
}

fn format_pct(pct: f64) -> String {
    if pct >= 99.5 {
        return "M".to_string();
    }
    let val = pct as u8;
    val.to_string()
}

const FONT_3X5: &[[[u8; 3]; 5]] = &[
    [[1, 1, 1], [1, 0, 1], [1, 0, 1], [1, 0, 1], [1, 1, 1]], // 0
    [[0, 1, 0], [1, 1, 0], [0, 1, 0], [0, 1, 0], [1, 1, 1]], // 1
    [[1, 1, 1], [0, 0, 1], [1, 1, 1], [1, 0, 0], [1, 1, 1]], // 2
    [[1, 1, 1], [0, 0, 1], [1, 1, 1], [0, 0, 1], [1, 1, 1]], // 3
    [[1, 0, 1], [1, 0, 1], [1, 1, 1], [0, 0, 1], [0, 0, 1]], // 4
    [[1, 1, 1], [1, 0, 0], [1, 1, 1], [0, 0, 1], [1, 1, 1]], // 5
    [[1, 1, 1], [1, 0, 0], [1, 1, 1], [1, 0, 1], [1, 1, 1]], // 6
    [[1, 1, 1], [0, 0, 1], [0, 1, 0], [0, 1, 0], [0, 1, 0]], // 7
    [[1, 1, 1], [1, 0, 1], [1, 1, 1], [1, 0, 1], [1, 1, 1]], // 8
    [[1, 1, 1], [1, 0, 1], [1, 1, 1], [0, 0, 1], [1, 1, 1]], // 9
];

fn draw_text_centered(rgba: &mut [u8], size: u32, text: &str) {
    let chars: Vec<u8> = text.bytes().filter(|b| b.is_ascii_digit()).collect();
    let is_max = text.starts_with('M');

    if is_max {
        let pattern: [[u8; 5]; 5] = [
            [1, 0, 0, 0, 1],
            [1, 1, 0, 1, 1],
            [1, 0, 1, 0, 1],
            [1, 0, 0, 0, 1],
            [1, 0, 0, 0, 1],
        ];
        let pw = 5;
        let ph = 5;
        let ox = (size as i32 - pw) / 2;
        let oy = (size as i32 - ph) / 2;
        for row in 0..ph as usize {
            for col in 0..pw as usize {
                if pattern[row][col] != 0 {
                    set_pixel(
                        rgba,
                        size,
                        ox + col as i32,
                        oy + row as i32,
                        220,
                        60,
                        30,
                        255,
                    );
                }
            }
        }
        return;
    }

    let num = chars.len();
    if num == 0 {
        return;
    }

    let char_w: i32 = 3;
    let char_h: usize = 5;
    let gap: i32 = 1;
    let total_w = num as i32 * char_w + (num as i32 - 1) * gap;
    let total_h = char_h as i32;
    let ox = (size as i32 - total_w) / 2;
    let oy = (size as i32 - total_h) / 2;

    for (i, &d) in chars.iter().enumerate() {
        let dx = ox + i as i32 * (char_w + gap);
        let digit = (d - b'0') as usize;
        if digit >= 10 {
            continue;
        }
        let glyph = &FONT_3X5[digit];

        for row in 0..char_h {
            for col in 0..char_w as usize {
                if glyph[row][col] != 0 {
                    let px = dx + col as i32;
                    let py = oy + row as i32;
                    set_pixel(rgba, size, px, py, 255, 255, 255, 255);
                }
            }
        }
    }
}

fn set_pixel(rgba: &mut [u8], size: u32, x: i32, y: i32, r: u8, g: u8, b: u8, a: u8) {
    if x >= 0 && x < size as i32 && y >= 0 && y < size as i32 {
        let idx = ((y as u32 * size + x as u32) * 4) as usize;
        rgba[idx] = r;
        rgba[idx + 1] = g;
        rgba[idx + 2] = b;
        rgba[idx + 3] = a;
    }
}
