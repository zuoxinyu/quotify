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

fn in_rounded_rect(x: f64, y: f64, w: f64, h: f64, r: f64) -> bool {
    if x < 0.0 || x >= w || y < 0.0 || y >= h {
        return false;
    }
    if x < r && y < r {
        let dx = x - r;
        let dy = y - r;
        return dx * dx + dy * dy <= r * r;
    }
    if x > w - r && y < r {
        let dx = x - (w - r);
        let dy = y - r;
        return dx * dx + dy * dy <= r * r;
    }
    if x < r && y > h - r {
        let dx = x - r;
        let dy = y - (h - r);
        return dx * dx + dy * dy <= r * r;
    }
    if x > w - r && y > h - r {
        let dx = x - (w - r);
        let dy = y - (h - r);
        return dx * dx + dy * dy <= r * r;
    }
    true
}

fn in_progress_bar(px: f64, py: f64, bar_y: f64, width: f64) -> bool {
    let bar_x = 6.0;
    let bar_h = 3.0;
    let rx = 1.5;

    if px < bar_x || px >= bar_x + width || py < bar_y || py >= bar_y + bar_h {
        return false;
    }

    if px < bar_x + rx && py < bar_y + rx {
        let dx = px - (bar_x + rx);
        let dy = py - (bar_y + rx);
        return dx * dx + dy * dy <= rx * rx;
    }
    if px > bar_x + width - rx && py < bar_y + rx {
        let dx = px - (bar_x + width - rx);
        let dy = py - (bar_y + rx);
        return dx * dx + dy * dy <= rx * rx;
    }
    if px < bar_x + rx && py > bar_y + bar_h - rx {
        let dx = px - (bar_x + rx);
        let dy = py - (bar_y + bar_h - rx);
        return dx * dx + dy * dy <= rx * rx;
    }
    if px > bar_x + width - rx && py > bar_y + bar_h - rx {
        let dx = px - (bar_x + width - rx);
        let dy = py - (bar_y + bar_h - rx);
        return dx * dx + dy * dy <= rx * rx;
    }

    true
}

fn blend_rgba(bg: [u8; 4], fg: [u8; 4]) -> [u8; 4] {
    let alpha_fg = fg[3] as f64 / 255.0;
    let alpha_bg = bg[3] as f64 / 255.0 * (1.0 - alpha_fg);
    let alpha_out = alpha_fg + alpha_bg;

    if alpha_out == 0.0 {
        return [0, 0, 0, 0];
    }

    let r = (fg[0] as f64 * alpha_fg + bg[0] as f64 * alpha_bg) / alpha_out;
    let g = (fg[1] as f64 * alpha_fg + bg[1] as f64 * alpha_bg) / alpha_out;
    let b = (fg[2] as f64 * alpha_fg + bg[2] as f64 * alpha_bg) / alpha_out;

    [
        r.clamp(0.0, 255.0) as u8,
        g.clamp(0.0, 255.0) as u8,
        b.clamp(0.0, 255.0) as u8,
        (alpha_out * 255.0).clamp(0.0, 255.0) as u8,
    ]
}

fn get_progress_percentages(data: &[UsageData], active_provider: Option<&str>) -> [f64; 3] {
    let mut pcts = [0.0; 3];

    if let Some(active_provider) = active_provider
        .map(str::trim)
        .filter(|provider| !provider.is_empty())
    {
        if let Some(provider_data) = data
            .iter()
            .find(|d| d.provider.eq_ignore_ascii_case(active_provider))
            && provider_data.error.is_none()
        {
            let windows = valid_windows(provider_data);
            for (i, w) in windows.iter().take(3).enumerate() {
                pcts[i] = w.used_percent;
            }
            if windows.len() == 1 {
                pcts[1] = pcts[0];
                pcts[2] = pcts[0];
            } else if windows.len() == 2 {
                pcts[2] = (pcts[0] + pcts[1]) / 2.0;
            }
        }
    } else {
        let mut provider_pcts: Vec<f64> = data.iter().filter_map(provider_percent).collect();

        provider_pcts.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));

        if provider_pcts.len() == 1 {
            pcts[0] = provider_pcts[0];
            pcts[1] = provider_pcts[0];
            pcts[2] = provider_pcts[0];
        } else if provider_pcts.len() == 2 {
            pcts[0] = provider_pcts[0];
            pcts[1] = provider_pcts[1];
            pcts[2] = (provider_pcts[0] + provider_pcts[1]) / 2.0;
        } else {
            for (i, &pct) in provider_pcts.iter().take(3).enumerate() {
                pcts[i] = pct;
            }
        }
    }

    pcts
}

