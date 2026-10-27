// Kiwi Art & Aesthetics Engine
// High-aesthetic TrueColor, point-cloud dithering, and micro-border rendering.

pub const KIWI_EMERALD: (u8, u8, u8) = (0, 245, 160); // #00F5A0 - Bioluminescent green
pub const CYBER_CYAN: (u8, u8, u8) = (0, 217, 245); // #00D9F5 - Electric cyan
pub const NEON_MAGENTA: (u8, u8, u8) = (247, 37, 133); // #F72585 - Hot cyber magenta
pub const DEEP_VIOLET: (u8, u8, u8) = (121, 40, 202); // #7928CA - Void violet
pub const ELECTRIC_AMBER: (u8, u8, u8) = (255, 183, 3); // #FFB703 - Warm accent / warning
pub const BORDER_COLOR: (u8, u8, u8) = (45, 55, 72); // #2D3748 - Sleek slate border
pub const DIM_GRAY: (u8, u8, u8) = (100, 116, 139); // #64748B - Muted labels
pub const BRIGHT_WHITE: (u8, u8, u8) = (241, 245, 249); // #F1F5F9 - Text highlight
pub const ICE_WHITE: (u8, u8, u8) = (224, 254, 254); // #E0FEFE - Core glow

/// Strip ANSI escape sequences from a string to measure printable width
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Approximate display width in terminal columns (accounting for wide emoji and zero-width codepoints)
pub fn char_width(c: char) -> usize {
    let cp = c as u32;
    // Zero-width characters (Variation selectors, zero-width spaces/joiners)
    if (0xFE00..=0xFE0F).contains(&cp) || (0x200B..=0x200D).contains(&cp) {
        0
    } else if (0x1F300..=0x1FAFF).contains(&cp)
        || (0x2600..=0x27BF).contains(&cp)
        || (0x2E80..=0x9FFF).contains(&cp)
        || (0xF900..=0xFAFF).contains(&cp)
        || (0xFF01..=0xFF60).contains(&cp)
    {
        2
    } else {
        1
    }
}

pub fn visible_width(s: &str) -> usize {
    strip_ansi(s).chars().map(char_width).sum()
}

pub fn color_fg(s: &str, rgb: (u8, u8, u8)) -> String {
    format!("\x1b[38;2;{};{};{}m{}\x1b[0m", rgb.0, rgb.1, rgb.2, s)
}

pub fn color_bold(s: &str, rgb: (u8, u8, u8)) -> String {
    format!("\x1b[1;38;2;{};{};{}m{}\x1b[0m", rgb.0, rgb.1, rgb.2, s)
}

pub fn lerp_rgb(c1: (u8, u8, u8), c2: (u8, u8, u8), t: f64) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    (
        (c1.0 as f64 + (c2.0 as f64 - c1.0 as f64) * t).round() as u8,
        (c1.1 as f64 + (c2.1 as f64 - c1.1 as f64) * t).round() as u8,
        (c1.2 as f64 + (c2.2 as f64 - c1.2 as f64) * t).round() as u8,
    )
}

pub fn multi_lerp(stops: &[(f64, (u8, u8, u8))], t: f64) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    if stops.is_empty() {
        return (255, 255, 255);
    }
    if t <= stops[0].0 {
        return stops[0].1;
    }
    for i in 0..stops.len() - 1 {
        let (t0, c0) = stops[i];
        let (t1, c1) = stops[i + 1];
        if t >= t0 && t <= t1 {
            let local_t = if (t1 - t0).abs() < 1e-6 {
                0.0
            } else {
                (t - t0) / (t1 - t0)
            };
            return lerp_rgb(c0, c1, local_t);
        }
    }
    stops.last().unwrap().1
}

/// Applies a smooth 24-bit TrueColor gradient across characters
pub fn gradient_text(text: &str, stops: &[(f64, (u8, u8, u8))]) -> String {
    let count = text.chars().count();
    if count == 0 {
        return String::new();
    }
    let mut out = String::with_capacity(text.len() * 16);
    for (i, c) in text.chars().enumerate() {
        let t = if count > 1 {
            i as f64 / (count - 1) as f64
        } else {
            0.5
        };
        let rgb = multi_lerp(stops, t);
        out.push_str(&format!("\x1b[38;2;{};{};{}m{}", rgb.0, rgb.1, rgb.2, c));
    }
    out.push_str("\x1b[0m");
    out
}

