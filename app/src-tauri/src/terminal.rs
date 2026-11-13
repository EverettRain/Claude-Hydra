//! Opens a script in the terminal app chosen in Settings (macOS).
use std::path::Path;
use std::process::Command;

/// Supported terminal apps: settings value and display name. "" is the default.
pub const APPS: [(&str, &str); 3] = [("", "终端（Terminal.app）"), ("iterm", "iTerm2"), ("ghostty", "Ghostty")];

pub fn validate(app: &str) -> Result<(), String> {
    if APPS.iter().any(|(v, _)| *v == app) { Ok(()) } else { Err("不支持此终端。".into()) }
}

/// The command that opens `script` (an executable file) in a new window of `app`.
fn command(app: &str, script: &Path) -> Command {
    match app {
        "iterm" => {
            // The path is passed as an argument, never spliced into the AppleScript source.
            let mut c = Command::new("/usr/bin/osascript");
            c.args([
                "-e", "on run argv",
                "-e", "tell application \"iTerm\"",
                "-e", "activate",
                "-e", "create window with default profile command (quoted form of item 1 of argv)",
                "-e", "end tell",
                "-e", "end run",
            ]).arg(script);
            c
        }
        "ghostty" => {
            let mut c = Command::new("/usr/bin/open");
            c.args(["-na", "Ghostty", "--args", "-e"]).arg(script);
            c
        }
        _ => {
            let mut c = Command::new("/usr/bin/open");
            c.args(["-a", "Terminal"]).arg(script);
            c
        }
    }
}

pub fn open(app: &str, script: &Path) -> Result<(), String> {
    validate(app)?;
    let name = APPS.iter().find(|(v, _)| *v == app).map(|(_, n)| *n).unwrap_or("终端");
    let out = command(app, script).output().map_err(|e| format!("无法打开{name}：{e}"))?;
    if out.status.success() { return Ok(()); }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(format!("无法打开{name}，请确认已安装，或在设置中换一个终端。{}", if err.is_empty() { String::new() } else { format!("\n{err}") }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_app_is_valid_and_others_are_not() {
        // Arrange
        let listed: Vec<&str> = APPS.iter().map(|(v, _)| *v).collect();

        // Act
        let results: Vec<bool> = listed.iter().map(|a| validate(a).is_ok()).collect();

        // Assert
        assert!(results.iter().all(|ok| *ok));
        assert!(validate("xterm").is_err());
    }

    #[test]
    fn script_path_is_a_separate_argument() {
        // Arrange
        let script = Path::new("/tmp/a b/it's \"here\".command");

        // Act
        let commands: Vec<Command> = APPS.iter().map(|(app, _)| command(app, script)).collect();

        // Assert
        for c in commands {
            assert_eq!(c.get_args().last().unwrap(), script.as_os_str());
        }
    }
}
