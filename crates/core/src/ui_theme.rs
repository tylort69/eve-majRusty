//! The settings window's Material 3 color scheme, for things the app draws itself (the Client List and History
//! Panel), so they match the window and follow the theme picked in Preferences.
//!
//! A port of the theme engine in `crates/config/ui/config_dialog.js` (`UI_THEME_PRESETS`, `UI_THEME_ROLES`,
//! `ThemeColor`, `buildUiThemeScheme`): each seed becomes an OKLCH hue and chroma, and each role is that hue/chroma at
//! an M3 dark-scheme tone (CIE L*), with the chroma reduced when the tone can't hold it inside sRGB. The default theme
//! isn't generated; it's the hand-tuned `:root` values in `config_dialog.css`. Keep the two in step.

use crate::config::UiTheme;

/// The roles the app's own panels use, as 0xRRGGBB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiPalette {
    pub primary: u32,
    pub on_primary: u32,
    pub primary_container: u32,
    pub on_primary_container: u32,
    pub secondary: u32,
    pub secondary_container: u32,
    pub on_secondary_container: u32,
    pub tertiary: u32,
    pub surface: u32,
    pub surface_container_lowest: u32,
    pub surface_container_low: u32,
    pub surface_container: u32,
    pub surface_container_high: u32,
    pub surface_container_highest: u32,
    pub on_surface: u32,
    pub on_surface_variant: u32,
    pub on_surface_muted: u32,
    pub outline: u32,
    pub outline_variant: u32,
}

/// Fixed roles the themes don't change (as in the CSS): error and success.
pub const ERROR: u32 = 0xFFB4AB;
pub const ERROR_CONTAINER: u32 = 0x93000A;
pub const SUCCESS: u32 = 0x8FD5A6;

impl Default for UiPalette {
    /// The stock theme: config_dialog.css's `:root`.
    fn default() -> Self {
        UiPalette {
            primary: 0xF5BD62,
            on_primary: 0x432C00,
            primary_container: 0x604100,
            on_primary_container: 0xFFDDB0,
            secondary: 0xDCC3A1,
            secondary_container: 0x56442A,
            on_secondary_container: 0xF9DFBB,
            tertiary: 0xB5CEA3,
            surface: 0x17130E,
            surface_container_lowest: 0x120E09,
            surface_container_low: 0x1F1B16,
            surface_container: 0x231F1A,
            surface_container_high: 0x2E2924,
            surface_container_highest: 0x39342E,
            on_surface: 0xEBE1D9,
            on_surface_variant: 0xD4C4B4,
            on_surface_muted: 0xA99A8B,
            outline: 0x9C8E80,
            outline_variant: 0x504539,
        }
    }
}

/// Faction seeds, as `UI_THEME_PRESETS`: (id, primary, secondary, tertiary, neutral).
const PRESETS: [(&str, u32, u32, u32, u32); 8] = [
    ("amarr", 0xE3B53E, 0xB08A4A, 0xD0434C, 0x3C2216),
    ("caldari", 0x4DA3DE, 0x7E90A6, 0x3CC4CC, 0x1B2632),
    ("gallente", 0x38BC8E, 0x6EA2B2, 0x8FB4EC, 0x13271F),
    ("minmatar", 0xD4683A, 0x9C6C4C, 0xCBB27A, 0x2B1C16),
    ("guristas", 0xEDD21C, 0x8E8C76, 0xE3572C, 0x1A1A17),
    ("serpentis", 0xA977E3, 0x8C7CA3, 0xB4D63C, 0x1F1829),
    ("sansha", 0x62D46A, 0x6F9075, 0x45C8B6, 0x111D13),
    ("triglavian", 0xF04848, 0xA55C5C, 0xF2A33E, 0x241212),
];

