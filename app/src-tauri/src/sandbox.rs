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
//!
//! A box virtualizes files and the registry, not what the system reports about itself, so every
//! box also carries the identity guard (`idguard.c`, same `InjectDll`) answering the machine GUID,
//! hardware profile GUID, volume serial, MAC addresses and computer name with values derived per
//! account (see `identity`). Two accounts then do not look like one device.
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

/// The same guard for 32-bit processes in a box (built when the x86 toolchain is installed).
#[cfg(hydra_guard32)]
const DNS_GUARD32: Option<&[u8]> = Some(include_bytes!(env!("HYDRA_DNSGUARD32")));
#[cfg(not(hydra_guard32))]
const DNS_GUARD32: Option<&[u8]> = None;

/// The identity guard DLL built from `idguard.c` (64-bit Windows builds only).
#[cfg(all(windows, target_arch = "x86_64", target_env = "msvc"))]
const IDENTITY_GUARD: Option<&[u8]> = Some(include_bytes!(env!("HYDRA_IDGUARD")));
#[cfg(not(all(windows, target_arch = "x86_64", target_env = "msvc")))]
const IDENTITY_GUARD: Option<&[u8]> = None;

/// The same guard for 32-bit processes in a box.
#[cfg(hydra_idguard32)]
const IDENTITY_GUARD32: Option<&[u8]> = Some(include_bytes!(env!("HYDRA_IDGUARD32")));
#[cfg(not(hydra_idguard32))]
const IDENTITY_GUARD32: Option<&[u8]> = None;

/// Writes a guard DLL next to Hydra's data, named by content so a version still loaded by a
/// running box is never overwritten.
fn guard_path(bytes: Option<&[u8]>, stem: &str) -> Result<Option<PathBuf>, String> {
    use sha2::{Digest, Sha256};
    let Some(bytes) = bytes else { return Ok(None) };
    let hash: String = Sha256::digest(bytes).iter().take(6).map(|b| format!("{b:02x}")).collect();
    let dir = crate::paths::storage_root().join("sandbox");
    let path = dir.join(format!("{stem}_{hash}.dll"));
    if std::fs::read(&path).ok().as_deref() != Some(bytes) {
        std::fs::create_dir_all(&dir)
            .and_then(|_| std::fs::write(&path, bytes))
            .map_err(|e| format!("无法写入沙箱防护组件：{e}"))?;
    }
    Ok(Some(path))
}

fn dns_guard_path() -> Result<Option<PathBuf>, String> { guard_path(DNS_GUARD, "hydra_dnsguard") }

fn identity_guard_path() -> Result<Option<PathBuf>, String> { guard_path(IDENTITY_GUARD, "hydra_idguard") }

/// Gives a box one `InjectDll` / `InjectDll64` entry per guard. Sandboxie reads each value as a
/// single DLL path and does not split it, so a comma-joined list is taken as one nonexistent
/// path and *no* guard loads — the box then runs with neither the network nor the identity
/// guard. Clearing first also drops an entry for a guard this build no longer carries.
#[cfg(windows)]
fn set_injection(dir: &Path, name: &str, key: &str, guards: &[Option<PathBuf>]) -> Result<(), String> {
    ini(dir, &["set", name, key, ""])?;
    for path in guards.iter().flatten() {
        ini(dir, &["append", name, key, &path.to_string_lossy()])?;
    }
    Ok(())
}

/// Whether the account's Claude (a 64-bit process) has every guard this box needs loaded.
/// `None` when that cannot be determined (no guard in this build, or its modules cannot be
/// listed).
#[cfg(windows)]
pub fn guard_loaded(pid: u32) -> Option<bool> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
    if DNS_GUARD.is_none() && IDENTITY_GUARD.is_none() { return None; }
    let wanted: Vec<String> = [dns_guard_path(), identity_guard_path()].into_iter()
        .filter_map(|p| p.ok().flatten())
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
        .collect();
    if wanted.is_empty() { return None; }
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE, pid);
        if snap == INVALID_HANDLE_VALUE { return None; }
        let mut entry: MODULEENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<MODULEENTRY32W>() as u32;
        let mut seen = 0;
        let mut found: Vec<bool> = vec![false; wanted.len()];
        let mut more = Module32FirstW(snap, &mut entry);
        while more != 0 {
            seen += 1;
            let len = entry.szModule.iter().position(|c| *c == 0).unwrap_or(entry.szModule.len());
            let name = String::from_utf16_lossy(&entry.szModule[..len]).to_lowercase();
            if let Some(i) = wanted.iter().position(|w| w == &name) { found[i] = true; }
            if found.iter().all(|f| *f) { break; }
            more = Module32NextW(snap, &mut entry);
        }
        CloseHandle(snap);
        // An empty list means the snapshot was not really readable, not that a guard is absent.
        (seen > 0).then_some(found.iter().all(|f| *f))
    }
}