/// Pad or safely truncate a string to fit exactly `target_width` columns
pub fn pad_to_width(content: &str, target_width: usize) -> String {
    let vis = visible_width(content);
    if vis < target_width {
        let pad = " ".repeat(target_width - vis);
        format!("{}{}", content, pad)
    } else if vis > target_width {
        // Safe truncate: keep printable chars up to target_width
        let mut out = String::new();
        let mut cur_width = 0;
        let mut in_escape = false;
        for c in content.chars() {
            if c == '\x1b' {
                in_escape = true;
                out.push(c);
            } else if in_escape {
                out.push(c);
                if c.is_ascii_alphabetic() {
                    in_escape = false;
                }
            } else {
                let w = char_width(c);
                if cur_width + w > target_width {
                    break;
                }
                cur_width += w;
                out.push(c);
            }
        }
        out.push_str("\x1b[0m");
        if cur_width < target_width {
            out.push_str(&" ".repeat(target_width - cur_width));
        }
        out
    } else {
        content.to_string()
    }
}

/// Generate a rounded box top: `╭─ [ Title ] ────────────────────────────────╮`
pub fn box_top(title: &str, width: usize) -> String {
    let border_fg = color_fg("─", BORDER_COLOR);
    let corner_l = color_fg("╭", BORDER_COLOR);
    let corner_r = color_fg("╮", BORDER_COLOR);

    if title.is_empty() {
        let h_bar = color_fg(&"─".repeat(width.saturating_sub(2)), BORDER_COLOR);
        return format!("{}{}{}", corner_l, h_bar, corner_r);
    }

    let left_dashes = 2;
    let max_title_vis = width.saturating_sub(left_dashes + 4);
    let safe_title = if visible_width(title) > max_title_vis {
        pad_to_width(title, max_title_vis)
    } else {
        title.to_string()
    };
    let title_vis = visible_width(&safe_title);
    let right_dashes = width.saturating_sub(left_dashes + title_vis + 4);

    format!(
        "{}{}{}{}{}{}",
        corner_l,
        border_fg.repeat(left_dashes),
        " ",
        safe_title,
        " ",
        border_fg.repeat(right_dashes),
    ) + &corner_r
}

/// Generate a rounded box bottom: `╰───────────────────────────────────────────╯`
pub fn box_bot(width: usize) -> String {
    let corner_l = color_fg("╰", BORDER_COLOR);
    let corner_r = color_fg("╯", BORDER_COLOR);
    let h_bar = color_fg(&"─".repeat(width.saturating_sub(2)), BORDER_COLOR);
    format!("{}{}{}", corner_l, h_bar, corner_r)
}

/// Generate a card row enclosed by rounded borders: `│ {content} │`
pub fn box_row(content: &str, width: usize) -> String {
    let border_l = color_fg("│ ", BORDER_COLOR);
    let border_r = color_fg(" │", BORDER_COLOR);
    let inner_width = width.saturating_sub(4);
    let padded = pad_to_width(content, inner_width);
    format!("{}{}{}", border_l, padded, border_r)
}

/// Generate a card dot-matrix separator: `│ · · · · · · · · · · · · · · · · · · · · · │`
pub fn box_sep(width: usize) -> String {
    let inner_width = width.saturating_sub(4);
    let mut dots = String::new();
    let count = inner_width / 2;
    for i in 0..count {
        dots.push('·');
        if i + 1 < count {
            dots.push(' ');
        }
    }
    let colored_dots = color_fg(&dots, (55, 65, 81));
    box_row(&colored_dots, width)
}

