//! The app icon for Linux desktops: installs assets/logo.svg as the `eve-maj-preview` icon of the user's hicolor
//! theme, which is what the `Icon=` key of every .desktop file the app writes (protocol handler, autostart) names.

use std::path::PathBuf;

/// Icon name for `Icon=` keys.
pub const ICON_NAME: &str = "eve-maj-preview";

const LOGO_SVG: &str = include_str!("../../../assets/logo.svg");

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
}
