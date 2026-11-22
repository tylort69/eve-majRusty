//! The app icon for Linux desktops: installs assets/logo.svg as the `eve-maj-preview` icon of the user's hicolor
//! theme, which is what the `Icon=` key of every .desktop file the app writes (protocol handler, autostart) names.

use std::path::PathBuf;

/// Icon name for `Icon=` keys.
pub const ICON_NAME: &str = "eve-maj-preview";

const LOGO_SVG: &str = include_str!("../../../assets/logo.svg");
const ICON_ICO: &[u8] = include_bytes!("../../../assets/icon.ico");

/// The `size`x`size` image of assets/icon.ico (one of its 32-bit BMP entries: 16, 24, 32, 48 or 64) as top-down
/// straight-alpha RGBA rows. None for a size the file doesn't have.
pub fn ico_rgba(size: u32) -> Option<Vec<u8>> {
    let d = ICON_ICO;
    let u16_at = |o: usize| -> Option<u16> { Some(u16::from_le_bytes(d.get(o..o + 2)?.try_into().ok()?)) };
    let u32_at = |o: usize| -> Option<u32> { Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?)) };
    let entry = (0..u16_at(4)? as usize).map(|i| 6 + 16 * i).find(|&e| d.get(e).copied() == Some(size as u8) && u16_at(e + 6) == Some(32))?;
    let off = u32_at(entry + 12)? as usize;
    let header_len = u32_at(off)? as usize;
    let (width, height) = (u32_at(off + 4)? as usize, u32_at(off + 8)? as usize / 2); // height counts the AND mask too
    if width != size as usize || height != size as usize {
        return None;
    }
    let pixels = d.get(off + header_len..off + header_len + width * height * 4)?;
    let mut rgba = Vec::with_capacity(width * height * 4);
    for row in pixels.chunks_exact(width * 4).rev() {
        for px in row.chunks_exact(4) {
            rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]); // BMP rows are bottom-up BGRA
        }
    }
    Some(rgba)
}

fn icon_path() -> Option<PathBuf> {
    let data = match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(x) => PathBuf::from(x),
        None => PathBuf::from(std::env::var_os("HOME").filter(|v| !v.is_empty())?).join(".local/share"),
    };
    Some(data.join("icons/hicolor/scalable/apps").join(format!("{ICON_NAME}.svg")))
}

/// Writes the icon if it's missing or out of date (a newer build with a changed logo). Errors are returned for the
/// caller to log; a missing icon only costs the desktop entry its picture.
pub fn install() -> std::io::Result<()> {
    let Some(path) = icon_path() else { return Ok(()) };
    if std::fs::read_to_string(&path).ok().as_deref() == Some(LOGO_SVG) {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, LOGO_SVG)
}

#[cfg(test)]
mod tests {
    #[test]
    fn bundled_logo_is_svg() {
        assert!(super::LOGO_SVG.trim_start().starts_with("<svg"));
        assert!(super::LOGO_SVG.contains("viewBox=\"0 0 256 256\""));
    }

    #[test]
    fn bundled_ico_decodes() {
        for size in [16, 24, 32, 48, 64] {
            let rgba = super::ico_rgba(size).unwrap_or_else(|| panic!("{size}px entry decodes"));
            assert_eq!(rgba.len(), (size * size * 4) as usize);
            assert!(rgba.chunks_exact(4).any(|p| p[3] == 255), "{size}px has opaque pixels");
            assert!(rgba.chunks_exact(4).any(|p| p[3] == 0), "{size}px has transparent corners");
        }
        assert!(super::ico_rgba(20).is_none());
    }
}