#[cfg(not(windows))]
pub fn guard_loaded(_pid: u32) -> Option<bool> { None }

/// Folder name for per-account data outside the profile (browser data, Claude Code config).
pub fn account_key(id: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(id.as_bytes()).iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// Claude Code's own config folder for an account whose environment keeps it separate
/// (`CLAUDE_CONFIG_DIR`): sign-in, settings and sessions of Claude Code are then not shared
/// with other accounts through `~/.claude`.
pub fn code_dir(id: &str) -> PathBuf { code_root().join(account_key(id)) }
pub fn code_root() -> PathBuf { crate::paths::storage_root().join("claude-code") }

/// Ends every process still running in the account's box.
pub fn terminate_box(id: &str) {
    #[cfg(windows)]
    if let Ok(dir) = available() {
        let mut cmd = Command::new(dir.join("Start.exe"));
        cmd.creation_flags(0x0800_0000);
        let _ = cmd.args([&format!("/box:{}", box_name(id)), "/silent", "/terminate"]).status();
    }
    #[cfg(not(windows))]
    let _ = id;
}

/// Removes what a deleted account left outside its profile folder: its box (settings and
/// contents), the box's proxy marker and its separate Claude Code folder.
pub fn forget(id: &str) {
    let name = box_name(id);
    let _ = std::fs::remove_dir_all(code_dir(id));
    let _ = std::fs::remove_file(crate::paths::storage_root().join("sandbox").join(format!("{name}.proxy")));
    #[cfg(windows)]
    if let Ok(dir) = available() {
        // Only a box Hydra configured for this account: it exists and is named by the id's hash.
        let (status, root) = match box_file_root(&dir, &name) { Ok(root) => (0, Some(root)), Err(_) => (-1, None) };
        let mut cmd = Command::new(dir.join("Start.exe"));
        cmd.creation_flags(0x0800_0000);
        let _ = cmd.args([&format!("/box:{name}"), "/silent", "/terminate"]).status();
        if let Some(root) = root.filter(|r| status == 0 && r.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(&name))) {
            let _ = std::fs::remove_dir_all(root);
        }
        // The derived identifiers first and by name: a box re-created for the same account must
        // not answer with the previous account's device if the wildcard wipe ever misses them.
        for key in ["HydraMachineGuid", "HydraHwProfileGuid", "HydraComputerName", "HydraMac", "HydraVolumeSerial"] {
            let _ = ini(&dir, &["set", &name, key, ""]);
        }
        let _ = ini(&dir, &["set", &name, "*", ""]);
        let mut reload = Command::new(dir.join("Start.exe"));
        reload.creation_flags(0x0800_0000);
        let _ = reload.args(["/silent", "/reload"]).status();
    }
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
/// `None` (no node) leaves the network unrestricted. `code_dir` = the account's own Claude
/// Code folder when its environment keeps Claude Code separate; otherwise the shared
/// `~/.claude` is opened to the box.
pub fn configure(id: &str, data_dir: &str, relay_port: Option<u16>, code_dir: Option<&Path>) -> Result<String, String> {
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
    let guard = dns_guard_path()?;
    let guard32 = guard_path(DNS_GUARD32, "hydra_dnsguard32")?;
    let id_guard = identity_guard_path()?;
    let id_guard32 = guard_path(IDENTITY_GUARD32, "hydra_idguard32")?;
    set_injection(&dir, &name, "InjectDll64", &[guard.clone(), id_guard.clone()])?;
    set_injection(&dir, &name, "InjectDll", &[guard32.clone(), id_guard32])?;
    // What `idguard.c` reports inside the box in place of this computer's own identifiers, so
    // accounts on one machine do not look like one device. The guard stays inert when any of
    // these is missing, which also keeps an unreadable value from becoming a random-looking one.
    let ids = crate::identity::Identifiers::for_account(id);
    set("HydraMachineGuid", &ids.machine_guid)?;
    set("HydraHwProfileGuid", &ids.hw_profile_guid)?;
    set("HydraComputerName", &ids.computer_name)?;
    set("HydraMac", &ids.mac)?;
    set("HydraVolumeSerial", &ids.volume_serial)?;
    set("OpenFilePath", data_dir)?;
    append("OpenFilePath", &drop.to_string_lossy())?;
    // Claude Code's sessions and settings, shared with unboxed accounts as without a box, so
    // Hydra (and other accounts) see what a boxed account did. `*` covers its temp-and-rename saves.
    let code = match code_dir {
        Some(dir) => {
            std::fs::create_dir_all(dir).map_err(|e| format!("无法创建账号的 Claude Code 目录：{e}"))?;
            vec![dir.to_path_buf()]
        }
        None => claude_code_paths(),
    };
    for path in &code { append("OpenFilePath", &path.to_string_lossy())?; }
    match relay_port {
        Some(port) => {
            set("NetworkAccess", "*,Block")?;
            // Loopback is narrowed again by the DNS guard (relay port or listeners in this box),
            // which lets runtimes connect socket pairs to themselves. Without it, only the relay.
            // All of loopback is opened only when every process in the box carries a guard:
            // a 32-bit program without one could otherwise reach other accounts' relays or a
            // local proxy of the host and leave through a different exit.
            let loopback = if guard.is_some() && guard32.is_some() { "*,Allow;Address=127.0.0.1".to_string() } else { format!("*,Allow;Address=127.0.0.1;Port={port}") };
            append("NetworkAccess", &loopback)?;
            set("HydraRelayPort", &port.to_string())?;
        }
        None => {
            let _ = set("NetworkAccess", "");
            let _ = set("HydraRelayPort", "");
        }
    }
    let mut reload = Command::new(dir.join("Start.exe"));
    #[cfg(windows)]
    reload.creation_flags(0x0800_0000);
    if !reload.args(["/silent", "/reload"]).status().map_err(|e| format!("无法让 Sandboxie 重新加载配置：{e}"))?.success() {
        return Err("Sandboxie 未能重新加载账号配置。".into());
    }
    #[cfg(windows)]
    {
        let root = box_file_root(&dir, &name)?;
        sync_box_proxy(&dir, &name, relay_port.filter(|_| crate::sys::claude_egress_policy().is_some()))?;
        sync_box_gitconfig(&root, relay_port)?;
        recover_claude_code_data(&root, &code);
    }
    Ok(name)
}

/// The user profile folder (where Claude Code and git keep their per-user files).
fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// `~/.claude` (or `CLAUDE_CONFIG_DIR`) and the `~/.claude.json*` files next to the home folder.
fn claude_code_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) { paths.push(PathBuf::from(dir)); }
    if let Some(home) = home() {
        if paths.is_empty() { paths.push(home.join(".claude")); }
        paths.push(home.join(".claude.json*"));
    }
    paths
}