/// Generates the Ethereal Bioluminescent Cyber-Kiwi Mascot / Dithered Point-Cloud Art.
/// Size: 8 rows tall x 26 chars wide.
/// Uses character density ramps: `· : ÷ + * # % @` and Braille `⡀⣀⣠⣤⣶⣾⣿`
/// with 24-bit TrueColor gradients (Deep Violet -> Neon Magenta -> Cyber Cyan -> Kiwi Emerald).
pub fn render_cyber_kiwi_point_cloud() -> Vec<String> {
    // 8 lines of handcrafted, mathematical point-cloud dither (exact 26 chars per row)
    let raw_grid: [&str; 8] = [
        "    . · ⡀ ÷ ⣠⣄ ÷ ⡀ · .    ",
        "  · ÷ * # ⣶ ⣿⣿ ⣶ # * ÷ ·  ",
        " : ÷ # @ ⣿ ⠿  ⠿ ⣿ @ # ÷ : ",
        "· ÷ % ⣾ ⡿ ⠋ ·· ⠙ ⢿ ⣾ % ÷ ·",
        ": + # ⣿ ⡿ · ⠂⠐ · ⢿ ⣿ # + :",
        "· ÷ % ⣾ ⡿ ⠋ ·· ⠙ ⢿ ⣾ % ÷ ·",
        " : ÷ # @ ⣿ ⠿  ⠿ ⣿ @ # ÷ : ",
        "  · ⣀ * # ⣶ ⣿⣿ ⣶ # * ⣀ ·  ",
    ];

    let stops = [
        (0.0, DEEP_VIOLET),
        (0.25, NEON_MAGENTA),
        (0.50, KIWI_EMERALD),
        (0.75, CYBER_CYAN),
        (1.0, ICE_WHITE),
    ];

    let mut out = Vec::with_capacity(8);
    let cy = 3.5;
    let cx = 12.5;

    for (y, row) in raw_grid.iter().enumerate() {
        let mut line = String::new();
        let dy = (y as f64 - cy).abs();

        let chars: Vec<char> = row.chars().collect();

        for (x, &ch) in chars.iter().enumerate() {
            if ch == ' ' {
                line.push(' ');
                continue;
            }

            let dx = (x as f64 - cx).abs() * 0.55;
            let dist = (dx * dx + dy * dy).sqrt();

            // Distance based normalized value (0.0 at center, 1.0 at outer edge)
            let norm = (dist / 3.8).clamp(0.0, 1.0);

            // TrueColor mapping:
            // Center is luminous cyan/white, seeds are electric magenta/amber,
            // pericarp flesh is kiwi emerald, outer skin/halo is violet/magenta
            let rgb = match ch {
                '·' | '.' | '⠂' | '⠐' | '⠠' if dist < 1.4 => ICE_WHITE,
                ':' if dist < 1.6 => CYBER_CYAN,
                '⠋' | '⠙' | '⠁' | '⠈' => ELECTRIC_AMBER,
                '⣿' | '⣾' | '⣶' | '⡿' | '⢿' | '⠿' => KIWI_EMERALD,
                '@' | '%' | '#' => {
                    if dist < 2.3 {
                        KIWI_EMERALD
                    } else {
                        NEON_MAGENTA
                    }
                }
                '*' | '+' | '÷' | '⣠' | '⣄' | '⣀' | '⡀' => {
                    if dist > 2.6 {
                        DEEP_VIOLET
                    } else {
                        NEON_MAGENTA
                    }
                }
                _ => multi_lerp(&stops, 1.0 - norm),
            };

            line.push_str(&color_fg(&ch.to_string(), rgb));
        }

        out.push(pad_to_width(&line, 26));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_art_point_cloud_dimensions() {
        let art = render_cyber_kiwi_point_cloud();
        assert_eq!(art.len(), 8);
        for line in &art {
            let vis = visible_width(line);
            assert_eq!(vis, 26);
        }
    }

    #[test]
    fn test_pad_to_width() {
        let s = "Hello";
        assert_eq!(pad_to_width(s, 10), "Hello     ");
        let wide = "🥝 Kiwi"; // 2 + 1 + 4 = 7
        assert_eq!(visible_width(wide), 7);
        let padded = pad_to_width(wide, 10);
        assert_eq!(visible_width(&padded), 10);

        // Test with zero-width variation selector
        let emoji_with_vs = "🏋️ Weights"; // 2 + 0 + 1 + 7 = 10
        assert_eq!(visible_width(emoji_with_vs), 10);
    }

    #[test]
    fn test_box_borders() {
        let top = box_top("TEST", 20);
        assert_eq!(visible_width(&top), 20);
        let bot = box_bot(20);
        assert_eq!(visible_width(&bot), 20);
        let row = box_row("Content", 20);
        assert_eq!(visible_width(&row), 20);

        // Overflowing title protection
        let overflow_top = box_top("A VERY LONG TITLE THAT EXCEEDS WIDTH", 20);
        assert_eq!(visible_width(&overflow_top), 20);
    }
}
