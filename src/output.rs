use std::{error::Error, fmt::Write, path::Path};

use clap::ValueEnum;
use image::{Rgba, RgbaImage};

use crate::colors::Rgb;

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// One #rrggbb per line
    Hex,
    /// JSON array of {hex, rgb} objects
    Json,
    /// CSS custom properties (--acolor-1, ...)
    Css,
    /// GIMP / Inkscape / Krita palette file
    Gpl,
    /// Color swatches in the terminal
    Term,
}

fn hex(&(r, g, b): &Rgb) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

pub fn render(palette: &[Rgb], format: Format) -> String {
    let mut out = String::new();
    match format {
        Format::Hex => {
            for c in palette {
                writeln!(out, "{}", hex(c)).unwrap();
            }
        }
        Format::Json => {
            let items: Vec<String> = palette
                .iter()
                .map(|c @ &(r, g, b)| {
                    format!("  {{ \"hex\": \"{}\", \"rgb\": [{r}, {g}, {b}] }}", hex(c))
                })
                .collect();
            writeln!(out, "[\n{}\n]", items.join(",\n")).unwrap();
        }
        Format::Css => {
            writeln!(out, ":root {{").unwrap();
            for (i, c) in palette.iter().enumerate() {
                writeln!(out, "  --acolor-{}: {};", i + 1, hex(c)).unwrap();
            }
            writeln!(out, "}}").unwrap();
        }
        Format::Gpl => {
            writeln!(out, "GIMP Palette\nName: acolor\nColumns: {}\n#", palette.len()).unwrap();
            for (i, &(r, g, b)) in palette.iter().enumerate() {
                writeln!(out, "{r:3} {g:3} {b:3}\tacolor-{}", i + 1).unwrap();
            }
        }
        Format::Term => {
            let cols = console::Term::stdout()
                .size_checked()
                .map_or(DEFAULT_COLS, |(_, cols)| cols as usize);
            out = render_swatches(palette, cols, ColorMode::detect());
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Terminal swatches
// ---------------------------------------------------------------------------

/// Width assumed when we can't ask the terminal (e.g. `-f term | less -R`).
const DEFAULT_COLS: usize = 80;
/// Swatch width limits, in terminal columns. The minimum fits " #rrggbb ".
const MIN_SWATCH: usize = 9;
const MAX_SWATCH: usize = 16;
const GAP: usize = 1; // blank columns between swatches
const MARGIN: usize = 2; // blank columns before the first swatch

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorMode {
    /// 24-bit "\x1b[48;2;R;G;Bm": exact colors
    TrueColor,
    /// 256-color "\x1b[48;5;Nm": nearest xterm palette entry
    Ansi256,
}

impl ColorMode {
    /// Terminals that support 24-bit color advertise it via COLORTERM
    /// (Ghostty, kitty, WezTerm, iTerm2, GNOME Terminal, Alacritty, ...).
    /// Anything else gets the 256-color approximation, which every modern
    /// terminal supports, rather than risking garbage escape codes.
    fn detect() -> Self {
        match std::env::var("COLORTERM").as_deref() {
            Ok("truecolor" | "24bit") => ColorMode::TrueColor,
            _ => ColorMode::Ansi256,
        }
    }

    fn bg(self, (r, g, b): Rgb) -> String {
        match self {
            ColorMode::TrueColor => format!("\x1b[48;2;{r};{g};{b}m"),
            ColorMode::Ansi256 => format!("\x1b[48;5;{}m", to_ansi256((r, g, b))),
        }
    }
}

/// Nearest entry in the xterm 256-color palette: either the 6x6x6 color
/// cube (16-231) or the 24-step grey ramp (232-255), whichever is closer.
fn to_ansi256((r, g, b): Rgb) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let nearest_level = |v: u8| -> usize {
        (0..6).min_by_key(|&i| (LEVELS[i] as i32 - v as i32).abs()).unwrap()
    };
    let dist = |(r2, g2, b2): Rgb| -> i32 {
        let d = |a: u8, b: u8| (a as i32 - b as i32).pow(2);
        d(r, r2) + d(g, g2) + d(b, b2)
    };

    let (ri, gi, bi) = (nearest_level(r), nearest_level(g), nearest_level(b));
    let cube_index = 16 + 36 * ri + 6 * gi + bi;
    let cube_color = (LEVELS[ri], LEVELS[gi], LEVELS[bi]);

    // Grey ramp: 8, 18, ..., 238
    let avg = (r as u32 + g as u32 + b as u32) / 3;
    let gi = ((avg as i32 - 8) as f32 / 10.0).round().clamp(0.0, 23.0) as u8;
    let grey = 8 + 10 * gi;

    if dist((grey, grey, grey)) < dist(cube_color) {
        232 + gi
    } else {
        cube_index as u8
    }
}

/// Black or white, whichever is more readable on `bg` (WCAG relative
/// luminance; 0.179 is where black and white have equal contrast).
fn text_color_for(bg: Rgb) -> &'static str {
    let linear = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let luminance = 0.2126 * linear(bg.0) + 0.7152 * linear(bg.1) + 0.0722 * linear(bg.2);
    if luminance > 0.179 {
        "\x1b[30m" // black
    } else {
        "\x1b[97m" // bright white
    }
}

/// Renders the palette as a strip of swatches, 3 rows tall, with each hex
/// code centered inside its swatch:
///
/// ```text
///   ▒▒▒▒▒▒▒▒▒▒▒▒ ▒▒▒▒▒▒▒▒▒▒▒▒ ▒▒▒▒▒▒▒▒▒▒▒▒
///   ▒▒#3a1427▒▒▒ ▒▒#9afa39▒▒▒ ▒▒#23f923▒▒▒
///   ▒▒▒▒▒▒▒▒▒▒▒▒ ▒▒▒▒▒▒▒▒▒▒▒▒ ▒▒▒▒▒▒▒▒▒▒▒▒
/// ```
///
/// Swatches stretch to fill the terminal (up to MAX_SWATCH wide) and wrap
/// onto more strips if even MIN_SWATCH-wide ones don't fit on one line.
pub fn render_swatches(palette: &[Rgb], cols: usize, mode: ColorMode) -> String {
    const RESET: &str = "\x1b[0m";
    let mut out = String::new();
    if palette.is_empty() {
        return out;
    }

    let usable = cols.saturating_sub(MARGIN);
    let per_line = (usable / (MIN_SWATCH + GAP)).clamp(1, palette.len());
    let width = (usable / per_line).saturating_sub(GAP).clamp(MIN_SWATCH, MAX_SWATCH);
    let margin = " ".repeat(MARGIN);
    let gap = " ".repeat(GAP);

    writeln!(out).unwrap();
    for line in palette.chunks(per_line) {
        for row in 0..3 {
            out.push_str(&margin);
            for (i, &color) in line.iter().enumerate() {
                if i > 0 {
                    out.push_str(&gap);
                }
                out.push_str(&mode.bg(color));
                if row == 1 {
                    let label = hex(&color);
                    let left = (width - label.len()) / 2;
                    let right = width - label.len() - left;
                    write!(
                        out,
                        "{}{}{label}{}",
                        " ".repeat(left),
                        text_color_for(color),
                        " ".repeat(right)
                    )
                    .unwrap();
                } else {
                    out.push_str(&" ".repeat(width));
                }
                out.push_str(RESET);
            }
            out.push('\n');
        }
        writeln!(out).unwrap();
    }
    out
}

// ---------------------------------------------------------------------------
// Palette card image
// ---------------------------------------------------------------------------

fn lerp(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t).round() as u8
}

