//! The app icon chosen in Settings.
//!
//! Applied while Hydra runs: the main window and tray icons, the in-app logo (the UI reads
//! `current`), and on macOS the Dock icon. The icon inside the bundle / executable (Finder,
//! Launchpad, Explorer, and the Dock while Hydra is not running) is fixed at build time and is
//! the default variant; changing the signed bundle at run time would break its signature and
//! be undone by every update.
use tauri::image::Image;
use tauri::{AppHandle, Manager};

pub const DEFAULT: &str = "maid-sky";

/// Selectable icons: id, display name, 256 px full-bleed artwork (window, tray, in-app logo).
const ICONS: [(&str, &str, &[u8]); 4] = [
    ("maid-sky", "女仆 · 天空蓝", include_bytes!("../../src/assets/app-icons/maid-sky.png")),
    ("maid-teal", "女仆 · 青绿", include_bytes!("../../src/assets/app-icons/maid-teal.png")),
    ("maid-indigo", "女仆 · 靛蓝", include_bytes!("../../src/assets/app-icons/maid-indigo.png")),
    ("hydra", "原版 Hydra", include_bytes!("../../src/assets/app-icons/hydra.png")),
];

/// The Dock variants follow Apple's icon template (margin and shadow), like the bundled icon.
#[cfg(target_os = "macos")]
const DOCK: [(&str, &[u8]); 4] = [
    ("maid-sky", include_bytes!("../icons/dock/maid-sky.png")),
    ("maid-teal", include_bytes!("../icons/dock/maid-teal.png")),
    ("maid-indigo", include_bytes!("../icons/dock/maid-indigo.png")),
    ("hydra", include_bytes!("../icons/dock/hydra.png")),
];

pub fn choices() -> Vec<(String, String)> {
    ICONS.iter().map(|(id, name, _)| (id.to_string(), name.to_string())).collect()
}

fn valid(id: &str) -> bool { ICONS.iter().any(|(i, _, _)| *i == id) }

/// The chosen icon; an unknown or unset value is the default.
pub fn current() -> String {
    Some(crate::settings::load().app_icon).filter(|id| valid(id)).unwrap_or_else(|| DEFAULT.to_string())
}

fn image(id: &str) -> Option<Image<'static>> {
    let (_, _, png) = ICONS.iter().find(|(i, _, _)| *i == id)?;
    Image::from_bytes(png).ok()
}

/// Sets a window's icon to the chosen one (windows are created on demand).
pub fn apply_window(w: &tauri::WebviewWindow) {
    if let Some(img) = image(&current()) { let _ = w.set_icon(img); }
}

/// Shows icon `id` everywhere it can change while Hydra runs.
pub fn apply(app: &AppHandle, id: &str) {
    if let Some(img) = image(id) {
        for w in app.webview_windows().values() { let _ = w.set_icon(img.clone()); }
        if let Some(tray) = app.tray_by_id(crate::TRAY_ID) { let _ = tray.set_icon(Some(img)); }
    }
    #[cfg(target_os = "macos")]
    if let Some((_, png)) = DOCK.iter().find(|(i, _)| *i == id) {
        let png: &'static [u8] = png;
        let _ = app.run_on_main_thread(move || {
            if let Err(e) = set_dock_icon(png) { crate::paths::append_log("hydra-error.log", &format!("dock icon: {e}")); }
        });
    }
}

/// Saves the choice and applies it.
pub fn set(app: &AppHandle, id: &str) -> Result<(), String> {
    if !valid(id) { return Err("没有这个图标。".into()); }
    crate::settings::update(|s| s.app_icon = if id == DEFAULT { String::new() } else { id.to_string() })?;
    apply(app, id);
    Ok(())
}

#[cfg(target_os = "macos")]
fn set_dock_icon(png: &[u8]) -> Result<(), String> {
    use objc2::{AllocAnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;
    let mtm = MainThreadMarker::new().ok_or("不在主线程上")?;
    let data = NSData::with_bytes(png);
    let image = NSImage::initWithData(NSImage::alloc(), &data).ok_or("图标数据无效")?;
    let app = NSApplication::sharedApplication(mtm);
    // SAFETY: called on the main thread with a valid image, as AppKit requires.
    unsafe { app.setApplicationIconImage(Some(&image)) };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_decodes_as_square_artwork() {
        // Arrange
        let icons: Vec<&str> = ICONS.iter().map(|(id, _, _)| *id).collect();

        // Act
        let images: Vec<Image<'static>> = icons.iter().filter_map(|id| image(id)).collect();

        // Assert
        assert_eq!(images.len(), ICONS.len());
        for img in images { assert_eq!(img.width(), img.height()); }
    }

    #[test]
    fn default_is_a_listed_icon() {
        // Arrange
        let ids: Vec<String> = choices().into_iter().map(|(id, _)| id).collect();

        // Act
        let listed = ids.iter().any(|id| id == DEFAULT);

        // Assert
        assert!(listed);
        assert!(!valid("unknown"));
    }
}