/// Where the box keeps its copy of `host_path`: Sandboxie maps the user profile to
/// `user\current` and every other path to `drive\<letter>`.
fn box_copy(root: &Path, host_path: &Path) -> Option<PathBuf> {
    if let Some(rest) = home().and_then(|h| host_path.strip_prefix(h).ok().map(Path::to_path_buf)) {
        return Some(root.join("user").join("current").join(rest));
    }
    let s = host_path.to_string_lossy();
    let (drive, rest) = s.split_once(":\\")?;
    (drive.len() == 1).then(|| root.join("drive").join(drive.to_ascii_uppercase()).join(rest))
}

/// Box folder on disk. Sandboxie reports an NT path (`\Device\HarddiskVolume3\Sandbox\...`),
/// mapped back to its drive letter.
#[cfg(windows)]
fn box_file_root(dir: &Path, name: &str) -> Result<PathBuf, String> {
    use windows_sys::Win32::Storage::FileSystem::QueryDosDeviceW;
    let (status, file, _) = query_box_path(dir, name)?;
    let fail = || "无法确认账号沙箱的文件位置。".to_string();
    if status != 0 { return Err(fail()); }
    if let Some(p) = file.strip_prefix(r"\??\").filter(|p| Path::new(p).is_absolute()) { return Ok(PathBuf::from(p)); }
    for letter in 'A'..='Z' {
        let drive: Vec<u16> = format!("{letter}:").encode_utf16().chain([0]).collect();
        let mut target = [0u16; 512];
        let n = unsafe { QueryDosDeviceW(drive.as_ptr(), target.as_mut_ptr(), target.len() as u32) };
        if n == 0 { continue; }
        let device = String::from_utf16_lossy(&target[..target.iter().position(|&c| c == 0).unwrap_or(0)]);
        if device.is_empty() { continue; }
        let Some(rest) = file.get(..device.len()).filter(|p| p.eq_ignore_ascii_case(&device)).map(|_| &file[device.len()..]) else { continue };
        if rest.starts_with('\\') { return Ok(PathBuf::from(format!("{letter}:{rest}"))); }
    }
    Err(fail())
}

/// `SbieApi_QueryBoxPath`: (status, file root, registry root).
#[cfg(windows)]
fn query_box_path(dir: &Path, name: &str) -> Result<(i32, String, String), String> {
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::FreeLibrary;
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<_>>();
    let dll = wide(&dir.join("SbieDll.dll").to_string_lossy());
    let (mut file, mut key) = ([0u16; 1024], [0u16; 1024]);
    let (mut file_size, mut key_size) = (std::mem::size_of_val(&file) as u32, std::mem::size_of_val(&key) as u32);
    type Query = unsafe extern "system" fn(*const u16, *mut u16, *mut u16, *mut u16, *mut u32, *mut u32, *mut u32) -> i32;
    let status = unsafe {
        let module = LoadLibraryW(dll.as_ptr());
        if module.is_null() { return Err("无法加载 Sandboxie。".into()); }
        let rc = GetProcAddress(module, c"SbieApi_QueryBoxPath".as_ptr().cast()).map(|f| {
            let query: Query = std::mem::transmute(f);
            query(wide(name).as_ptr(), file.as_mut_ptr(), key.as_mut_ptr(), null_mut(), &mut file_size, &mut key_size, null_mut())
        });
        FreeLibrary(module);
        rc
    };
    let text = |b: &[u16]| String::from_utf16_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]);
    Ok((status.unwrap_or(-1), text(&file), text(&key)))
}

