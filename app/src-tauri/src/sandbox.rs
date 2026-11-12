//! Optional per-account isolation in a Sandboxie-Plus box (Windows only, opt-in per environment).
//!
//! Without a box, Claude's network is routed through the account's relay by Chromium switches
//! and proxy variables, which a process may ignore. Inside a box, Sandboxie enforces it:
//! - `NetworkAccess`: everything is blocked except the account's relay on 127.0.0.1, so a
//!   process that ignores the proxy simply has no network (fail closed), UDP included;
//! - files and registry writes stay in the box, except the account's own data folder (so Hydra
//!   still sees sessions and usage) and the link drop folder (see `browser`);
//! - after starting, Hydra asks Sandboxie whether the account's Claude really runs in its box,
//!   and closes it if not (packaged Store builds may escape the box).
//!
//! Store builds cannot start in a box (Sandboxie gets "access denied" on `WindowsApps`), so a
//! sandboxed account runs a private copy of the package's `app` folder (see `launch_exe`).
//!
//! Hostname lookups in a box are normally handed to the Windows DNS client outside the box,
//! which settings cannot close. Every box therefore loads Hydra's DNS guard (`dnsguard.c`,
//! `InjectDll64`), which refuses that endpoint: external names then fail inside the box, while
//! Claude itself sends host names to its relay.
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the copies of Store builds for sandboxed accounts live (one folder per package version).
pub fn apps_dir() -> PathBuf { crate::paths::storage_root().join("sandbox-app") }

/// `...\WindowsApps\<package full name>\app` of the installed Store build (newest version).
#[cfg(windows)]
fn package_app_dir(install: &crate::install::Install) -> Option<PathBuf> {
    if let Some(dir) = install.exe.as_ref().and_then(|e| e.parent()).filter(|d| d.join("Claude.exe").is_file()) {
        return Some(dir.to_path_buf());
    }
    const REPO: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";
    let family = install.family.as_deref()?.to_lowercase();
    let (name, publisher) = family.split_once('_')?;
    let version = |full: &str| -> Vec<u32> { full.split('_').nth(1).unwrap_or("").split('.').map(|n| n.parse().unwrap_or(0)).collect() };
    let mut packages: Vec<String> = crate::sys::reg_subkeys(false, REPO).into_iter()
        .filter(|k| { let l = k.to_lowercase(); l.starts_with(&format!("{name}_")) && l.ends_with(&format!("__{publisher}")) })
        .collect();
    packages.sort_by_key(|k| std::cmp::Reverse(version(k)));
    packages.iter().find_map(|k| {
        let root = crate::sys::reg_string(false, &format!(r"{REPO}\{k}"), "PackageRootFolder")?;
        let app = PathBuf::from(root).join("app");
        app.join("Claude.exe").is_file().then_some(app)
    })
}

#[cfg(not(windows))]
fn package_app_dir(_install: &crate::install::Install) -> Option<PathBuf> { None }

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let target = to.join(e.file_name());
        if e.file_type()?.is_dir() { copy_tree(&e.path(), &target)?; } else { std::fs::copy(e.path(), &target)?; }
    }
    Ok(())
}

/// Executable a sandboxed account starts. Classic installs run in place. A Store build is
/// copied once per version out of `WindowsApps` (about 700 MB; the first sandboxed start of a
/// new version takes a moment), and copies of other versions no longer running are removed.
pub fn launch_exe(install: &crate::install::Install) -> Result<PathBuf, String> {
    if !install.is_msix() {
        return install.launch_exe().ok_or_else(|| "找不到 Claude 的启动程序。".into());
    }
    static COPYING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = COPYING.lock().unwrap_or_else(|e| e.into_inner());
    let src = package_app_dir(install).ok_or("找不到商店版 Claude 的安装目录，无法在沙箱中运行。")?;
    let package = src.parent().and_then(Path::file_name).ok_or("商店版 Claude 的安装目录无效。")?.to_string_lossy().to_string();
    let root = apps_dir();
    let dst = root.join(&package);
    if !dst.join(".complete").is_file() {
        let partial = root.join(format!("{package}.partial"));
        let _ = std::fs::remove_dir_all(&partial);
        let _ = std::fs::remove_dir_all(&dst);
        copy_tree(&src, &partial)
            .and_then(|_| std::fs::write(partial.join(".complete"), &package))
            .and_then(|_| std::fs::rename(&partial, &dst))
            .map_err(|e| { let _ = std::fs::remove_dir_all(&partial); format!("无法为沙箱复制商店版 Claude（{}）：{e}", root.display()) })?;
    }
    remove_stale_copies(&root, &package);
    Ok(dst.join("Claude.exe"))
}

