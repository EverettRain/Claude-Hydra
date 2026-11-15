//! The probe: a small Electron app (`probe/`) that Hydra starts as an environment's accounts'
//! Claude would start (variables, switches, relay, the same launch checks), to show where each
//! network path of an Electron app like Claude really leaves from. Claude itself refuses the
//! debugging switches that would show this, and the probe needs no Claude installed.
//!
//! Each environment gets a hidden probe account, so the probe goes through a relay with the same
//! exit guard as any account of that environment. "Protected", it runs inside the protection
//! Hydra has on the platform: Seatbelt on macOS (see `seatbelt`), the account's Sandboxie box on
//! Windows (see `sandbox`).
//!
//! It runs on the Electron version the installed Claude is built on where that can be read
//! (macOS), else a recent stable one, downloaded once from Electron's own GitHub releases and
//! checked against their published SHA-256.
#[cfg(any(target_os = "macos", windows))]
use std::path::{Path, PathBuf};

/// Used without a Claude to match: the latest stable Electron when this was written.
#[cfg(any(target_os = "macos", windows))]
const DEFAULT_ELECTRON: &str = "45.3.2";
const ACCOUNT_PREFIX: &str = "hydra-probe-";
/// In the probe's own profile folder: what it judges its findings against. A file rather than a
/// variable, since a Sandboxie box only passes variables without spaces or quotes.
#[cfg(any(target_os = "macos", windows))]
const EXPECTED: &str = "hydra-probe-expected.json";

/// The hidden account the probe runs as for environment `environment`.
pub fn account_id(environment: &str) -> String { format!("{ACCOUNT_PREFIX}{environment}") }

pub fn is_probe(account: &str) -> bool { account.starts_with(ACCOUNT_PREFIX) }

#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
const FILES: [(&str, &str); 5] = [
    ("package.json", include_str!("../probe/package.json")),
    ("main.js", include_str!("../probe/main.js")),
    ("preload.js", include_str!("../probe/preload.js")),
    ("index.html", include_str!("../probe/index.html")),
    ("renderer.js", include_str!("../probe/renderer.js")),
];

/// `major.minor.patch`, as Electron versions its releases.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn valid_version(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// The SHA-256 a `SHASUMS256.txt` lists for `file` (lines are `<hex> *<file>`).
#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
fn listed_sha256(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let (hash, name) = l.split_once(' ')?;
        (name.trim_start_matches('*') == file && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
    })
}

#[cfg(any(target_os = "macos", windows))]
fn root() -> PathBuf { crate::paths::storage_root().join("probe") }

/// The Electron version the installed Claude is built on (read from its bundle on macOS).
#[cfg(target_os = "macos")]
fn electron_version() -> Result<String, String> {
    let Some(app) = crate::install::resolve().ok().and_then(|i| i.aumid) else { return Ok(DEFAULT_ELECTRON.into()) };
    let plist = Path::new(&app).join("Contents/Frameworks/Electron Framework.framework/Resources/Info.plist");
    let out = std::process::Command::new("/usr/bin/plutil")
        .args(["-extract", "CFBundleVersion", "raw", "-o", "-"]).arg(&plist)
        .output().map_err(|e| format!("无法读取 Claude 的 Electron 版本：{e}"))?;
    let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || !valid_version(&version) { return Err("无法读取 Claude 的 Electron 版本。".into()); }
    Ok(version)
}

#[cfg(windows)]
fn electron_version() -> Result<String, String> { Ok(DEFAULT_ELECTRON.into()) }

/// Writes through to `file` while hashing what passes.
#[cfg(any(target_os = "macos", windows))]
struct Hashing<W> { file: W, hash: sha2::Sha256 }

#[cfg(any(target_os = "macos", windows))]
impl<W: std::io::Write> std::io::Write for Hashing<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        use sha2::Digest;
        let n = self.file.write(buf)?;
        self.hash.update(&buf[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> { self.file.flush() }
}

#[cfg(target_os = "macos")]
const PLATFORM: &str = "darwin";
#[cfg(windows)]
const PLATFORM: &str = "win32";

/// Where Electron's executable sits in its unpacked release.
#[cfg(target_os = "macos")]
const EXECUTABLE: &str = "Electron.app/Contents/MacOS/Electron";
#[cfg(windows)]
const EXECUTABLE: &str = "electron.exe";

#[cfg(target_os = "macos")]
fn unpack(zip: &Path, to: &Path) -> Result<(), String> {
    // `ditto` keeps the bundle's symbolic links and signature, which a plain unzip may not.
    let status = std::process::Command::new("/usr/bin/ditto").args(["-x", "-k"]).arg(zip).arg(to).status().map_err(|e| e.to_string())?;
    if status.success() { Ok(()) } else { Err("解压失败".into()) }
}

#[cfg(windows)]
fn unpack(zip: &Path, to: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip).map_err(|e| e.to_string())?;
    zip::ZipArchive::new(file).and_then(|mut a| a.extract(to)).map_err(|e| format!("解压失败：{e}"))
}

