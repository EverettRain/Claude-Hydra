//! Claude's third-party inference mode ("3P": an API gateway, Bedrock, Vertex, ...).
//!
//! Claude keeps the 3P setup and the user's 1P/3P choice in one machine-wide folder
//! (`%LOCALAPPDATA%\Claude-3p` on Windows, `~/Library/Application Support/Claude-3p` on macOS),
//! not in the profile passed with `--user-data-dir`. When 3P is set up and the stored choice is
//! not "1p", Claude ignores `--user-data-dir` and switches every launch to that shared folder;
//! an extra account then just hands over to the 3P instance and exits. Worse, a fresh profile
//! has no sign-in cookie, and Claude takes that as the first-party choice being stale: it drops
//! the stored "1p" for the whole machine before switching. Launching with Claude's own
//! `--claude-first-party-sign-in` switch keeps the choice (Claude logs `first_party_pick:
//! 'own_launch'`), so the account stays in its own folder.
use std::path::PathBuf;

/// Claude's one-shot switch that keeps the stored first-party choice for this launch.
pub const KEEP_FIRST_PARTY_SWITCH: &str = "--claude-first-party-sign-in";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// No 3P setup: profiles are used as given.
    NotConfigured,
    /// 3P is set up but the first-party choice is in effect.
    FirstParty,
    /// 3P is in effect: Claude will run every profile from its shared 3P folder.
    ThirdParty,
}

fn data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    return std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Claude-3p"));
    #[cfg(target_os = "macos")]
    return std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/Claude-3p"));
    #[cfg(not(any(windows, target_os = "macos")))]
    return std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/Claude-3p"));
}

fn read_json(path: PathBuf) -> Option<serde_json::Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn text(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key)?.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// Managed (MDM / Group Policy) value under `Software\Policies\Claude`, machine policy first.
fn policy(name: &str) -> Option<String> {
    #[cfg(windows)]
    return [true, false]
        .into_iter()
        .find_map(|machine| crate::sys::reg_string(machine, "Software\\Policies\\Claude", name))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    #[cfg(not(windows))]
    {
        let _ = name;
        None
    }
}

/// The inference setup applied from Claude's local config library, if any.
fn applied_config(dir: &std::path::Path) -> Option<serde_json::Value> {
    let lib = dir.join("configLibrary");
    let id = text(&read_json(lib.join("_meta.json"))?, "appliedId")?;
    if id.len() != 36 || !id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return None;
    }
    read_json(lib.join(format!("{id}.json")))
}

pub fn mode() -> Mode {
    let Some(dir) = data_dir() else { return Mode::NotConfigured };
    let applied = applied_config(&dir);
    let configured = policy("inferenceProvider").is_some()
        || applied.as_ref().and_then(|c| text(c, "inferenceProvider")).is_some();
    if !configured {
        return Mode::NotConfigured;
    }
    let forced = policy("deploymentMode").or_else(|| applied.as_ref().and_then(|c| text(c, "deploymentMode")));
    let chosen = forced.or_else(|| read_json(dir.join("claude_desktop_config.json")).and_then(|c| text(&c, "deploymentMode")));
    if chosen.as_deref() == Some("1p") { Mode::FirstParty } else { Mode::ThirdParty }
}