/// 0.0 outside the rounded corners, 1.0 inside, with a 1px antialiased edge.
fn corner_alpha(x: u32, y: u32, w: u32, h: u32, r: f32) -> f32 {
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let cx = if px < r { r } else if px > w as f32 - r { w as f32 - r } else { return 1.0 };
    let cy = if py < r { r } else if py > h as f32 - r { h as f32 - r } else { return 1.0 };
    let dist = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
    (r - dist + 0.5).clamp(0.0, 1.0)
}

/// Saves a 1200x630 card: a smooth gradient through the palette on top,
/// solid swatches along the bottom, rounded corners.
pub fn save_palette_card(palette: &[Rgb], path: &Path) -> Result<(), Box<dyn Error>> {
    const W: u32 = 1200;
    const H: u32 = 630;
    const SWATCH_H: u32 = 140; // height of the solid swatch strip at the bottom
    const RADIUS: f32 = 32.0;

    let n = palette.len();
    if n == 0 {
        return Err("palette is empty".into());
    }

    let img = RgbaImage::from_fn(W, H, |x, y| {
        let (r, g, b) = if y >= H - SWATCH_H {
            // Bottom strip: one solid block per color
            palette[(x as usize * n / W as usize).min(n - 1)]
        } else if n == 1 {
            palette[0]
        } else {
            // Gradient: find which two colors this x sits between
            let t = x as f32 / (W - 1) as f32 * (n - 1) as f32;
            let i = (t.floor() as usize).min(n - 2);
            let f = t - i as f32;
            let f = f * f * (3.0 - 2.0 * f); // smoothstep for softer transitions
            let (c0, c1) = (palette[i], palette[i + 1]);
            (lerp(c0.0, c1.0, f), lerp(c0.1, c1.1, f), lerp(c0.2, c1.2, f))
        };

        let alpha = (corner_alpha(x, y, W, H, RADIUS) * 255.0) as u8;
        Rgba([r, g, b, alpha])
    });

    img.save(path)
        .map_err(|e| format!("could not save {}: {e}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PALETTE: [Rgb; 2] = [(255, 0, 128), (16, 32, 48)];

    #[test]
    fn hex_format() {
        assert_eq!(render(&PALETTE, Format::Hex), "#ff0080\n#102030\n");
    }

    #[test]
    fn json_format() {
        assert_eq!(
            render(&PALETTE, Format::Json),
            "[\n  { \"hex\": \"#ff0080\", \"rgb\": [255, 0, 128] },\n  \
             { \"hex\": \"#102030\", \"rgb\": [16, 32, 48] }\n]\n"
        );
    }

    #[test]
    fn css_format() {
        assert_eq!(
            render(&PALETTE, Format::Css),
            ":root {\n  --acolor-1: #ff0080;\n  --acolor-2: #102030;\n}\n"
        );
    }

    #[test]
    fn gpl_format() {
        assert_eq!(
            render(&PALETTE, Format::Gpl),
            "GIMP Palette\nName: acolor\nColumns: 2\n#\n255   0 128\tacolor-1\n 16  32  48\tacolor-2\n"
        );
    }

    /// Strips ANSI escape codes so we can check the visible layout
    fn visible(s: &str) -> String {
        let mut out = String::new();
        let mut in_escape = false;
        for ch in s.chars() {
            match (in_escape, ch) {
                (false, '\x1b') => in_escape = true,
                (true, 'm') => in_escape = false,
                (true, _) => {}
                (false, c) => out.push(c),
            }
        }
        out
    }

    #[test]
    fn swatches_use_truecolor_and_show_hex() {
        let out = render_swatches(&PALETTE, 80, ColorMode::TrueColor);
        assert!(out.contains("\x1b[48;2;255;0;128m"));
        assert!(out.contains("#ff0080"));
        assert!(out.contains("#102030"));
    }

    #[test]
    fn swatches_fall_back_to_256_colors() {
        let out = render_swatches(&PALETTE, 80, ColorMode::Ansi256);
        assert!(!out.contains("48;2;"));
        assert!(out.contains("\x1b[48;5;"));
    }

    #[test]
    fn swatches_are_three_rows_and_fit_the_width() {
        let palette = [(10, 20, 30); 5];
        let out = render_swatches(&palette, 80, ColorMode::TrueColor);
        let lines: Vec<String> = out.lines().map(visible).filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 3, "one strip of 3 rows");
        // 2 margin + 5 swatches of 14 + 4 gaps = 76 columns
        assert!(lines.iter().all(|l| l.chars().count() <= 80), "{lines:?}");
        assert_eq!(lines[1].matches("#0a141e").count(), 5);
    }

    #[test]
    fn swatches_wrap_on_narrow_terminals() {
        let palette = [(10, 20, 30); 8];
        let out = render_swatches(&palette, 40, ColorMode::TrueColor);
        let lines: Vec<String> = out.lines().map(visible).filter(|l| !l.is_empty()).collect();
        // 40 cols fits 3 swatches per strip -> 3 strips of 3 rows
        assert_eq!(lines.len(), 9, "{lines:?}");
        assert!(lines.iter().all(|l| l.chars().count() <= 40), "{lines:?}");
    }

    #[test]
    fn text_contrasts_with_background() {
        assert_eq!(text_color_for((255, 255, 255)), "\x1b[30m"); // black on white
        assert_eq!(text_color_for((250, 250, 60)), "\x1b[30m");  // black on yellow
        assert_eq!(text_color_for((0, 0, 0)), "\x1b[97m");       // white on black
        assert_eq!(text_color_for((40, 20, 120)), "\x1b[97m");   // white on indigo
    }

    #[test]
    fn ansi256_conversion() {
        assert_eq!(to_ansi256((255, 0, 0)), 196);     // pure red in the cube
        assert_eq!(to_ansi256((0, 0, 0)), 16);        // black in the cube
        assert_eq!(to_ansi256((128, 128, 128)), 244); // mid grey -> grey ramp
        assert_eq!(to_ansi256((95, 135, 175)), 67);   // exact cube entry
    }
}