const GITCONFIG_MARK: &str = "# Claude-Hydra: sandbox copy of the global git config";

/// Git reads proxies from its config before the environment, so a host config that sends git
/// to a host proxy, or straight out for some sites (`http.<url>.proxy =`), cannot work in the
/// box. The box gets its own `~/.gitconfig`: the host file followed by `http.proxy` and every
/// site-specific proxy pointed at the relay. Rewritten at each launch (so host edits carry
/// over; changes made from inside the box are replaced) and removed without a relay.
fn sync_box_gitconfig(root: &Path, relay_port: Option<u16>) -> Result<(), String> {
    let Some(host) = home().map(|h| h.join(".gitconfig")) else { return Ok(()) };
    let Some(copy) = box_copy(root, &host) else { return Ok(()) };
    let ours = std::fs::read_to_string(&copy).is_ok_and(|s| s.starts_with(GITCONFIG_MARK));
    let Some(port) = relay_port else {
        if ours { std::fs::remove_file(&copy).map_err(|e| format!("无法移除账号沙箱的 git 配置：{e}"))?; }
        return Ok(());
    };
    let original = std::fs::read_to_string(&host).unwrap_or_default();
    let content = gitconfig_for_box(&host, &original, &site_proxies(), port);
    if std::fs::read_to_string(&copy).ok().as_deref() == Some(content.as_str()) { return Ok(()); }
    if let Some(parent) = copy.parent() { std::fs::create_dir_all(parent).map_err(|e| format!("无法写入账号沙箱的 git 配置：{e}"))?; }
    std::fs::write(&copy, content).map_err(|e| format!("无法写入账号沙箱的 git 配置：{e}"))
}

fn gitconfig_for_box(host: &Path, original: &str, sites: &[String], port: u16) -> String {
    let proxy = format!("http://127.0.0.1:{port}");
    let mut out = format!("{GITCONFIG_MARK} ({}); regenerated at every launch.\n", host.display());
    out.push_str(original);
    if !out.ends_with('\n') { out.push('\n'); }
    out.push_str(&format!("\n{GITCONFIG_MARK}: network goes through the account relay.\n[http]\n\tproxy = {proxy}\n"));
    for site in sites {
        let quoted = site.replace('\\', "\\\\").replace('"', "\\\"");
        out.push_str(&format!("[http \"{quoted}\"]\n\tproxy = {proxy}\n"));
    }
    out
}

