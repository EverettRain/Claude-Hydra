use crate::{paths, profiles::Profile};
use std::{
    ffi::OsStr,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
};

extern "C" {
    fn hydra_draw_badge(label: *const u16, rgb: u32, base: *const u16, size: i32, pixels: *mut u8) -> bool;
    fn hydra_tag_windows(
        pid: u32,
        id: *const u16,
        name: *const u16,
        command: *const u16,
        icon: *const u16,
        badge: *const u16,
    ) -> u32;
    fn hydra_tag_shortcut(path: *const u16, id: *const u16, icon: *const u16) -> i32;
}

fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

fn identity(profile: &Profile) -> Vec<u16> {
    // FNV-1a is deterministic across builds. Include the data directory for custom roots.
    let key = crate::profiles::path_key(&profile.path);
    let hash = key.bytes().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    });
    wide(format!("ClaudeHydra.Profile.{hash:016X}"))
}

const COLORS: [u32; 8] = [0x4285b4, 0x4c9660, 0x9660b4, 0xbe506e, 0x3c9696, 0x5a6ec8, 0xc89632, 0x505664];

fn cached_icon(name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let dir = paths::storage_root().join("icons");
    let file = dir.join(name);
    if !file.exists() {
        std::fs::create_dir_all(dir)?;
        std::fs::write(&file, bytes)?;
    }
    Ok(file)
}

fn icon(profile: &Profile, composite: bool) -> std::io::Result<PathBuf> {
    let key = profile.badge.chars().map(|c| format!("{:x}", c as u32)).collect::<Vec<_>>().join("-");
    let color = profile.color % COLORS.len();
    let name = format!("account-v3-{color}-{key}-{composite}.ico");
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = paths::storage_root().join("icons").join(&name);
    if path.is_file() { return Ok(path); }
    let base = if composite {
        Some(wide(cached_icon("claude-base-v3.png", include_bytes!("../../src/assets/claude-default.png"))?))
    } else { None };
    let label = wide(&profile.badge);
    let sizes = [16usize, 32, 48, 64, 128];
    let mut ico = vec![0, 0, 1, 0, sizes.len() as u8, 0];
    let mut frames = Vec::new();
    for size in sizes {
        let mut pixels = vec![0u8; size * size * 4];
        let ok = unsafe { hydra_draw_badge(label.as_ptr(), COLORS[color],
            base.as_ref().map_or(std::ptr::null(), |p| p.as_ptr()), size as i32, pixels.as_mut_ptr()) };
        if !ok { return Err(std::io::Error::other("无法绘制账号标记")); }
        // Each ICO entry contains a bottom-up BGRA bitmap followed by its AND mask.
        let mut frame = vec![0u8; 40];
        frame[..4].copy_from_slice(&40u32.to_le_bytes());
        frame[4..8].copy_from_slice(&(size as u32).to_le_bytes());
        frame[8..12].copy_from_slice(&(2 * size as u32).to_le_bytes());
        frame[12..16].copy_from_slice(&[1, 0, 32, 0]);
        frame.extend(pixels);
        frame.resize(frame.len() + size.div_ceil(32) * 4 * size, 0);
        ico.extend([size as u8, size as u8, 0, 0, 1, 0, 32, 0]);
        ico.extend((frame.len() as u32).to_le_bytes());
        ico.extend(((6 + 16 * sizes.len() + frames.len()) as u32).to_le_bytes());
        frames.extend(frame);
    }
    ico.extend(frames);
    cached_icon(&name, &ico)
}

pub fn apply(profile: &Profile, pid: u32) -> bool {
    if profile.is_default {
        return true;
    }
    let Ok(badge) = icon(profile, false) else { return false; };
    let (Ok(icon), Ok(exe)) = (
        cached_icon(&format!("base-{}", badge.file_name().unwrap().to_string_lossy()), include_bytes!("../icons/profiles/base.ico")),
        std::env::current_exe(),
    ) else {
        return false;
    };
    let command = wide(format!(r#""{}" --launch "{}""#, exe.display(), profile.id));
    unsafe {
        hydra_tag_windows(
            pid,
            identity(profile).as_ptr(),
            wide(&profile.name).as_ptr(),
            command.as_ptr(),
            wide(icon).as_ptr(),
            wide(badge).as_ptr(),
        ) != 0
    }
}

pub fn shortcut(profile: &Profile, path: &Path) -> Result<(), String> {
    if profile.is_default {
        return Ok(());
    }
    let icon = icon(profile, true).map_err(|e| format!("无法创建账号图标：{e}"))?;
    let hr = unsafe {
        hydra_tag_shortcut(
            wide(path).as_ptr(),
            identity(profile).as_ptr(),
            wide(icon).as_ptr(),
        )
    };
    if hr < 0 {
        return Err(format!("无法设置快捷方式的任务栏标识（{hr:#x}）。"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rasterizes_distinct_letters_at_taskbar_size() {
        let draw = |text| {
            let mut pixels = [0u8; 32 * 32 * 4];
            assert!(unsafe { hydra_draw_badge(wide(text).as_ptr(), COLORS[0], std::ptr::null(), 32, pixels.as_mut_ptr()) });
            assert!(pixels.chunks(4).any(|p| p[3] > 0));
            pixels
        };
        assert_ne!(draw("A"), draw("B"));
        assert_ne!(draw("上"), draw("北"));
    }
}