fn draw_provider_watermark(px: f64, py: f64, provider: &str) -> Option<[u8; 4]> {
    let dx = px - 16.0;
    let dy = py - 16.0;

    match provider.to_ascii_lowercase().as_str() {
        "gemini" => {
            // 4-pointed sparkle
            if dx.abs().powf(0.65) + dy.abs().powf(0.65) <= 5.5_f64.powf(0.65) {
                Some([24, 144, 255, 35])
            } else {
                None
            }
        }
        "claude" => {
            // 3-petaled clover/crown
            let d1 = dx * dx + (dy + 2.5) * (dy + 2.5);
            let d2 = (dx + 2.5) * (dx + 2.5) + (dy - 1.5) * (dy - 1.5);
            let d3 = (dx - 2.5) * (dx - 2.5) + (dy - 1.5) * (dy - 1.5);
            if d1 <= 2.2 * 2.2 || d2 <= 2.2 * 2.2 || d3 <= 2.2 * 2.2 {
                Some([217, 107, 83, 30])
            } else {
                None
            }
        }
        "deepseek" => {
            // Sleek shield
            let adx = dx.abs();
            let inside = if (-6.0..=2.0).contains(&dy) {
                adx <= 5.0
            } else if dy > 2.0 && dy <= 7.0 {
                adx <= 5.0 * (1.0 - (dy - 2.0) / 5.0)
            } else {
                false
            };
            if inside {
                Some([0, 106, 255, 30])
            } else {
                None
            }
        }
        "opencode" | "codex" => {
            // Stylized code tag </ >
            let is_slash = (dx + 2.0 * dy).abs() <= 1.0 && dy.abs() <= 5.0;
            let is_left = (dx + dy.abs() - 1.5).abs() <= 0.8 && dx <= -0.5 && dy.abs() <= 3.5;
            let is_right = (dx - dy.abs() + 1.5).abs() <= 0.8 && dx >= 0.5 && dy.abs() <= 3.5;

            if is_slash || is_left || is_right {
                if provider.eq_ignore_ascii_case("opencode") {
                    Some([16, 124, 65, 30])
                } else {
                    Some([120, 120, 120, 30])
                }
            } else {
                None
            }
        }
        "antigravity" => {
            // Upward vector arrow
            let adx = dx.abs();
            let is_shaft = adx <= 1.2 && (-1.0..=6.0).contains(&dy);
            let is_head = (-6.0..-1.0).contains(&dy) && adx <= (dy + 6.0) * 0.9;
            if is_shaft || is_head {
                Some([128, 0, 255, 35])
            } else {
                None
            }
        }
        "mimo" => {
            // Stylized "M"
            let adx = dx.abs();
            let is_shaft = (3.5..=5.5).contains(&adx) && dy.abs() <= 5.0;
            let is_diag = (-5.0..=5.0).contains(&dy) && (adx - (dy + 5.0) / 2.0).abs() <= 0.8;
            if is_shaft || is_diag {
                Some([255, 128, 0, 30])
            } else {
                None
            }
        }
        _ => None,
    }
}

pub fn generate_icon(data: &[UsageData], active_provider: Option<&str>) -> TrayIcon {
    let size = 32u32;
    let mut rgba = vec![0u8; (size * size * 4) as usize];

    let pcts = get_progress_percentages(data, active_provider);
    let bar_ys = [9.0, 14.0, 19.0];

    for y in 0..size {
        for x in 0..size {
            let mut sub_color = [0.0; 4];

            for sy in 0..3 {
                for sx in 0..3 {
                    let px = x as f64 + (sx as f64 + 0.5) / 3.0;
                    let py = y as f64 + (sy as f64 + 0.5) / 3.0;

                    let c = if in_rounded_rect(px, py, 32.0, 32.0, 7.0) {
                        let mut base = [234, 218, 215, 255];

                        if let Some(provider_name) = active_provider
                            && let Some(w_color) = draw_provider_watermark(px, py, provider_name)
                        {
                            base = blend_rgba(base, w_color);
                        }

                        if !in_rounded_rect(px, py, 32.0, 32.0, 6.0) {
                            base = blend_rgba(base, [109, 109, 109, 25]);
                        }

                        let spy = py - 0.5;
                        let mut in_shadow = false;
                        for &bar_y in &bar_ys {
                            if in_progress_bar(px, spy, bar_y, 20.0) {
                                in_shadow = true;
                                break;
                            }
                        }
                        if in_shadow {
                            base = blend_rgba(base, [128, 128, 128, 50]);
                        }

                        for (i, &bar_y) in bar_ys.iter().enumerate() {
                            let pct = pcts[i];
                            let fill_w = (pct / 100.0 * 20.0).clamp(0.0, 20.0);

                            if in_progress_bar(px, py, bar_y, fill_w) {
                                base = blend_rgba(base, [0, 120, 212, 255]);
                            } else if in_progress_bar(px, py, bar_y, 20.0) {
                                base = blend_rgba(base, [0, 120, 212, 30]);
                            }
                        }

                        base
                    } else {
                        [0, 0, 0, 0]
                    };

                    for i in 0..4 {
                        sub_color[i] += c[i] as f64;
                    }
                }
            }

            let idx = ((y * size + x) * 4) as usize;
            for i in 0..4 {
                rgba[idx + i] = (sub_color[i] / 9.0).clamp(0.0, 255.0) as u8;
            }
        }
    }

    TrayIcon {
        rgba,
        width: size,
        height: size,
    }
}