impl UiPalette {
    /// The palette for a saved theme: a faction preset, a custom theme, or the stock one (also for an unknown id or
    /// a custom theme without a usable primary color, like the settings window).
    pub fn for_theme(theme: &UiTheme) -> UiPalette {
        let seeds = match theme.id.as_str() {
            "" | "default" => None,
            "custom" => parse_color(&theme.primary).map(|p| [Some(p), parse_color(&theme.secondary), parse_color(&theme.tertiary), parse_color(&theme.neutral)]),
            id => PRESETS.iter().find(|p| p.0 == id).map(|&(_, p, s, t, n)| [Some(p), Some(s), Some(t), Some(n)]),
        };
        match seeds {
            Some([Some(p), s, t, n]) => build(p, s, t, n),
            _ => UiPalette::default(),
        }
    }
}

/// `buildUiThemeScheme`: optional seeds left out are derived from primary, M3-style.
fn build(primary: u32, secondary: Option<u32>, tertiary: Option<u32>, neutral: Option<u32>) -> UiPalette {
    let p = to_oklch(primary);
    let s = secondary.map(to_oklch).unwrap_or((p.0, p.1 * 0.35));
    let t = tertiary.map(to_oklch).unwrap_or(((p.0 + 60.0) % 360.0, p.1 * 0.6));
    let n_seed = neutral.map(to_oklch);
    // Capped so a strongly colored background seed still leaves readable surfaces.
    let n = n_seed.map(|n| (n.0, n.1.min(0.045))).unwrap_or((p.0, (p.1 * 0.1).min(0.016)));
    let v = n_seed.map(|n| (n.0, (n.1 * 1.4).min(0.06))).unwrap_or((p.0, (p.1 * 0.15).min(0.026)));
    let at = |(h, c): (f64, f64), t: f64| tone(h, c, t);
    UiPalette {
        primary: at(p, 80.0),
        on_primary: at(p, 20.0),
        primary_container: at(p, 30.0),
        on_primary_container: at(p, 90.0),
        secondary: at(s, 80.0),
        secondary_container: at(s, 30.0),
        on_secondary_container: at(s, 90.0),
        tertiary: at(t, 80.0),
        surface: at(n, 6.0),
        surface_container_lowest: at(n, 4.0),
        surface_container_low: at(n, 10.0),
        surface_container: at(n, 12.0),
        surface_container_high: at(n, 17.0),
        surface_container_highest: at(n, 22.0),
        on_surface: at(n, 90.0),
        on_surface_variant: at(v, 80.0),
        on_surface_muted: at(v, 65.0),
        outline: at(v, 60.0),
        outline_variant: at(v, 30.0),
    }
}

/// `ThemeColor.parse` without the alpha diagnostics: "#RGB", "#RRGGBB" (# optional), "rgb(r, g, b)" or "r, g, b" / "r g b".
pub fn parse_color(text: &str) -> Option<u32> {
    let s = text.trim();
    let hex = s.strip_prefix('#').unwrap_or(s);
    if (hex.len() == 3 || hex.len() == 6) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        let full: String = if hex.len() == 3 { hex.chars().flat_map(|c| [c, c]).collect() } else { hex.to_owned() };
        return u32::from_str_radix(&full, 16).ok();
    }
    let lower = s.to_ascii_lowercase();
    let inner = match lower.strip_prefix("rgb") {
        Some(rest) => rest.trim_start().strip_prefix('(')?.trim_end().strip_suffix(')')?,
        None => lower.as_str(),
    };
    let parts: Vec<&str> = inner.split(|c: char| c == ',' || c.is_whitespace()).filter(|p| !p.is_empty()).collect();
    if parts.len() != 3 {
        return None;
    }
    let mut rgb = 0u32;
    for p in parts {
        if p.len() > 3 || !p.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let v: u32 = p.parse().ok()?;
        if v > 255 {
            return None;
        }
        rgb = (rgb << 8) | v;
    }
    Some(rgb)
}

fn to_linear(c: f64) -> f64 {
    let c = c / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn from_linear(c: f64) -> f64 {
    (if c <= 0.0031308 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }) * 255.0
}

