/// Maps vertical fill levels (0..4) to braille column dot bitmasks
/// Col 0 dots: Row 3=0x40 (Dot 7), Row 2=0x04 (Dot 3), Row 1=0x02 (Dot 2), Row 0=0x01 (Dot 1)
/// Col 1 dots: Row 3=0x80 (Dot 8), Row 2=0x20 (Dot 6), Row 1=0x10 (Dot 5), Row 0=0x08 (Dot 4)
const COL0_MASKS: [u8; 5] = [
    0x00,                      // 0 dots
    0x40,                      // 1 dot (bottom)
    0x40 | 0x04,               // 2 dots
    0x40 | 0x04 | 0x02,        // 3 dots
    0x40 | 0x04 | 0x02 | 0x01, // 4 dots (full)
];

const COL1_MASKS: [u8; 5] = [
    0x00,                      // 0 dots
    0x80,                      // 1 dot (bottom)
    0x80 | 0x20,               // 2 dots
    0x80 | 0x20 | 0x10,        // 3 dots
    0x80 | 0x20 | 0x10 | 0x08, // 4 dots (full)
];

/// Render a 2-row high-density Braille waveform from a time-series history of throughput values.
/// Returns a tuple of (top_row_string, bottom_row_string).
pub fn render_braille_canvas(history: &[f64], char_width: usize) -> (String, String) {
    if char_width == 0 {
        return (String::new(), String::new());
    }

    let col_count = char_width * 2;
    // Resample or take latest `col_count` values
    let mut columns = vec![0.0f64; col_count];
    if history.is_empty() {
        // Empty waveform
        let empty_line = " ".repeat(char_width);
        return (empty_line.clone(), empty_line);
    }

    let max_val = history.iter().cloned().fold(1.0f64, f64::max);
    let history_len = history.len();

    for i in 0..col_count {
        if history_len <= col_count {
            let offset = col_count - history_len;
            if i >= offset {
                columns[i] = history[i - offset] / max_val;
            }
        } else {
            // Sample evenly from history
            let idx = (i * history_len) / col_count;
            columns[i] = history[idx] / max_val;
        }
    }

    // Total height = 8 levels (4 for bottom row, 4 for top row)
    let mut top_chars = String::with_capacity(char_width);
    let mut bot_chars = String::with_capacity(char_width);

    for c in 0..char_width {
        let val_left = columns[c * 2].clamp(0.0, 1.0);
        let val_right = columns[c * 2 + 1].clamp(0.0, 1.0);

        let total_level_left = (val_left * 8.0).round() as usize;
        let total_level_right = (val_right * 8.0).round() as usize;

        // Bottom row: levels 0..4
        let bot_lvl_l = total_level_left.min(4);
        let bot_lvl_r = total_level_right.min(4);
        let bot_byte = COL0_MASKS[bot_lvl_l] | COL1_MASKS[bot_lvl_r];
        let bot_char = if bot_byte == 0 {
            ' '
        } else {
            char::from_u32(0x2800 + bot_byte as u32).unwrap_or(' ')
        };
        bot_chars.push(bot_char);

        // Top row: levels 4..8
        let top_lvl_l = total_level_left.saturating_sub(4).min(4);
        let top_lvl_r = total_level_right.saturating_sub(4).min(4);
        let top_byte = COL0_MASKS[top_lvl_l] | COL1_MASKS[top_lvl_r];
        let top_char = if top_byte == 0 {
            ' '
        } else {
            char::from_u32(0x2800 + top_byte as u32).unwrap_or(' ')
        };
        top_chars.push(top_char);
    }

    (top_chars, bot_chars)
}

/// Render a dynamic tachometer gauge: `[████████░░░░░░] 524.8 / 600 Mbps`
pub fn render_tachometer(current_mbps: f64, max_mbps: f64, bar_width: usize) -> String {
    let ratio = if max_mbps > 0.0 {
        (current_mbps / max_mbps).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let filled_len = (ratio * bar_width as f64).round() as usize;
    let unfilled_len = bar_width.saturating_sub(filled_len);

    let filled = "█".repeat(filled_len);
    let unfilled = "░".repeat(unfilled_len);

    format!("[{}{}]", filled, unfilled)
}

/// Calculates an intuitive dynamic gauge upper scale (e.g. 100, 250, 500, 1000, 2500, 5000, 10000 Mbps)
pub fn calculate_gauge_scale(peak_mbps: f64) -> f64 {
    let scales = [50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0, 10000.0];
    for &scale in &scales {
        if peak_mbps <= scale * 0.90 {
            return scale;
        }
    }
    ((peak_mbps / 1000.0).ceil() + 1.0) * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_braille_canvas() {
        let history = vec![0.0, 10.0, 50.0, 100.0, 80.0, 30.0, 10.0, 0.0];
        let (top, bot) = render_braille_canvas(&history, 10);
        assert_eq!(top.chars().count(), 10);
        assert_eq!(bot.chars().count(), 10);
        // Ensure valid braille characters are produced
        assert!(
            bot.chars()
                .any(|c| (c as u32) >= 0x2800 && (c as u32) <= 0x28FF)
        );
    }

    #[test]
    fn test_tachometer() {
        let bar = render_tachometer(50.0, 100.0, 10);
        assert_eq!(bar, "[█████░░░░░]");
    }
}
