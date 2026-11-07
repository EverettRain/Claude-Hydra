//! Storage directory resolution and lightweight logging for Claude-Hydra.

use std::path::PathBuf;

pub const DEFAULT_ID: &str = "Default";

fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// Root directory for Claude-Hydra configuration and isolated account data.
/// Can be overridden via `CLAUDE_HYDRA_ROOT`.
pub fn storage_root() -> PathBuf {
    if let Some(p) = env_dir("CLAUDE_HYDRA_ROOT") {
        return p;
    }
    #[cfg(windows)]
    {
        let d = PathBuf::from(r"D:\");
        if d.is_dir() {
            return d.join("ClaudeHydra");
        }
        env_dir("LOCALAPPDATA")
            .unwrap_or_else(|| PathBuf::from("."))
            .join("ClaudeHydra")
    }
    #[cfg(target_os = "macos")]
    {
        env_dir("CLAUDE_HYDRA_SUPPORT_DIR").unwrap_or_else(|| {
            env_dir("HOME")
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join("Library/Application Support/ClaudeHydra")
        })
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        env_dir("XDG_CONFIG_HOME")
            .or_else(|| env_dir("HOME").map(|h| h.join(".config")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("claude-hydra")
    }
}

pub fn profiles_dir() -> PathBuf {
    storage_root().join("profiles")
}

pub fn logs_dir() -> PathBuf {
    storage_root().join("logs")
}

pub fn settings_path() -> PathBuf {
    storage_root().join("settings.json")
}

pub fn extra_profile_dir(id: &str) -> PathBuf {
    profiles_dir().join(id)
}

pub fn desktop_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        env_dir("USERPROFILE").map(|p| p.join("Desktop"))
    }
    #[cfg(not(windows))]
    {
        env_dir("HOME").map(|p| p.join("Desktop"))
    }
}

pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn now_secs() -> u64 {
    now_millis() / 1000
}

pub fn append_log(file: &str, message: &str) {
    use std::io::Write;
    let dir = logs_dir();
    let path = dir.join(file);
    let _ = std::fs::create_dir_all(&dir);
    if std::fs::metadata(&path).map(|m| m.len() > 100 * 1024).unwrap_or(false) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{}] {}", now_secs(), message);
    }
}

pub fn read_log_tail(file: &str, max_bytes: usize) -> String {
    let path = logs_dir().join(file);
    let Ok(bytes) = std::fs::read(&path) else {
        return String::new();
    };
    let start = bytes.len().saturating_sub(max_bytes);
    String::from_utf8_lossy(&bytes[start..]).to_string()
}