/// (hue in degrees, chroma) of an sRGB color in OKLCH.
fn to_oklch(rgb: u32) -> (f64, f64) {
    let r = to_linear(((rgb >> 16) & 0xFF) as f64);
    let g = to_linear(((rgb >> 8) & 0xFF) as f64);
    let b = to_linear((rgb & 0xFF) as f64);
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    let a = 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s;
    let bb = 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s;
    (((bb.atan2(a)).to_degrees() + 360.0) % 360.0, a.hypot(bb))
}

/// Linear-light sRGB of an OKLCH color (may fall outside 0..1 when out of gamut).
fn linear_from_oklch(l: f64, c: f64, h: f64) -> [f64; 3] {
    let (a, b) = (c * h.to_radians().cos(), c * h.to_radians().sin());
    let l_ = (l + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let m_ = (l - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let s_ = (l - 0.0894841775 * a - 1.2914855480 * b).powi(3);
    [
        4.0767416621 * l_ - 3.3077115913 * m_ + 0.2309699292 * s_,
        -1.2684380046 * l_ + 2.6097574011 * m_ - 0.3413193965 * s_,
        -0.0041960863 * l_ - 0.7034186147 * m_ + 1.7076147010 * s_,
    ]
}

fn luminance([r, g, b]: [f64; 3]) -> f64 {
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn tone_to_y(t: f64) -> f64 {
    let f = (t + 16.0) / 116.0;
    if f > 6.0 / 29.0 { f * f * f } else { t / 903.2963 }
}

fn in_gamut(rgb: [f64; 3]) -> bool {
    rgb.iter().all(|&v| (-1e-6..=1.0 + 1e-6).contains(&v))
}

/// The OKLab lightness whose luminance matches `y` at this hue/chroma.
fn lightness_for(c: f64, h: f64, y: f64) -> f64 {
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..26 {
        let mid = (lo + hi) / 2.0;
        if luminance(linear_from_oklch(mid, c, h)) < y {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) / 2.0
}

/// `ThemeColor.tone`: the color at hue `h`, chroma `chroma` and tone `t` (0-100), as 0xRRGGBB.
fn tone(h: f64, chroma: f64, t: f64) -> u32 {
    let rgb = if t <= 0.0 {
        [0.0; 3]
    } else if t >= 100.0 {
        [1.0; 3]
    } else {
        let y = tone_to_y(t);
        let rgb = linear_from_oklch(lightness_for(chroma, h, y), chroma, h);
        if in_gamut(rgb) {
            rgb
        } else {
            // Highest chroma this tone can hold at this hue.
            let (mut lo, mut hi) = (0.0, chroma);
            for _ in 0..18 {
                let mid = (lo + hi) / 2.0;
                if in_gamut(linear_from_oklch(lightness_for(mid, h, y), mid, h)) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            linear_from_oklch(lightness_for(lo, h, y), lo, h)
        }
    };
    // JS Math.round rounds .5 up; f64::round rounds it away from zero, the same for these non-negative values.
    let ch = |v: f64| from_linear(v.clamp(0.0, 1.0)).round().clamp(0.0, 255.0) as u32;
    (ch(rgb[0]) << 16) | (ch(rgb[1]) << 8) | ch(rgb[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme(id: &str) -> UiTheme {
        UiTheme { id: id.to_owned(), ..UiTheme::default() }
    }

    #[test]
    fn default_and_unknown_themes_use_the_stock_palette() {
        assert_eq!(UiPalette::for_theme(&theme("")), UiPalette::default());
        assert_eq!(UiPalette::for_theme(&theme("default")), UiPalette::default());
        assert_eq!(UiPalette::for_theme(&theme("no-such-faction")), UiPalette::default());
        // A custom theme needs a usable primary color, as in the settings window.
        assert_eq!(UiPalette::for_theme(&UiTheme { id: "custom".into(), primary: "#12345".into(), ..UiTheme::default() }), UiPalette::default());
    }

    #[test]
    fn tones_have_the_requested_lightness_and_stay_in_gamut() {
        // CIE L* of the result is within a step of the tone asked for, for saturated and grey seeds alike.
        for &(_, p, ..) in &PRESETS {
            let (h, c) = to_oklch(p);
            for t in [4.0, 10.0, 20.0, 30.0, 65.0, 80.0, 90.0] {
                let rgb = tone(h, c, t);
                let lin = [(rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF].map(|v| to_linear(v as f64));
                let y = luminance(lin);
                let lstar = if y > 216.0 / 24389.0 { 116.0 * y.cbrt() - 16.0 } else { y * 903.2963 };
                assert!((lstar - t).abs() < 1.5, "tone {t} came out as L* {lstar:.2} ({rgb:06X})");
            }
        }
    }

    #[test]
    fn presets_keep_their_hue() {
        let caldari = UiPalette::for_theme(&theme("caldari"));
        let gallente = UiPalette::for_theme(&theme("gallente"));
        let blue = |c: u32| (c & 0xFF) > ((c >> 16) & 0xFF);
        assert!(blue(caldari.primary) && blue(caldari.surface_container_low));
        assert!(((gallente.primary >> 8) & 0xFF) > ((gallente.primary >> 16) & 0xFF), "Gallente primary is green");
        assert_ne!(caldari, gallente);
        // Text stays light and surfaces dark.
        for p in [caldari, gallente] {
            assert!(p.on_surface & 0xFF > 0xC8 && p.surface_container_low & 0xFF < 0x30);
        }
    }

    #[test]
    fn custom_theme_derives_missing_seeds() {
        let only_primary = UiPalette::for_theme(&UiTheme { id: "custom".into(), primary: "rgb(77, 163, 222)".into(), ..UiTheme::default() });
        let with_hex = UiPalette::for_theme(&UiTheme { id: "custom".into(), primary: "4DA3DE".into(), ..UiTheme::default() });
        assert_eq!(only_primary, with_hex);
        assert_ne!(only_primary.tertiary, only_primary.primary);
    }

    #[test]
    fn matches_the_settings_window_engine() {
        // Values computed by config_dialog.js's buildUiThemeScheme: primary, secondary-container,
        // surface-container-low, on-surface, on-surface-muted, outline-variant.
        let roles = |p: UiPalette| [p.primary, p.secondary_container, p.surface_container_low, p.on_surface, p.on_surface_muted, p.outline_variant];
        assert_eq!(roles(UiPalette::for_theme(&theme("caldari"))), [0x8BCEFF, 0x38485B, 0x121C28, 0xD5E4F4, 0x8DA0B5, 0x37485B]);
        assert_eq!(roles(UiPalette::for_theme(&theme("serpentis"))), [0xD7BAFF, 0x4F4063, 0x1F1829, 0xE8DFF7, 0xA599B8, 0x4D425D]);
        let custom = UiTheme { id: "custom".into(), primary: "#4DA3DE".into(), ..UiTheme::default() };
        assert_eq!(roles(UiPalette::for_theme(&custom)), [0x8BCEFF, 0x32495B, 0x171C21, 0xDCE4EA, 0x949FA8, 0x3E4850]);
    }

    #[test]
    fn parses_hex_and_rgb_like_the_settings_window() {
        assert_eq!(parse_color("#f5bd62"), Some(0xF5BD62));
        assert_eq!(parse_color("abc"), Some(0xAABBCC));
        assert_eq!(parse_color("rgb(1, 2, 3)"), Some(0x010203));
        assert_eq!(parse_color("10 20 30"), Some(0x0A141E));
        assert_eq!(parse_color("#11223344"), None);
        assert_eq!(parse_color("256, 0, 0"), None);
        assert_eq!(parse_color(""), None);
    }
}
