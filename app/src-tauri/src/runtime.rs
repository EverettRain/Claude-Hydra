//! Binding to the user's own Docker-compatible runtime: OrbStack on macOS, Docker Desktop
//! (WSL 2, Linux containers) on Windows.
//!
//! Hydra does not bundle a container runtime. It runs the user's `docker` CLI (found in the
//! runtime's usual locations, or a path set in Settings) and always passes the runtime's own
//! `--context`, so another default context is never used by accident; `docker info` must then
//! report that runtime.
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub struct Runtime {
    pub name: &'static str,
    context: &'static str,
    /// Part of `docker info`'s operating system when connected to this runtime.
    marker: &'static str,
    /// Where its docker CLI is usually installed (`~/` and `%VAR%` are expanded).
    candidates: &'static [&'static str],
    /// This computer as containers reach it: connections to it arrive on the computer's
    /// loopback, where Hydra's relays listen. An IP, or a name `add_host` defines.
    pub host: &'static str,
    /// `--add-host` entry for the gateway that makes `host` resolvable.
    pub add_host: Option<&'static str>,
    /// Whether the runtime forwards this computer's SSH agent into containers.
    pub ssh_agent: bool,
}

pub const ORBSTACK: Runtime = Runtime {
    name: "OrbStack",
    context: "orbstack",
    marker: "OrbStack",
    candidates: &[
        "~/.orbstack/bin/docker",
        "/usr/local/bin/docker",
        "/opt/homebrew/bin/docker",
        "/Applications/OrbStack.app/Contents/MacOS/xbin/docker",
    ],
    host: "0.250.250.254",
    add_host: None,
    ssh_agent: true,
};

pub const DOCKER_DESKTOP: Runtime = Runtime {
    name: "Docker Desktop",
    context: "desktop-linux",
    marker: "Docker Desktop",
    candidates: &["%ProgramFiles%\\Docker\\Docker\\resources\\bin\\docker.exe"],
    // `host-gateway` is Docker Desktop's way into the host (what host.docker.internal names).
    host: "hydra-host",
    add_host: Some("hydra-host:host-gateway"),
    ssh_agent: false,
};

/// The runtime containers use on this platform, if there is one.
pub fn current() -> Option<&'static Runtime> {
    if cfg!(target_os = "macos") { Some(&ORBSTACK) } else if cfg!(windows) { Some(&DOCKER_DESKTOP) } else { None }
}

pub fn supported() -> bool { current().is_some() }

fn unsupported() -> String { "Claude Code 容器需要 macOS 上的 OrbStack，或 Windows 上的 Docker Desktop。".into() }

fn expand(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        return std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(rest);
    }
    if let Some((var, rest)) = path.strip_prefix('%').and_then(|p| p.split_once('%')) {
        if let Some(value) = std::env::var_os(var) { return PathBuf::from(value).join(rest.trim_start_matches(['\\', '/'])); }
    }
    PathBuf::from(path)
}

/// `docker` on the PATH, for installs outside the usual places.
fn on_path() -> Option<PathBuf> {
    let name = if cfg!(windows) { "docker.exe" } else { "docker" };
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file())
}

impl Runtime {
    /// The docker CLI to use: the configured one, else the first existing candidate.
    pub fn locate(&self, configured: Option<&str>) -> Result<PathBuf, String> {
        if let Some(p) = configured.map(str::trim).filter(|p| !p.is_empty()) {
            let p = expand(p);
            return if p.is_file() { Ok(p) } else { Err(format!("找不到指定的 docker 程序：{}", p.display())) };
        }
        self.candidates.iter().map(|c| expand(c)).find(|p| p.is_file()).or_else(on_path)
            .ok_or_else(|| format!("找不到 {} 的 docker 命令。请安装并启动 {}，或在设置中指定 docker 的路径。", self.name, self.name))
    }

    /// Checks `docker info` (operating system, server version, OS type) against this runtime;
    /// returns the server version.
    fn check_info(&self, info: &str) -> Result<String, String> {
        let mut parts = info.split('|');
        let (os, version, os_type) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        if !os.contains(self.marker) {
            return Err(format!("{} 上下文连接到的不是 {}（{os}）。", self.context, self.name));
        }
        if os_type != "linux" {
            return Err(format!("{} 正在使用 {os_type} 容器。请切换到 Linux 容器后再试。", self.name));
        }
        Ok(version.to_string())
    }
}

pub struct Docker {
    exe: PathBuf,
    context: &'static str,
}