/// Removes copies of other versions, unless a process still runs from them.
fn remove_stale_copies(root: &Path, keep: &str) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    let images: Vec<String> = crate::sys::processes().iter()
        .filter_map(|p| crate::sys::image_path(p.pid)).map(|i| i.to_lowercase()).collect();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name == keep || name.ends_with(".partial") { continue; }
        let prefix = format!("{}\\", e.path().to_string_lossy().to_lowercase());
        if images.iter().any(|i| i.starts_with(&prefix)) { continue; }
        let _ = std::fs::remove_dir_all(e.path());
    }
}

/// The DNS guard DLL built from `dnsguard.c` (64-bit Windows builds only).
#[cfg(all(windows, target_arch = "x86_64", target_env = "msvc"))]
const DNS_GUARD: Option<&[u8]> = Some(include_bytes!(env!("HYDRA_DNSGUARD")));
#[cfg(not(all(windows, target_arch = "x86_64", target_env = "msvc")))]
const DNS_GUARD: Option<&[u8]> = None;

/// Writes the DNS guard next to Hydra's data, named by content so a version still loaded by a
/// running box is never overwritten.
fn dns_guard_path() -> Result<Option<PathBuf>, String> {
    use sha2::{Digest, Sha256};
    let Some(bytes) = DNS_GUARD else { return Ok(None) };
    let hash: String = Sha256::digest(bytes).iter().take(6).map(|b| format!("{b:02x}")).collect();
    let dir = crate::paths::storage_root().join("sandbox");
    let path = dir.join(format!("hydra_dnsguard_{hash}.dll"));
    if std::fs::read(&path).ok().as_deref() != Some(bytes) {
        std::fs::create_dir_all(&dir)
            .and_then(|_| std::fs::write(&path, bytes))
            .map_err(|e| format!("无法写入沙箱 DNS 防护组件：{e}"))?;
    }
    Ok(Some(path))
}

/// Box name for an account: stable, short and free of characters Sandboxie rejects.
pub fn box_name(id: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(id.as_bytes());
    format!("Hydra_{}", hash.iter().take(5).map(|b| format!("{b:02x}")).collect::<String>())
}

/// Folder where links opened inside a box are dropped for the host Hydra (see `browser`).
pub fn drop_dir() -> PathBuf { crate::paths::storage_root().join("browser-requests") }