/// Electron's executable of `version`, downloaded and verified the first time.
#[cfg(any(target_os = "macos", windows))]
fn electron(version: &str) -> Result<PathBuf, String> {
    use sha2::Digest;
    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
    let dir = root().join(format!("electron-{version}-{arch}"));
    if dir.join(EXECUTABLE).is_file() { return Ok(dir.join(EXECUTABLE)); }
    let failed = |e: String| format!("无法下载探针运行环境（Electron {version}）：{e}");
    let name = format!("electron-v{version}-{PLATFORM}-{arch}.zip");
    let base = format!("https://github.com/electron/electron/releases/download/v{version}/");
    let client = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(900)).user_agent("Claude-Hydra")
        .build().map_err(|e| failed(e.to_string()))?;
    let sums = client.get(format!("{base}SHASUMS256.txt")).send().and_then(|r| r.error_for_status()).and_then(|r| r.text())
        .map_err(|e| failed(e.to_string()))?;
    let wanted = listed_sha256(&sums, &name).ok_or_else(|| failed("发布页没有列出它的校验值".into()))?;
    std::fs::create_dir_all(root()).map_err(|e| failed(e.to_string()))?;
    let zip = root().join(format!("{name}.part"));
    let mut sink = Hashing { file: std::fs::File::create(&zip).map_err(|e| failed(e.to_string()))?, hash: sha2::Sha256::new() };
    let copied = client.get(format!("{base}{name}")).send().and_then(|r| r.error_for_status()).map_err(|e| e.to_string())
        .and_then(|mut r| std::io::copy(&mut r, &mut sink).map_err(|e| e.to_string()));
    drop(sink.file);
    let got: String = sink.hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    let unpacked = root().join(format!("electron-{version}-{arch}.part"));
    let result = copied.and_then(|_| if got == wanted { Ok(()) } else { Err("下载的文件与发布页的校验值不符".into()) }).and_then(|_| {
        let _ = std::fs::remove_dir_all(&unpacked);
        unpack(&zip, &unpacked)?;
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::rename(&unpacked, &dir).map_err(|e| e.to_string())
    });
    let _ = std::fs::remove_file(&zip);
    let _ = std::fs::remove_dir_all(&unpacked);
    result.map_err(failed)?;
    Ok(dir.join(EXECUTABLE))
}

/// Writes the probe's files (fresh each time, so they match this Hydra).
#[cfg(any(target_os = "macos", windows))]
fn write_app() -> Result<PathBuf, String> {
    let dir = root().join("app");
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法写入探针：{e}"))?;
    for (name, text) in FILES {
        std::fs::write(dir.join(name), text).map_err(|e| format!("无法写入探针：{e}"))?;
    }
    Ok(dir)
}

/// Starts the probe as a Claude of environment `environment` would start: the same checks,
/// variables, switches and relay; `protected`, inside the platform's protection as well.
#[cfg(any(target_os = "macos", windows))]
pub fn launch(environment: &str, protected: bool) -> Result<(), String> {
    // Before the route check and a download that would be in vain.
    if protected && cfg!(windows) { crate::sandbox::available()?; }
    let name = crate::settings::load().environments.get(environment).map(|e| e.name.clone()).ok_or("找不到这个环境。")?;
    let id = account_id(environment);
    crate::settings::update(|s| {
        let a = s.account_mut(&id);
        a.environment = Some(environment.to_string());
        a.label = Some(format!("探针 · {name}"));
    })?;
    let spec = crate::launcher::prepare_for(&id, false, false)?;
    let port = match (protected, spec.relay_port) {
        (true, None) => return Err("加保护的测试只用于带代理节点的环境。".into()),
        (_, port) => port,
    };
    let electron = electron(&electron_version()?)?;
    let app = write_app()?;
    let profile = root().join("profiles").join(crate::sandbox::account_key(&id));
    std::fs::create_dir_all(&profile).map_err(|e| format!("无法创建探针数据目录：{e}"))?;
    let protection = protected.then_some(if cfg!(windows) { "sandboxie" } else { "seatbelt" });
    let expected = serde_json::json!({
        "subject": format!("环境「{name}」"),
        "relayPort": port,
        "exitIp": spec.exit.as_ref().map(|e| e.ip.clone()),
        "timezone": spec.env.iter().find(|(k, _)| k == "TZ").map(|(_, v)| v.clone()),
        "protection": protection,
    });
    std::fs::write(profile.join(EXPECTED), expected.to_string()).map_err(|e| format!("无法写入探针：{e}"))?;
    start(&id, &electron, &app, &profile, &spec, port.filter(|_| protected))
}

