use crate::{paths, profiles::Profile};
use std::{
    ffi::OsStr,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
};

extern "C" {
    fn hydra_tag_windows(
        pid: u32,
        id: *const u16,
        name: *const u16,
        command: *const u16,
        icon: *const u16,
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

const ICONS: [&[u8]; 8] = [
    include_bytes!("../icons/profiles/0.ico"),
    include_bytes!("../icons/profiles/1.ico"),
    include_bytes!("../icons/profiles/2.ico"),
    include_bytes!("../icons/profiles/3.ico"),
    include_bytes!("../icons/profiles/4.ico"),
    include_bytes!("../icons/profiles/5.ico"),
    include_bytes!("../icons/profiles/6.ico"),
    include_bytes!("../icons/profiles/7.ico"),
];

fn icon(profile: &Profile) -> std::io::Result<PathBuf> {
    let color = profile.color % ICONS.len();
    let dir = paths::storage_root().join("icons");
    let file = dir.join(format!("profile-v1-{color}.ico"));
    if !file.exists() {
        std::fs::create_dir_all(dir)?;
        std::fs::write(&file, ICONS[color])?;
    }
    Ok(file)
}

pub fn apply(profile: &Profile, pid: u32) -> bool {
    if profile.is_default {
        return true;
    }
    let (Ok(icon), Ok(exe)) = (icon(profile), std::env::current_exe()) else {
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
        ) != 0
    }
}

pub fn shortcut(profile: &Profile, path: &Path) -> Result<(), String> {
    if profile.is_default {
        return Ok(());
    }
    let icon = icon(profile).map_err(|e| format!("无法创建账号图标：{e}"))?;
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