impl Docker {
    fn command<I, S>(&self, args: I) -> Command
    where I: IntoIterator<Item = S>, S: AsRef<OsStr> {
        let mut c = Command::new(&self.exe);
        c.arg("--context").arg(self.context).args(args);
        c.env_remove("DOCKER_HOST").env_remove("DOCKER_CONTEXT");
        // Credential helpers and buildx sit next to the CLI. Apps started from Finder get a
        // minimal PATH, so the usual places are added there too.
        let mut path: Vec<PathBuf> = self.exe.parent().map(Path::to_path_buf).into_iter().collect();
        if cfg!(windows) {
            path.extend(std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default());
        } else {
            path.extend(["/usr/bin", "/bin", "/usr/sbin", "/sbin", "/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
        }
        if let Ok(joined) = std::env::join_paths(path) { c.env("PATH", joined); }
        c
    }

    /// A command whose output Hydra reads; it never opens a console window of its own.
    fn quiet<I, S>(&self, args: I) -> Command
    where I: IntoIterator<Item = S>, S: AsRef<OsStr> {
        #[allow(unused_mut)]
        let mut c = self.command(args);
        #[cfg(windows)]
        { use std::os::windows::process::CommandExt; c.creation_flags(0x0800_0000); } // CREATE_NO_WINDOW
        c
    }

    /// Runs a docker command and returns its trimmed stdout; a failure returns its stderr.
    pub fn run<I, S>(&self, args: I) -> Result<String, String>
    where I: IntoIterator<Item = S>, S: AsRef<OsStr> {
        let out = self.quiet(args).stdin(Stdio::null()).output().map_err(|e| format!("无法运行 docker：{e}"))?;
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
        self.quiet(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .status().is_ok_and(|s| s.success())
    }

    /// Runs a docker command on the caller's terminal (progress stays visible) and returns
    /// whether it succeeded.
    pub fn attached<I, S>(&self, args: I) -> Result<bool, String>
    where I: IntoIterator<Item = S>, S: AsRef<OsStr> {
        self.command(args).status().map(|s| s.success()).map_err(|e| format!("无法运行 docker：{e}"))
    }

    /// Runs an interactive docker command on the caller's terminal and returns its exit code.
    pub fn interactive<I, S>(&self, args: I) -> Result<i32, String>
    where I: IntoIterator<Item = S>, S: AsRef<OsStr> {
        self.command(args).status().map(|s| s.code().unwrap_or(1)).map_err(|e| format!("无法进入容器：{e}"))
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

/// Connects to this platform's runtime: finds the CLI and checks that the runtime's context
/// answers and really is that runtime. Returns the CLI, the runtime and its server version.
pub fn connect_with(configured: Option<&str>) -> Result<(Docker, &'static Runtime, String), String> {
    let runtime = current().ok_or_else(unsupported)?;
    let docker = Docker { exe: runtime.locate(configured)?, context: runtime.context };
    let info = docker.run(["info", "--format", "{{.OperatingSystem}}|{{.ServerVersion}}|{{.OSType}}"]).map_err(|e| {
        if e.contains("context") && e.contains("not found") {
            format!("docker 中没有 {} 上下文。请确认安装的是 {}，并至少启动过一次。", runtime.context, runtime.name)
        } else {
            format!("无法连接 {}，请确认 {} 正在运行。\n{e}", runtime.name, runtime.name)
        }
    })?;
    let version = runtime.check_info(&info)?;
    Ok((docker, runtime, version))
}

pub fn connect() -> Result<(Docker, &'static Runtime), String> {
    connect_with(crate::settings::load().docker_path.as_deref()).map(|(d, r, _)| (d, r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_path_must_exist() {
        // Arrange
        let missing = std::env::temp_dir().join(format!("hydra-no-docker-{}", std::process::id()));

        // Act
        let result = ORBSTACK.locate(Some(&missing.to_string_lossy()));

        // Assert
        assert!(result.unwrap_err().contains("找不到指定的 docker 程序"));
    }

    #[test]
    fn configured_path_wins_over_candidates() {
        // Arrange
        let file = std::env::temp_dir().join(format!("hydra-docker-{}", std::process::id()));
        std::fs::write(&file, "").unwrap();

        // Act
        let found = DOCKER_DESKTOP.locate(Some(&format!("  {}  ", file.display())));

        // Assert
        assert_eq!(found.unwrap(), file);
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn home_and_variables_are_expanded() {
        // Arrange
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let (var, value) = std::env::vars_os().find(|(k, v)| !k.is_empty() && !v.is_empty() && k.to_str().is_some_and(|k| k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))).unwrap();

        // Act
        let orb = expand("~/.orbstack/bin/docker");
        let program_files = expand(&format!("%{}%\\Docker\\docker.exe", var.to_string_lossy()));
        let unknown = expand("%HYDRA_NOT_SET_ANYWHERE%\\docker.exe");

        // Assert
        assert_eq!(orb, home.join(".orbstack/bin/docker"));
        assert_eq!(program_files, PathBuf::from(value).join("Docker\\docker.exe"));
        assert_eq!(unknown, PathBuf::from("%HYDRA_NOT_SET_ANYWHERE%\\docker.exe"), "an unset variable is left as written");
    }

    #[test]
    fn info_must_name_the_runtime_and_linux_containers() {
        // Arrange
        let cases = [
            (&ORBSTACK, "OrbStack|28.1.1|linux", Ok("28.1.1")),
            (&DOCKER_DESKTOP, "Docker Desktop|28.3.2|linux", Ok("28.3.2")),
            (&DOCKER_DESKTOP, "Docker Desktop|28.3.2|windows", Err("windows")),
            (&DOCKER_DESKTOP, "Ubuntu 24.04 LTS|28.3.2|linux", Err("不是 Docker Desktop")),
            (&ORBSTACK, "Docker Desktop|28.3.2|linux", Err("不是 OrbStack")),
        ];

        for (runtime, info, expected) in cases {
            // Act
            let result = runtime.check_info(info);

            // Assert
            match expected {
                Ok(version) => assert_eq!(result.as_deref(), Ok(version), "{info}"),
                Err(part) => assert!(result.unwrap_err().contains(part), "{info}"),
            }
        }
    }

    #[test]
    fn each_runtime_can_reach_the_host() {
        // Arrange
        let runtimes = [&ORBSTACK, &DOCKER_DESKTOP];

        for r in runtimes {
            // Act
            let literal = r.host.parse::<std::net::Ipv4Addr>().is_ok();
            let named = r.add_host.is_some_and(|h| h.split_once(':').is_some_and(|(name, _)| name == r.host));

            // Assert
            assert!(literal != named, "{}: the host is either an address or a name the gateway is told about", r.name);
        }
    }
}