/// URLs with their own `http.<url>.proxy` in the host's system or global git config.
fn site_proxies() -> Vec<String> {
    let mut cmd = Command::new("git");
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    cmd.args(["config", "--includes", "--name-only", "--get-regexp", r"^http\..+\.proxy$"])
        .current_dir(std::env::temp_dir());
    let Ok(out) = cmd.output() else { return Vec::new() };
    let mut sites: Vec<String> = String::from_utf8_lossy(&out.stdout).lines()
        .filter_map(|l| l.trim().strip_prefix("http.")?.strip_suffix(".proxy").map(str::to_string))
        .filter(|s| !s.is_empty())
        .collect();
    sites.sort();
    sites.dedup();
    sites
}

/// Copies Claude Code files a box kept for itself before its folders were opened (sessions
/// started in the box) to the real folders, without replacing anything there.
fn recover_claude_code_data(root: &Path, paths: &[PathBuf]) {
    fn merge(from: &Path, to: &Path) {
        let Ok(entries) = std::fs::read_dir(from) else { return };
        for e in entries.flatten() {
            let target = to.join(e.file_name());
            match e.file_type() {
                Ok(t) if t.is_dir() => merge(&e.path(), &target),
                Ok(t) if t.is_file() && !target.exists() => {
                    let _ = std::fs::create_dir_all(to).and_then(|_| std::fs::copy(e.path(), &target));
                }
                _ => {}
            }
        }
    }
    for dir in paths.iter().filter(|p| !p.to_string_lossy().ends_with('*')) {
        if let Some(copy) = box_copy(root, dir) { merge(&copy, dir); }
    }
}