#[cfg(windows)]
fn install_dir() -> Option<PathBuf> {
    let uninstall = ["Sandboxie-Plus_is1", "Sandboxie-Plus"].iter().find_map(|k| {
        crate::sys::reg_string(true, &format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{k}"), "InstallLocation")
    });
    let program_files = std::env::var("ProgramW6432").or_else(|_| std::env::var("ProgramFiles")).unwrap_or_else(|_| r"C:\Program Files".into());
    uninstall.into_iter().map(|p| PathBuf::from(p.trim_matches('"')))
        .chain(["Sandboxie-Plus", "Sandboxie"].map(|d| Path::new(&program_files).join(d)))
        .find(|d| d.join("Start.exe").is_file() && d.join("SbieIni.exe").is_file())
}

#[cfg(not(windows))]
fn install_dir() -> Option<PathBuf> { None }

/// The Sandboxie-Plus folder, or a user-facing reason it cannot be used.
pub fn available() -> Result<PathBuf, String> {
    if !cfg!(windows) { return Err("沙箱隔离目前只支持 Windows（Sandboxie-Plus）。".into()); }
    install_dir().ok_or_else(|| "没有检测到 Sandboxie-Plus。请先安装 Sandboxie-Plus（需要管理员权限并安装驱动），再启用沙箱隔离。".into())
}

/// Whether this process itself runs inside a Sandboxie box.
#[cfg(windows)]
pub fn inside() -> bool {
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    let name: Vec<u16> = "SbieDll.dll".encode_utf16().chain([0]).collect();
    unsafe { !GetModuleHandleW(name.as_ptr()).is_null() }
}

#[cfg(not(windows))]
pub fn inside() -> bool { false }

fn ini(dir: &Path, args: &[&str]) -> Result<(), String> {
    let mut cmd = Command::new(dir.join("SbieIni.exe"));
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let status = cmd.args(args).status().map_err(|e| format!("无法调用 SbieIni：{e}"))?;
    if status.success() { Ok(()) } else { Err(format!("Sandboxie 拒绝了配置 {}（可能限制了只有管理员才能修改配置）。", args.get(2).unwrap_or(&""))) }
}

/// Sandboxie resolves UserSettings_ to the calling user's settings section.
/// Apply once on explicit request, since controller auto-start is shared by all boxes.
pub fn configure_quiet_start() -> Result<(), String> {
    let dir = available()?;
    let section = "UserSettings_";
    let detail = "2205,ConsoleInit (C00000D4)";
    ini(&dir, &["set", section, "SbieCtrl_EnableAutoStart", "n"])?;
    ini(&dir, &["delete", section, "SbieCtrl_HideMessage", detail])?;
    ini(&dir, &["append", section, "SbieCtrl_HideMessage", detail])
}

/// Creates or updates the account's box. `relay_port` = the only address the box may reach;
/// `None` (no node) leaves the network unrestricted.
pub fn configure(id: &str, data_dir: &str, relay_port: Option<u16>) -> Result<String, String> {
    let dir = available()?;
    let name = box_name(id);
    let drop = drop_dir();
    std::fs::create_dir_all(&drop).map_err(|e| format!("无法创建链接转交目录：{e}"))?;
    let set = |k: &str, v: &str| ini(&dir, &["set", &name, k, v]);
    let append = |k: &str, v: &str| ini(&dir, &["append", &name, k, v]);
    set("Enabled", "y")?;
    // Without a config level, the Sandboxie UI "upgrades" a new box with its defaults, which
    // include AutoRecover (copying Desktop/Documents files out) and opened device templates.
    set("ConfigLevel", "10")?;
    set("AutoRecover", "n")?;
    set("AutoDelete", "n")?;
    set("BlockNetworkFiles", "y")?;
    match dns_guard_path()? {
        Some(dll) => set("InjectDll64", &dll.to_string_lossy())?,
        None => { let _ = set("InjectDll64", ""); }
    }
    set("OpenFilePath", data_dir)?;
    append("OpenFilePath", &drop.to_string_lossy())?;
    match relay_port {
        Some(port) => {
            set("NetworkAccess", "*,Block")?;
            append("NetworkAccess", &format!("*,Allow;Address=127.0.0.1;Port={port}"))?;
        }
        None => { let _ = set("NetworkAccess", ""); }
    }
    let mut reload = Command::new(dir.join("Start.exe"));
    #[cfg(windows)]
    reload.creation_flags(0x0800_0000);
    if !reload.args(["/silent", "/reload"]).status().map_err(|e| format!("无法让 Sandboxie 重新加载配置：{e}"))?.success() {
        return Err("Sandboxie 未能重新加载账号配置。".into());
    }
    #[cfg(windows)]
    sync_box_proxy(&dir, &name, relay_port.filter(|_| crate::sys::claude_egress_policy().is_some()))?;
    Ok(name)
}

/// Edit only the physical copy of the value in the box hive. Removing that copy
/// restores Sandboxie's merged view of the current host policy.
#[cfg(windows)]
fn sync_box_proxy(dir: &Path, name: &str, port: Option<u16>) -> Result<(), String> {
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{FreeLibrary, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND};
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows_sys::Win32::System::Registry::*;
    let folder = crate::paths::storage_root().join("sandbox");
    let marker = folder.join(format!("{name}.proxy"));
    if port.is_none() && !marker.try_exists().map_err(|e| e.to_string())? { return Ok(()); }
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<_>>();
    let dll = wide(&dir.join("SbieDll.dll").to_string_lossy());
    let mut path = [0u16; 1024];
    let mut size = std::mem::size_of_val(&path) as u32;
    type Query = unsafe extern "system" fn(*const u16, *mut u16, *mut u16, *mut u16, *mut u32, *mut u32, *mut u32) -> i32;
    let status = unsafe {
        let module = LoadLibraryW(dll.as_ptr());
        if module.is_null() { return Err("无法加载 Sandboxie。".into()); }
        let rc = GetProcAddress(module, c"SbieApi_QueryBoxPath".as_ptr().cast()).map(|f| {
            let query: Query = std::mem::transmute(f);
            query(wide(name).as_ptr(), null_mut(), path.as_mut_ptr(), null_mut(), null_mut(), &mut size, null_mut())
        });
        FreeLibrary(module);
        rc
    };
    let path = String::from_utf16_lossy(&path[..path.iter().position(|&c| c == 0).unwrap_or(path.len())]);
    let lower = path.to_ascii_lowercase();
    let hive = lower.strip_prefix(r"\registry\user\").filter(|s| !s.contains('\\') && s.starts_with("sandbox_") && s.ends_with(&format!("_{}", name.to_ascii_lowercase())));
    let hive = hive.filter(|_| status == Some(0)).ok_or("无法确认账号沙箱的专用注册表位置。")?;
    let mut mount = Command::new(dir.join("Start.exe"));
    mount.args([&format!("/box:{name}"), "/silent", "mount_hive"]).creation_flags(0x0800_0000);
    mount.spawn().map_err(|e| format!("无法挂载账号沙箱注册表：{e}"))?;
    // Open the existing mount. Never create a hive or address HKCU/HKLM.
    let mut root = null_mut();
    let start = std::time::Instant::now();
    loop {
        let rc = unsafe { RegOpenKeyExW(HKEY_USERS, wide(hive).as_ptr(), 0, KEY_READ | KEY_WRITE, &mut root) };
        if rc == 0 { break; }
        if rc != ERROR_FILE_NOT_FOUND || start.elapsed().as_secs() >= 10 {
            return Err(format!("无法打开账号沙箱注册表（Windows 错误 {rc}）。"));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let result = (|| -> Result<(), String> {
        if port.is_some() {
            std::fs::create_dir_all(&folder).and_then(|_| std::fs::write(&marker, [])).map_err(|e| e.to_string())?;
        }
        for branch in [r"machine", r"user\current"] {
            let key = wide(&format!(r"{branch}\Software\Policies\Claude"));
            let value = wide("egressProxyUrl");
            let rc = if let Some(port) = port {
                let url = wide(&format!("http://127.0.0.1:{port}"));
                unsafe { RegSetKeyValueW(root, key.as_ptr(), value.as_ptr(), REG_SZ, url.as_ptr().cast(), (url.len() * 2) as u32) }
            } else {
                unsafe { RegDeleteKeyValueW(root, key.as_ptr(), value.as_ptr()) }
            };
            if rc != 0 && !(port.is_none() && matches!(rc, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND)) {
                return Err(format!("无法更新账号沙箱的代理策略（Windows 错误 {rc}）。"));
            }
        }
        if port.is_none() { std::fs::remove_file(&marker).map_err(|e| e.to_string())?; }
        Ok(())
    })();
    unsafe { RegCloseKey(root); }
    result
}

/// A command that runs `exe raw_args` inside the account's box. `env` is passed both inherited
/// and as `/env:` switches (only values without whitespace).
pub fn command(id: &str, exe: &Path, raw_args: &str, env: &[(String, String)]) -> Result<Command, String> {
    let dir = available()?;
    let mut cmd = Command::new(dir.join("Start.exe"));
    let switches: String = env.iter()
        .filter(|(k, v)| !k.contains(['=', '"']) && !v.contains(char::is_whitespace) && !v.contains('"'))
        .map(|(k, v)| format!(" /env:{k}={v}"))
        .collect();
    let line = format!("/box:{} /silent{switches} \"{}\" {raw_args}", box_name(id), exe.display());
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000).raw_arg(line);
    #[cfg(not(windows))]
    cmd.arg(line);
    cmd.envs(env.iter().cloned());
    Ok(cmd)
}

/// Asks Sandboxie which box `pid` runs in (`None` = not boxed or Sandboxie unavailable).
#[cfg(windows)]
pub fn box_of(pid: u32) -> Option<String> {
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    type QueryProcess = unsafe extern "system" fn(isize, *mut u16, *mut u16, *mut u16, *mut u32) -> i32;
    let dll: Vec<u16> = available().ok()?.join("SbieDll.dll").to_string_lossy().encode_utf16().chain([0]).collect();
    unsafe {
        let module = LoadLibraryW(dll.as_ptr());
        if module.is_null() { return None; }
        let f: QueryProcess = std::mem::transmute(GetProcAddress(module, c"SbieApi_QueryProcess".as_ptr().cast())?);
        let (mut name, mut image, mut sid, mut session) = ([0u16; 34], [0u16; 96], [0u16; 96], 0u32);
        if f(pid as isize, name.as_mut_ptr(), image.as_mut_ptr(), sid.as_mut_ptr(), &mut session) != 0 { return None; }
        let len = name.iter().position(|c| *c == 0).unwrap_or(name.len());
        Some(String::from_utf16_lossy(&name[..len])).filter(|n| !n.is_empty())
    }
}

#[cfg(not(windows))]
pub fn box_of(_pid: u32) -> Option<String> { None }

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    #[ignore = "requires Sandboxie-Plus and CLAUDE_HYDRA_ROOT pointing to a scratch folder"]
    fn boxed_proxy_round_trip() {
        assert!(std::env::var_os("CLAUDE_HYDRA_ROOT").is_some());
        let dir = super::available().unwrap();
        let data = crate::paths::storage_root().join("proxy-test");
        std::fs::create_dir_all(&data).unwrap();
        let id = "hydra-proxy-integration-test";
        let name = super::configure(id, &data.to_string_lossy(), None).unwrap();
        let key = r"Software\Policies\Claude";
        let host = [true, false].map(|machine| crate::sys::reg_string(machine, key, "egressProxyUrl"));
        for port in [Some(34567), Some(34568), None] {
            super::sync_box_proxy(&dir, &name, port).unwrap();
            for (i, hive) in ["HKLM", "HKCU"].iter().enumerate() {
                let output = data.join("policy.txt");
                let script = data.join("query.cmd");
                std::fs::write(&script, format!("@echo off\r\nreg query {hive}\\{key} /v egressProxyUrl > \"{}\" 2>nul\r\n", output.display())).unwrap();
                let cmd = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/cmd.exe");
                let status = std::process::Command::new(dir.join("Start.exe"))
                    .args([&format!("/box:{name}"), "/silent", "/wait"]).arg(cmd)
                    .args(["/d", "/c"]).arg(&script).status().unwrap();
                let expected = port.map(|p| format!("http://127.0.0.1:{p}")).or_else(|| host[i].clone());
                assert_eq!(status.success(), expected.is_some());
                if let Some(expected) = expected {
                    assert!(std::fs::read_to_string(&output).unwrap().trim_end().ends_with(&expected));
                }
                assert_eq!(crate::sys::reg_string(i == 0, key, "egressProxyUrl"), host[i]);
            }
        }
    }

    /// Real Sandboxie-Plus + Claude check: `CLAUDE_HYDRA_ROOT=<scratch> cargo test --lib sandboxie_end_to_end -- --ignored --nocapture`
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn sandboxie_end_to_end() {
        use std::time::{Duration, Instant};
        let root = crate::paths::storage_root();
        assert!(std::env::var_os("CLAUDE_HYDRA_ROOT").is_some(), "set CLAUDE_HYDRA_ROOT to a scratch folder");
        let id = "hydra-e2e";
        let data = root.join("e2e-data");
        let _ = std::fs::remove_dir_all(&data);
        std::fs::create_dir_all(&data).unwrap();
        let relay = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = relay.local_addr().unwrap().port();
        let other = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let other_port = other.local_addr().unwrap().port();
        let name = super::configure(id, &data.to_string_lossy(), Some(port)).unwrap();
        println!("box {name}, relay port {port}");

        // 1) network rules, through `command` (what launcher uses).
        let out = data.join("net.txt");
        let script = format!(
            "$r=@(); foreach($t in @(@('127.0.0.1',{port}),@('127.0.0.1',{other_port}),@('1.1.1.1',80),@('204.79.197.200',443))){{ try{{ $c=New-Object Net.Sockets.TcpClient; $c.Connect($t[0],$t[1]); $r+=\"$($t[0]):$($t[1])=open\"; $c.Close() }}catch{{ $r+=\"$($t[0]):$($t[1])=blocked\" }} }}; $n='h'+[guid]::NewGuid().ToString('N').Substring(0,10)+'.5.6.7.8.nip.io'; try{{ [void][Net.Dns]::GetHostAddresses($n); $r+='dns-external=resolved' }}catch{{ $r+='dns-external=failed' }}; try{{ [void](Resolve-DnsName example.org -Server 8.8.8.8 -DnsOnly -ErrorAction Stop); $r+='dns-server=resolved' }}catch{{ $r+='dns-server=failed' }}; try{{ [void][Net.Dns]::GetHostAddresses('localhost'); $r+='dns-localhost=ok' }}catch{{ $r+='dns-localhost=failed' }}; $r -join ' ' | Set-Content -Encoding ascii '{}'",
            out.display()
        );
        let probe = data.join("probe.ps1");
        std::fs::write(&probe, script).unwrap();
        let ps = std::path::PathBuf::from(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe");
        super::command(id, &ps, &format!("-NoProfile -ExecutionPolicy Bypass -File \"{}\"", probe.display()), &[]).unwrap().spawn().unwrap();
        let started = Instant::now();
        while !out.is_file() && started.elapsed() < Duration::from_secs(60) { std::thread::sleep(Duration::from_millis(300)); }
        let net = std::fs::read_to_string(&out).expect("boxed probe wrote no result");
        println!("net: {}", net.trim());
        assert!(net.contains(&format!("127.0.0.1:{port}=open")), "{net}");
        assert!(net.contains(&format!("127.0.0.1:{other_port}=blocked")), "{net}");
        assert!(net.contains("1.1.1.1:80=blocked") && net.contains("204.79.197.200:443=blocked"), "{net}");
        assert!(net.contains("dns-external=failed") && net.contains("dns-server=failed"), "DNS leaks out of the box: {net}");
        assert!(net.contains("dns-localhost=ok"), "{net}");

        // 2) Claude itself (Store builds through the copy), detected and confirmed in its box.
        relay.set_nonblocking(true).unwrap();
        while relay.accept().is_ok() {} // discard the connectivity probe above
        let install = crate::install::resolve().unwrap();
        let t = Instant::now();
        let exe = super::launch_exe(&install).unwrap();
        println!("exe {} (prepared in {:?})", exe.display(), t.elapsed());
        let udd = data.join("profile");
        super::command(id, &exe, &format!("--user-data-dir=\"{}\"", udd.display()), &[]).unwrap().spawn().unwrap();
        let key = crate::profiles::path_key(&udd.to_string_lossy());
        let started = Instant::now();
        let mut pid = None;
        while pid.is_none() && started.elapsed() < Duration::from_secs(40) {
            pid = crate::profiles::running_map(&install).get(&key).copied();
            std::thread::sleep(Duration::from_millis(500));
        }
        let pid = pid.expect("boxed Claude not detected by running_map");
        let boxed = super::box_of(pid);
        println!("claude pid {pid} in box {boxed:?}");
        std::thread::sleep(Duration::from_secs(8));
        let log = std::fs::read_to_string(udd.join("logs").join("main.log")).unwrap_or_default();
        let _ = std::process::Command::new(super::available().unwrap().join("Start.exe")).args([&format!("/box:{name}"), "/terminate"]).status();
        assert_eq!(boxed.as_deref(), Some(name.as_str()));
        if crate::sys::claude_egress_policy().is_some() {
            assert!(relay.accept().is_ok(), "Claude did not connect to its box's policy proxy");
        }
        assert!(udd.join("Local State").is_file() || udd.join("logs").is_dir(), "profile data did not reach the real folder");
        println!("claude log net lines: {}", log.lines().filter(|l| l.contains("net::") || l.contains("proxy")).take(4).collect::<Vec<_>>().join(" | "));
    }

    #[test]
    fn box_names_are_valid_and_stable() {
        let a = super::box_name("Account 2");
        assert_eq!(a, super::box_name("Account 2"));
        assert_ne!(a, super::box_name("Account 3"));
        assert!(a.len() <= 32 && a.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'), "{a}");
    }
}