pub fn tray_tooltip(data: &[UsageData], active_provider: Option<&str>) -> String {
    let Some(active_provider) = active_provider
        .map(str::trim)
        .filter(|provider| !provider.is_empty())
    else {
        let pct = aggregate_percent(data);
        return format!("Quotify - AI Quota Monitor\nAll providers: {pct:.0}%");
    };

    let display_name = provider_display_name(active_provider);
    let Some(provider) = data
        .iter()
        .find(|data| data.provider.eq_ignore_ascii_case(active_provider))
    else {
        return format!("Quotify - {display_name}\nNo usage data");
    };

    if let Some(error) = provider.error.as_deref() {
        return format!("Quotify - {display_name}\nError: {error}");
    }

    let windows = valid_windows(provider);
    if windows.is_empty() {
        return format!("Quotify - {display_name}\nNo usage data");
    }

    let max_pct = provider_percent(provider).unwrap_or(0.0);
    let details = windows
        .into_iter()
        .take(3)
        .map(|window| format!("{} {:.0}%", window.label, window.used_percent))
        .collect::<Vec<_>>()
        .join(", ");
    format!("Quotify - {display_name}\nMax {max_pct:.0}%\n{details}")
}

fn provider_percent(data: &UsageData) -> Option<f64> {
    if data.error.is_some() || !data.has_data() {
        return None;
    }

    let percents = valid_windows(data)
        .into_iter()
        .filter(|window| window.used_percent > 0.0)
        .map(|window| window.used_percent);
    percents.reduce(f64::max)
}

fn valid_windows(data: &UsageData) -> Vec<&crate::provider::UsageWindow> {
    data.windows
        .iter()
        .filter(|w| w.label != "No data" && w.label != "Error" && w.label != "Connected" && w.label != "Reset Credits")
        .collect()
}

fn aggregate_percent(data: &[UsageData]) -> f64 {
    let valid: Vec<f64> = data.iter().filter_map(provider_percent).collect();

    if valid.is_empty() {
        return 0.0;
    }
    valid.iter().sum::<f64>() / valid.len() as f64
}

fn provider_display_name(provider: &str) -> String {
    match provider.to_ascii_lowercase().as_str() {
        "codex" => "Codex".to_string(),
        "openai" => "OpenAI".to_string(),
        "opencode" => "OpenCode".to_string(),
        "opencodego" => "OpenCode Go".to_string(),
        "claude" => "Claude".to_string(),
        "gemini" => "Gemini".to_string(),
        "antigravity" => "Antigravity".to_string(),
        "deepseek" => "DeepSeek".to_string(),
        "openrouter" => "OpenRouter".to_string(),
        "moonshot" => "Moonshot".to_string(),
        "elevenlabs" => "ElevenLabs".to_string(),
        "doubao" => "Doubao".to_string(),
        "zai" => "z.ai".to_string(),
        "venice" => "Venice".to_string(),
        "crof" => "Crof".to_string(),
        "synthetic" => "Synthetic".to_string(),
        "warp" => "Warp".to_string(),
        "groqcloud" => "GroqCloud".to_string(),
        "deepgram" => "Deepgram".to_string(),
        "llmproxy" => "LLM Proxy".to_string(),
        "codebuff" => "Codebuff".to_string(),
        "kiro" => "Kiro".to_string(),
        "copilot" => "Copilot".to_string(),
        "azureopenai" => "Azure OpenAI".to_string(),
        "ollama" => "Ollama".to_string(),
        "minimax" => "MiniMax".to_string(),
        "jetbrains" => "JetBrains AI".to_string(),
        "kimi" => "Kimi".to_string(),
        "kilo" => "Kilo Code".to_string(),
        "augment" => "Augment".to_string(),
        "bedrock" => "AWS Bedrock".to_string(),
        "vertexai" => "Vertex AI".to_string(),
        "stepfun" => "StepFun".to_string(),
        "abacus" => "Abacus AI".to_string(),
        "alibabatoken" => "Alibaba Token".to_string(),
        "t3chat" => "T3 Chat".to_string(),
        "amp" => "Amp".to_string(),
        "mistral" => "Mistral".to_string(),
        "grok" => "Grok".to_string(),
        "cursor" => "Cursor".to_string(),
        "droid" => "Factory Droid".to_string(),
        "windsurf" => "Windsurf".to_string(),
        "mimo" => "MiMo".to_string(),
        other => other.to_string(),
    }
}