/// Edit only the physical copy of the value in the box hive. Removing that copy
/// restores Sandboxie's merged view of the current host policy.
#[cfg(windows)]
fn sync_box_proxy(dir: &Path, name: &str, port: Option<u16>) -> Result<(), String> {
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND};
    use windows_sys::Win32::System::Registry::*;
    let folder = crate::paths::storage_root().join("sandbox");
    let marker = folder.join(format!("{name}.proxy"));
    if port.is_none() && !marker.try_exists().map_err(|e| e.to_string())? { return Ok(()); }
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<_>>();
    let (status, _, path) = query_box_path(dir, name)?;
    let lower = path.to_ascii_lowercase();
    let hive = lower.strip_prefix(r"\registry\user\").filter(|s| !s.contains('\\') && s.starts_with("sandbox_") && s.ends_with(&format!("_{}", name.to_ascii_lowercase())));
    let hive = hive.filter(|_| status == 0).ok_or("无法确认账号沙箱的专用注册表位置。")?;
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
        let found = GetProcAddress(module, c"SbieApi_QueryProcess".as_ptr().cast()).and_then(|f| {
            let f: QueryProcess = std::mem::transmute(f);
            let (mut name, mut image, mut sid, mut session) = ([0u16; 34], [0u16; 96], [0u16; 96], 0u32);
            if f(pid as isize, name.as_mut_ptr(), image.as_mut_ptr(), sid.as_mut_ptr(), &mut session) != 0 { return None; }
            let len = name.iter().position(|c| *c == 0).unwrap_or(name.len());
            Some(String::from_utf16_lossy(&name[..len])).filter(|n| !n.is_empty())
        });
        // Called for every boxed process on every refresh: do not leak a module reference.
        windows_sys::Win32::Foundation::FreeLibrary(module);
        found
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
        let name = super::configure(id, &data.to_string_lossy(), None, None).unwrap();
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
        let name = super::configure(id, &data.to_string_lossy(), Some(port), None).unwrap();
        println!("box {name}, relay port {port}");

        // 1) network rules, through `command` (what launcher uses).
        let out = data.join("net.txt");
        let script = format!(
            "$r=@(); foreach($t in @(@('127.0.0.1',{port}),@('127.0.0.1',{other_port}),@('1.1.1.1',80),@('204.79.197.200',443))){{ try{{ $c=New-Object Net.Sockets.TcpClient; $c.Connect($t[0],$t[1]); $r+=\"$($t[0]):$($t[1])=open\"; $c.Close() }}catch{{ $r+=\"$($t[0]):$($t[1])=blocked\" }} }}; $n='h'+[guid]::NewGuid().ToString('N').Substring(0,10)+'.5.6.7.8.nip.io'; try{{ [void][Net.Dns]::GetHostAddresses($n); $r+='dns-external=resolved' }}catch{{ $r+='dns-external=failed' }}; try{{ [void](Resolve-DnsName example.org -Server 8.8.8.8 -DnsOnly -ErrorAction Stop); $r+='dns-server=resolved' }}catch{{ $r+='dns-server=failed' }}; try{{ [void][Net.Dns]::GetHostAddresses('localhost'); $r+='dns-localhost=ok' }}catch{{ $r+='dns-localhost=failed' }}; try{{ $l=New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback,0); $l.Start(); $c=New-Object Net.Sockets.TcpClient; $c.Connect('127.0.0.1',$l.LocalEndpoint.Port); $c.Close(); $l.Stop(); $r+='loopback-self=open' }}catch{{ $r+='loopback-self=blocked' }}; $r -join ' ' | Set-Content -Encoding ascii '{}'",
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
        assert!(net.contains("loopback-self=open"), "listeners inside the box must stay reachable: {net}");

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

    /// Real Sandboxie-Plus check of the five identifiers: `CLAUDE_HYDRA_ROOT=<scratch> cargo test --lib
    /// sandboxie_identity -- --ignored --nocapture`. System tools are used rather than a helper exe so
    /// the test also proves the hooks reach programs that bind straight to kernelbase.
    #[cfg(windows)]
    #[test]
    #[ignore = "requires Sandboxie-Plus and CLAUDE_HYDRA_ROOT pointing to a scratch folder"]
    fn sandboxie_identity_is_per_account() {
        use std::time::{Duration, Instant};
        assert!(std::env::var_os("CLAUDE_HYDRA_ROOT").is_some());
        let dir = super::available().unwrap();
        let data = crate::paths::storage_root().join("identity-test");
        std::fs::create_dir_all(&data).unwrap();
        let ids = crate::identity::Identifiers::for_account("hydra-identity-test");
        let name = super::configure("hydra-identity-test", &data.to_string_lossy(), None, None).unwrap();

        let out = data.join("ids.txt");
        let _ = std::fs::remove_file(&out);
        let script = data.join("ids.cmd");
        std::fs::write(
            &script,
            format!(
                "@echo off\r\n(\r\n\
                 reg query \"HKLM\\SOFTWARE\\Microsoft\\Cryptography\" /v MachineGuid\r\n\
                 reg query \"HKLM\\SYSTEM\\CurrentControlSet\\Control\\IDConfigDB\\Hardware Profiles\\0001\" /v HwProfileGuid\r\n\
                 hostname\r\n\
                 vol C:\r\n\
                 vol D:\r\n\
                 ipconfig /all\r\n\
                 echo HYDRA-ENUM\r\n\
                 reg query \"HKLM\\SOFTWARE\\Microsoft\\Cryptography\"\r\n\
                 echo HYDRA-WHOAMI\r\n\
                 whoami\r\n\
                 echo HYDRA-NODE\r\n\
                 node -e \"const o=require('os'),f=require('fs');console.log('NODE-HOST='+o.hostname());console.log('NODE-DEV-C='+f.statSync('C:/').dev);console.log('NODE-DEV-D='+f.statSync('D:/').dev);for(const l of Object.values(o.networkInterfaces()))for(const a of l)if(a.mac!=='00:00:00:00:00:00')console.log('NODE-MAC='+a.mac)\"\r\n\
                 echo HYDRA-DONE\r\n\
                 ) > \"{}\" 2>&1\r\n",
                out.display()
            ),
        )
        .unwrap();
        let cmd = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/cmd.exe");
        super::command("hydra-identity-test", &cmd, &format!("/d /c \"{}\"", script.display()), &[])
            .unwrap()
            .spawn()
            .unwrap();
        // The redirect creates the file before the four commands have run, so wait for the last
        // line instead of for the file itself. `vol` prints in the OEM code page, which is not
        // UTF-8, so the bytes are decoded lossily — every value asserted below is ASCII.
        let started = Instant::now();
        let mut text = String::new();
        while started.elapsed() < Duration::from_secs(60) {
            text = std::fs::read(&out).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
            if text.contains("HYDRA-DONE") { break; }
            std::thread::sleep(Duration::from_millis(300));
        }
        assert!(text.contains("HYDRA-DONE"), "boxed probe did not finish: {text}");
        println!("{text}");

        assert!(text.contains(&ids.machine_guid), "MachineGuid is still the host's: {text}");
        assert!(text.contains(&ids.hw_profile_guid), "HwProfileGuid is still the host's: {text}");
        assert!(text.contains(&ids.computer_name), "hostname is still the host's: {text}");
        // Each volume reports its own serial, not the account's single base value.
        // The derived serial is keyed on the volume's real one, read here outside the box.
        let host_serial = |drive: &str| -> u32 {
            let out = std::process::Command::new("cmd").args(["/d", "/c", "vol", drive]).output().unwrap();
            let text = String::from_utf8_lossy(&out.stdout).into_owned();
            let token = text
                .split_whitespace()
                .find(|w| w.len() == 9 && w.as_bytes()[4] == b'-' && w.chars().filter(|c| *c != '-').all(|c| c.is_ascii_hexdigit()))
                .unwrap_or_else(|| panic!("no serial in {text}"));
            u32::from_str_radix(&token.replace('-', ""), 16).unwrap()
        };
        let (host_c, host_d) = (host_serial("C:"), host_serial("D:"));
        let c_value = crate::identity::volume_serial_for(&ids, host_c);
        let d_value = crate::identity::volume_serial_for(&ids, host_d);
        let c = crate::identity::volume_serial_text(c_value);
        let d = crate::identity::volume_serial_text(d_value);
        assert!(!text.contains(&crate::identity::volume_serial_text(host_c)), "C: still reports the host serial: {text}");
        assert_ne!(c, d);
        assert!(text.contains(&c), "C: does not report the account's serial {c}: {text}");
        assert!(text.contains(&d), "D: does not report the account's serial {d}: {text}");

        // `ipconfig` prints the adapter addresses the guard handed to `GetAdaptersAddresses`:
        // upper case, colon replaced by a dash. Every adapter must start with the account's own
        // three-byte prefix, and the addresses must differ from each other — one shared address
        // for all adapters is what a naive override produces and it is not what a real PC looks
        // like.
        let macs: Vec<String> = text
            .split_whitespace()
            .filter(|w| w.len() == 17 && w.split('-').count() == 6 && w.chars().all(|c| c.is_ascii_hexdigit() || c == '-'))
            .map(|w| w.to_ascii_lowercase())
            .collect();
        let prefix = format!("{}-{}-{}", &ids.mac[..2], &ids.mac[2..4], &ids.mac[4..6]);
        assert!(macs.len() >= 4, "expected one address per adapter, got {macs:?}");
        for mac in &macs {
            assert!(mac.starts_with(&prefix), "adapter address {mac} is not the account's ({prefix}…): {text}");
        }
        assert_eq!(
            macs.iter().collect::<std::collections::BTreeSet<_>>().len(),
            macs.len(),
            "adapters share one address: {macs:?}"
        );

        // The host is untouched: the same read outside the box still gives the real value.
        let host = crate::sys::reg_string(true, r"SOFTWARE\Microsoft\Cryptography", "MachineGuid").unwrap();
        assert_ne!(host.to_lowercase(), ids.machine_guid);
        // Listing the key's values (no /v) goes through NtEnumerateValueKey, not a named query.
        let section = |from: &str, to: &str| text.split(from).nth(1).and_then(|r| r.split(to).next()).unwrap_or("").to_string();
        let listed = section("HYDRA-ENUM", "HYDRA-WHOAMI");
        assert!(listed.contains(&ids.machine_guid), "enumerated MachineGuid is not the account's: {listed}");
        assert!(!listed.to_lowercase().contains(&host.to_lowercase()), "enumeration leaks the host MachineGuid: {listed}");
        // `whoami` prints `DOMAIN\user`; for a local account the domain is the computer name.
        let who = section("HYDRA-WHOAMI", "HYDRA-NODE").to_lowercase();
        let host_name = std::env::var("COMPUTERNAME").unwrap().to_lowercase();
        assert!(who.contains(&format!("{}\\", ids.computer_name.to_lowercase())), "whoami domain is not the account's: {who}");
        assert!(!who.contains(&format!("{host_name}\\")), "whoami still names the host: {who}");
        // Claude Code is a Node program: what Node reads is what matters most.
        let node = section("HYDRA-NODE", "HYDRA-DONE");
        if node.contains("NODE-HOST=") {
            let value = |key: &str| node.lines().find_map(|l| l.trim().strip_prefix(key).map(str::to_owned)).unwrap_or_default();
            assert_eq!(value("NODE-HOST="), ids.computer_name, "os.hostname(): {node}");
            assert_eq!(value("NODE-DEV-C="), c_value.to_string(), "fs.stat('C:').dev: {node}");
            assert_eq!(value("NODE-DEV-D="), d_value.to_string(), "fs.stat('D:').dev: {node}");
            let prefix = format!("{}:{}:{}", &ids.mac[..2], &ids.mac[2..4], &ids.mac[4..6]);
            let node_macs: Vec<&str> = node.lines().filter_map(|l| l.trim().strip_prefix("NODE-MAC=")).collect();
            assert!(!node_macs.is_empty(), "os.networkInterfaces() listed no adapter: {node}");
            for mac in &node_macs {
                assert!(mac.starts_with(&prefix), "os.networkInterfaces() mac {mac} is not the account's: {node}");
            }
        } else {
            println!("node is not on PATH: the Node checks were skipped");
        }
        assert_eq!(crate::identity::Identifiers::for_account("hydra-identity-test"), ids);
        assert_ne!(crate::identity::Identifiers::for_account("other-account"), ids);
        let _ = std::process::Command::new(dir.join("Start.exe")).args([&format!("/box:{name}"), "/terminate"]).status();
    }

    /// Regression for the bug where `set_injection` joined the two guard paths with a comma.
    /// Sandboxie takes one path per value and does not split on commas, so the joined string
    /// named a file that does not exist and *neither* guard loaded. It looked healthy in the ini
    /// (the key had a value), which is why this asserts the line count instead.
    #[cfg(windows)]
    #[test]
    #[ignore = "requires Sandboxie-Plus"]
    fn every_guard_is_its_own_injection_line() {
        let dir = super::available().unwrap();
        use std::os::windows::process::CommandExt;
        let id = "hydra-injection-test";
        let name = super::configure(id, &crate::paths::storage_root().join("injection-test").to_string_lossy(), None, None).unwrap();
        let count = |key: &str| {
            let out = std::process::Command::new(dir.join("SbieIni.exe")).args(["query", &name, key]).creation_flags(0x0800_0000).output().unwrap();
            assert!(out.status.success(), "query {key} failed");
            String::from_utf8_lossy(&out.stdout).lines().filter(|l| !l.trim().is_empty()).map(str::to_owned).collect::<Vec<_>>()
        };
        let wide = count("InjectDll64");
        let narrow = count("InjectDll");
        println!("InjectDll64 {wide:?}\nInjectDll {narrow:?}");
        // Two guards per bitness. A comma-joined value collapses these to one line each.
        assert_eq!(wide.len(), 2, "InjectDll64 must list each guard separately: {wide:?}");
        assert_eq!(narrow.len(), 2, "InjectDll must list each guard separately: {narrow:?}");
        for path in wide.iter().chain(narrow.iter()) {
            assert!(std::path::Path::new(path).is_file(), "guard was not materialised: {path}");
            assert!(!path.contains(','), "guard paths must not be comma-joined: {path}");
        }
        let _ = std::process::Command::new(dir.join("Start.exe")).args([&format!("/box:{name}"), "/silent", "/terminate"]).status();
    }

    #[test]
    fn box_gitconfig_overrides_every_proxy_after_the_host_file() {
        let host = std::path::Path::new(r"C:\Users\u\.gitconfig");
        let original = "[http]\n\tproxy = http://127.0.0.1:7891\n[http \"https://gitee.com\"]\n\tproxy =";
        let out = super::gitconfig_for_box(host, original, &["https://gitee.com".into(), "https://a\"b".into()], 4321);
        assert!(out.starts_with(super::GITCONFIG_MARK));
        let (head, tail) = out.split_once("proxy =\n").unwrap();
        assert!(head.contains("127.0.0.1:7891"));
        assert!(tail.contains("[http]\n\tproxy = http://127.0.0.1:4321\n"));
        assert!(tail.contains("[http \"https://gitee.com\"]\n\tproxy = http://127.0.0.1:4321\n"));
        assert!(tail.contains(r#"[http "https://a\"b"]"#));
    }

    #[test]
    fn box_copies_map_profile_and_drives() {
        let root = std::path::Path::new(r"C:\Sandbox\u\Hydra_x");
        if let Some(home) = super::home() {
            assert_eq!(super::box_copy(root, &home.join(".claude")), Some(root.join("user").join("current").join(".claude")));
        }
        if cfg!(windows) {
            assert_eq!(super::box_copy(root, std::path::Path::new(r"q:\data\x")), Some(root.join("drive").join("Q").join(r"data\x")));
        }
    }

    #[test]
    fn box_names_are_valid_and_stable() {
        let a = super::box_name("Account 2");
        assert_eq!(a, super::box_name("Account 2"));
        assert_ne!(a, super::box_name("Account 3"));
        assert!(a.len() <= 32 && a.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'), "{a}");
    }
}