#[cfg(target_os = "macos")]
fn start(_id: &str, electron: &Path, app: &Path, profile: &Path, spec: &crate::launcher::Spec, protect: Option<u16>) -> Result<(), String> {
    let lead = [app.to_string_lossy().into_owned()];
    let profile = profile.to_string_lossy();
    let Some(port) = protect else {
        let bundle = electron.ancestors().find(|a| a.extension().is_some_and(|e| e == "app")).ok_or("Electron 不完整。")?;
        return crate::launcher::mac_open(&bundle.to_string_lossy(), &spec.env, &lead, Some(&profile), &spec.managed, &spec.extra)
            .map_err(|e| format!("无法启动探针：{e}"));
    };
    // Started directly, not through `open`: launchd would start it outside the sandbox.
    let mut args = crate::launcher::mac_args(&lead, Some(&profile), &spec.managed, &spec.extra);
    // macOS lets no process inside a sandbox set up another one, and Chromium sandboxes each of
    // its helper processes: they would fail to start. The probe loads only its own page and
    // public test services, so it can do without; a Claude showing claude.ai should not.
    args.push("--no-sandbox".into());
    let log_path = root().join("seatbelt.log");
    let log = std::fs::File::create(&log_path).map_err(|e| format!("无法写入探针日志：{e}"))?;
    let mut child = crate::seatbelt::command(electron, &args, port)
        .envs(spec.env.iter().cloned())
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log)
        .spawn().map_err(|e| format!("无法启动探针：{e}"))?;
    // A profile the system rejects, or an app that cannot run under it, ends right away.
    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if let Ok(Some(status)) = child.try_wait() {
            let text = std::fs::read_to_string(&log_path).unwrap_or_default();
            let tail: Vec<&str> = text.lines().rev().take(8).collect();
            let tail: Vec<&str> = tail.into_iter().rev().collect();
            return Err(format!("探针在 Seatbelt 下没能启动（{status}）。日志：{}\n{}", log_path.display(), tail.join("\n")));
        }
    }
    std::thread::spawn(move || { let _ = child.wait(); });
    Ok(())
}

#[cfg(windows)]
fn start(id: &str, electron: &Path, app: &Path, profile: &Path, spec: &crate::launcher::Spec, protect: Option<u16>) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let profile = profile.to_string_lossy();
    let line = format!("\"{}\" {}", app.display(), crate::launcher::windows_line(Some(&profile), &spec.managed, &spec.extra));
    let mut cmd = match protect {
        // The account's own box, set up as for a sandboxed account: only the relay is reachable,
        // and Hydra's DNS and identity guards are loaded.
        Some(port) => {
            crate::sandbox::configure(id, &profile, Some(port), crate::launcher::zone_of(&spec.env), &spec.locale)?;
            crate::sandbox::command(id, electron, &line, &spec.env)?
        }
        None => {
            let mut cmd = std::process::Command::new(electron);
            cmd.raw_arg(&line).envs(spec.env.iter().cloned());
            cmd
        }
    };
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    let mut child = cmd.spawn().map_err(|e| format!("无法启动探针：{e}"))?;
    std::thread::spawn(move || { let _ = child.wait(); });
    Ok(())
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn launch(_environment: &str, _protected: bool) -> Result<(), String> {
    Err("探针只支持 macOS 和 Windows。".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_accounts_are_told_apart_from_real_ones() {
        // Arrange
        let probe = account_id("env-1");

        // Act
        let (is, real) = (is_probe(&probe), is_probe("Account 2"));

        // Assert
        assert!(is && !real);
        assert_ne!(probe, account_id("env-2"), "each environment has its own probe account");
    }

    #[test]
    fn electron_versions_are_plain_releases() {
        // Arrange
        let versions = ["37.2.0", "37.2", "37.2.0-beta.1", "../37.2.0", "", "37..0"];

        // Act
        let valid: Vec<bool> = versions.iter().map(|v| valid_version(v)).collect();

        // Assert
        assert_eq!(valid, [true, false, false, false, false, false]);
    }

    #[test]
    fn the_listed_checksum_belongs_to_the_exact_file() {
        // Arrange
        let a = "a".repeat(64);
        let b = "B".repeat(64);
        let sums = format!("{a} *electron-v37.2.0-darwin-arm64.zip.sig\n{b} *electron-v37.2.0-darwin-arm64.zip\nshort *x.zip\n");

        // Act
        let found = listed_sha256(&sums, "electron-v37.2.0-darwin-arm64.zip");
        let missing = listed_sha256(&sums, "x.zip");

        // Assert
        assert_eq!(found, Some("b".repeat(64)));
        assert_eq!(missing, None, "a malformed hash is not a checksum");
    }

    #[test]
    fn the_probe_scripts_parse_as_javascript() {
        for (name, text) in FILES.iter().filter(|(n, _)| n.ends_with(".js")) {
            let path = std::env::temp_dir().join(format!("hydra-probe-check-{name}"));
            std::fs::write(&path, text).unwrap();
            let out = std::process::Command::new("node").arg("--check").arg(&path).output()
                .expect("Node.js is required for the probe script tests");
            let _ = std::fs::remove_file(&path);
            assert!(out.status.success(), "{name}: {}", String::from_utf8_lossy(&out.stderr));
        }
    }
}
