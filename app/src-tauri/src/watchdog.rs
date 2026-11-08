//! Watchdog for accounts configured with "keep running".
//! Automatically reopens accounts that exited due to updates or session shutdown,
//! while ignoring accounts closed intentionally by the user.

use crate::install::Install;
use crate::profiles::{self, Profile};
use crate::{launcher, paths, settings};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

const USER_QUIT: [&str; 2] = ["beforeQuit: handler fired", "willQuit: handler fired"];
const FORCED: [&str; 4] = [
    "Windows session ending",
    "going down for update",
    "beforeQuitForUpdate",
    "[stealth-update]",
];
const COOLDOWN_SECS: u64 = 10 * 60;

static LAST_REOPENED: LazyLock<Mutex<HashMap<String, u64>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, PartialEq)]
pub enum ExitKind {
    User,
    Forced,
    Unknown,
}

pub fn exit_kind(tail: &[&str]) -> ExitKind {
    for line in tail.iter().rev() {
        if USER_QUIT.iter().any(|m| line.contains(m)) {
            return ExitKind::User;
        }
        if FORCED.iter().any(|m| line.contains(m)) {
            return ExitKind::Forced;
        }
    }
    ExitKind::Unknown
}

fn log_tail(install: &Install, p: &Profile) -> String {
    let Ok(bytes) = std::fs::read(install.data_dir(&PathBuf::from(&p.path)).join("logs").join("main.log")) else {
        return String::new();
    };
    let start = bytes.len().saturating_sub(16 * 1024);
    String::from_utf8_lossy(&bytes[start..]).to_string()
}

pub fn any_kept() -> bool {
    settings::load().accounts.values().any(|a| a.keep_running)
}

pub fn run_once(install: &Install) -> Vec<String> {
    if !any_kept() {
        return Vec::new();
    }
    let now = paths::now_secs();
    let mut done = Vec::new();
    for p in profiles::list(install, true) {
        if !p.keep_running || p.pid.is_some() {
            continue;
        }
        let tail = log_tail(install, &p);
        let lines: Vec<&str> = tail.lines().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect();
        if exit_kind(&lines) != ExitKind::Forced {
            continue;
        }
        let recent = LAST_REOPENED
            .lock()
            .ok()
            .and_then(|m| m.get(&p.id).copied())
            .map(|t| now.saturating_sub(t) < COOLDOWN_SECS)
            .unwrap_or(false);
        if recent {
            continue;
        }
        match launcher::launch(install, &p) {
            Ok(_) => {
                if let Ok(mut m) = LAST_REOPENED.lock() {
                    m.insert(p.id.clone(), now);
                }
                paths::append_log("watchdog.log", &format!("Reopened '{}' after forced exit.", p.name));
                done.push(p.name.clone());
                std::thread::sleep(std::time::Duration::from_secs(3));
            }
            Err(e) => paths::append_log("watchdog.log", &format!("Could not reopen '{}': {e}", p.name)),
        }
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_marker_wins() {
        assert_eq!(exit_kind(&["going down for update", "beforeQuit: handler fired"]), ExitKind::User);
        assert_eq!(exit_kind(&["beforeQuit: handler fired", "x [stealth-update] y"]), ExitKind::Forced);
        assert_eq!(exit_kind(&["nothing here"]), ExitKind::Unknown);
    }
}
