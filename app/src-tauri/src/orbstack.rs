//! Binding to the user's own OrbStack (macOS): its `docker` CLI on the `orbstack` context.
//!
//! Hydra does not bundle a container runtime. It runs the user's `docker` CLI (found in the
//! usual OrbStack / Homebrew locations, or a path set in Settings) and always passes
//! `--context orbstack`, so another default context (e.g. Docker Desktop) is never used by
//! accident; `docker info` must then report OrbStack.
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const CONTEXT: &str = "orbstack";

/// The Mac as seen from OrbStack containers. Connections to it reach services that listen only
/// on the Mac's loopback, which is where Hydra's relays are.
pub const HOST_IP: &str = "0.250.250.254";

const CANDIDATES: [&str; 4] = [
    "~/.orbstack/bin/docker",
    "/usr/local/bin/docker",
    "/opt/homebrew/bin/docker",
    "/Applications/OrbStack.app/Contents/MacOS/xbin/docker",
];

pub fn supported() -> bool { cfg!(target_os = "macos") }

fn expand(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(rest),
        None => PathBuf::from(path),
    }
}

/// The docker CLI to use: the configured one, else the first existing candidate.
pub fn locate(configured: Option<&str>) -> Result<PathBuf, String> {
    if let Some(p) = configured.map(str::trim).filter(|p| !p.is_empty()) {
        let p = expand(p);
        return if p.is_file() { Ok(p) } else { Err(format!("找不到指定的 docker 程序：{}", p.display())) };
    }
    CANDIDATES.iter().map(|c| expand(c)).find(|p| p.is_file())
        .ok_or_else(|| "找不到 OrbStack 的 docker 命令。请安装并启动 OrbStack，或在设置中指定 docker 的路径。".into())
}

pub struct Docker {
    exe: PathBuf,
}

impl Docker {
    fn command<I, S>(&self, args: I) -> Command
    where I: IntoIterator<Item = S>, S: AsRef<OsStr> {
        let mut c = Command::new(&self.exe);
        c.arg("--context").arg(CONTEXT).args(args);
        c.env_remove("DOCKER_HOST").env_remove("DOCKER_CONTEXT");
        // Apps started from Finder get a minimal PATH; buildx and credential helpers sit next
        // to the CLI.
        let mut path: Vec<PathBuf> = self.exe.parent().map(Path::to_path_buf).into_iter().collect();
        path.extend(["/usr/bin", "/bin", "/usr/sbin", "/sbin", "/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
        if let Ok(joined) = std::env::join_paths(path) { c.env("PATH", joined); }
        c
    }

    /// Runs a docker command and returns its trimmed stdout; a failure returns its stderr.
    pub fn run<I, S>(&self, args: I) -> Result<String, String>
    where I: IntoIterator<Item = S>, S: AsRef<OsStr> {
        let out = self.command(args).stdin(Stdio::null()).output().map_err(|e| format!("无法运行 docker：{e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            Err(if err.is_empty() { format!("docker 返回错误（{}）。", out.status) } else { err })
        }
    }

    /// Whether a docker command succeeds (its output is discarded).
    pub fn ok<I, S>(&self, args: I) -> bool
    where I: IntoIterator<Item = S>, S: AsRef<OsStr> {
        self.command(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .status().is_ok_and(|s| s.success())
    }

    /// Runs a docker command on the caller's terminal (progress stays visible) and returns
    /// whether it succeeded.
    pub fn attached<I, S>(&self, args: I) -> Result<bool, String>
    where I: IntoIterator<Item = S>, S: AsRef<OsStr> {
        self.command(args).status().map(|s| s.success()).map_err(|e| format!("无法运行 docker：{e}"))
    }

    /// Replaces this process with an interactive docker command (Unix), so the terminal talks
    /// to the container directly. Returns only on failure.
    #[cfg(unix)]
    pub fn exec<I, S>(&self, args: I) -> String
    where I: IntoIterator<Item = S>, S: AsRef<OsStr> {
        use std::os::unix::process::CommandExt;
        format!("无法进入容器：{}", self.command(args).exec())
    }
}

/// Connects to OrbStack: finds the CLI and checks that the `orbstack` context answers and
/// really is OrbStack. Returns the CLI and OrbStack's server version.
pub fn connect_with(configured: Option<&str>) -> Result<(Docker, String), String> {
    if !supported() { return Err("Claude Code 容器需要 macOS 上的 OrbStack。".into()); }
    let docker = Docker { exe: locate(configured)? };
    let info = docker.run(["info", "--format", "{{.OperatingSystem}}|{{.ServerVersion}}"]).map_err(|e| {
        if e.contains("context") && e.contains("not found") {
            "docker 中没有 orbstack 上下文。请确认安装的是 OrbStack，并至少启动过一次。".to_string()
        } else {
            format!("无法连接 OrbStack，请确认 OrbStack 正在运行。\n{e}")
        }
    })?;
    let (os, version) = info.split_once('|').unwrap_or((info.as_str(), ""));
    if !os.contains("OrbStack") {
        return Err(format!("orbstack 上下文连接到的不是 OrbStack（{os}）。"));
    }
    Ok((docker, version.to_string()))
}

pub fn connect() -> Result<Docker, String> {
    connect_with(crate::settings::load().docker_path.as_deref()).map(|(d, _)| d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_path_must_exist() {
        // Arrange
        let missing = std::env::temp_dir().join(format!("hydra-no-docker-{}", std::process::id()));

        // Act
        let result = locate(Some(&missing.to_string_lossy()));

        // Assert
        assert!(result.unwrap_err().contains("找不到指定的 docker 程序"));
    }

    #[test]
    fn configured_path_wins_over_candidates() {
        // Arrange
        let file = std::env::temp_dir().join(format!("hydra-docker-{}", std::process::id()));
        std::fs::write(&file, "").unwrap();

        // Act
        let found = locate(Some(&format!("  {}  ", file.display())));

        // Assert
        assert_eq!(found.unwrap(), file);
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn home_prefix_is_expanded() {
        // Arrange
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();

        // Act
        let path = expand("~/.orbstack/bin/docker");

        // Assert
        assert_eq!(path, home.join(".orbstack/bin/docker"));
    }
}
