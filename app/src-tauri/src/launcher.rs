//! Account process lifecycle, native desktop shortcuts, and diagnostic reports.

use crate::install::Install;
use crate::profiles::{self, path_key, Profile};
use crate::{paths, settings, sys};
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

pub fn launch(install: &Install, p: &Profile) -> Result<&'static str, String> {
    let running = profiles::running_map(install).get(&path_key(&p.path)).copied();
    if let Some(pid) = running {
        #[cfg(windows)]
        crate::taskbar::apply(p, pid);
        if sys::focus(pid) {
            return Ok("focused");
        }
    }

    let s = settings::load();
    let outcome = if running.is_some() { "focused" } else { "started" };
    let effective = s.effective(&p.id);
    let node = effective.node.clone();
    let extra = profiles::extra_args(&s, &p.id);
    if let Some(f) = crate::node::forbidden_switch(&extra) {
        return Err(format!("Claude 不允许使用启动参数 {f}，带上它会直接退出。请在环境配置的额外启动参数中移除。"));
    }
    let mut env = profiles::env_vars(&s, &p.id);
    // Hydra-managed switches; passed as separate arguments (some contain spaces).
    let mut managed: Vec<String> = Vec::new();
    let mut timezone = match effective.timezone.as_str() { "" | "proxy" | "system" => None, tz => Some(tz.to_string()) };
    if (node.is_some() || timezone.is_some()) && p.is_default && install.is_msix() {
        return Err("商店版默认账号无法应用环境配置中的代理节点或时区。".into());
    }
    if let Some(node) = &node {
        if crate::node::args_conflict(&extra) { return Err("额外启动参数中的代理开关与代理节点冲突，请先移除。".into()); }
        // Claude pins itself to a managed egress proxy over `--proxy-server`, so the account
        // would leave through that proxy (e.g. a local Clash port) instead of its node.
        if let Some(policy) = crate::sys::claude_egress_policy() {
            return Err(format!(
                "账号未启动：系统策略为 Claude 固定了出口代理 {policy}（注册表 HKCU 或 HKLM 下 Software\\Policies\\Claude 的 egressProxyUrl）。Claude 会忽略 Hydra 的代理节点、改走这个代理。请删除该策略值后重试；不使用代理节点的账号不受影响。"
            ));
        }
        let port = crate::relay::port_for_launch(&p.id, node).map_err(|e| format!("账号未启动：{e}"))?;
        // Verify the exact route Claude will use: local relay -> node -> internet.
        let client = crate::node::relay_client(port, std::time::Duration::from_secs(15))?;
        let exit = crate::node::inspect(&client, node).map_err(|e| format!("账号未启动：{e}"))?;
        if matches!(effective.timezone.as_str(), "" | "proxy") {
            if exit.timezone.is_empty() {
                return Err("账号未启动：IP 查询服务没有返回出口时区。请换一个 IP 查询渠道，或在账号设置中手动指定时区。".into());
            }
            timezone = Some(exit.timezone.clone());
        }
        let _ = settings::update(|s| s.account_mut(&p.id).last_exit = Some(exit));
        managed = crate::node::chromium_args(port);
        env.retain(|(k, _)| !crate::node::is_proxy_env(k));
        env.extend(crate::node::child_env(port));
    }
    if let Some(tz) = timezone {
        env.retain(|(k, _)| !k.eq_ignore_ascii_case("TZ"));
        env.push(("TZ".into(), tz));
    }
    if p.is_default && install.is_msix() {
        let aumid = install.aumid.as_deref().ok_or("缺少商店版应用标识。")?;
        Command::new("explorer.exe")
            .arg(format!("shell:AppsFolder\\{aumid}"))
            .spawn()
            .map_err(|e| format!("无法启动 Claude：{e}"))?;
        return Ok(outcome);
    }

    let path = PathBuf::from(&p.path);
    if !p.is_default {
        std::fs::create_dir_all(&path).map_err(|e| format!("无法创建账号目录：{e}"))?;
    }

    #[cfg(target_os = "macos")]
    {
        let app = install.aumid.clone().ok_or("找不到 Claude.app。")?;
        let mut cmd = Command::new("/usr/bin/open");
        cmd.args(["-n", "-a", &app]);
        for (k, v) in &env {
            cmd.args(["--env", &format!("{k}={v}")]);
        }
        let mut pass: Vec<String> = Vec::new();
        if !p.is_default {
            pass.push(format!("--user-data-dir={}", p.path));
        }
        pass.extend(managed);
        pass.extend(extra.split_whitespace().map(str::to_string));
        if !pass.is_empty() {
            cmd.arg("--args").args(pass);
        }
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
        let status = cmd.status().map_err(|e| format!("无法启动 Claude：{e}"))?;
        if !status.success() { return Err("无法启动 Claude（open 返回错误）。".into()); }
        // `open` returns once the app is launched; Claude may still quit right away (e.g. it
        // rejects a switch). Report that instead of a false "started".
        return confirm_running(install, p).map(|_| outcome);
    }

    #[cfg(not(target_os = "macos"))]
    {
        let exe = install
            .launch_exe()
            .ok_or("找不到 Claude 的启动程序。Claude 可能正在更新，请稍后再试。")?;
        let mut line = Vec::new();
        if !p.is_default {
            line.push(format!("--user-data-dir=\"{}\"", p.path));
        }
        line.extend(managed.into_iter().map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a }));
        if !extra.is_empty() {
            line.push(extra);
        }
        let mut cmd = Command::new(&exe);
        #[cfg(windows)]
        cmd.raw_arg(line.join(" "));
        #[cfg(not(windows))]
        cmd.args(line);
        cmd.envs(env).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        let child = cmd.spawn().map_err(|e| format!("无法启动 Claude（{}）：{e}", exe.display()))?;
        #[cfg(windows)]
        if !p.is_default {
            for _ in 0..20 {
                if crate::taskbar::apply(p, child.id()) || !sys::is_alive(child.id()) { break; }
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
        }
        drop(child);
        Ok(outcome)
    }
}

fn family(main: u32) -> Vec<u32> {
    let all = sys::processes();
    let mut out = vec![main];
    let mut i = 0;
    while i < out.len() {
        let parent = out[i];
        for p in &all {
            if p.ppid == parent && p.pid != parent && !out.contains(&p.pid) {
                out.push(p.pid);
            }
        }
        i += 1;
    }
    out
}

pub fn close(install: &Install, p: &Profile) -> Result<&'static str, String> {
    let Some(main) = profiles::running_map(install).get(&path_key(&p.path)).copied() else {
        return Ok("not running");
    };
    let fam = family(main);
    sys::close_windows(main);
    sys::wait_exit(main, 3000);
    for pid in fam.iter().rev() {
        sys::terminate(*pid);
    }
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < until && sys::is_alive(main) {
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    if sys::is_alive(main) {
        return Err(format!(
            "系统拒绝结束“{}”（进程 {main}）。它可能以更高权限运行，请从其窗口或托盘图标手动退出。",
            p.name
        ));
    }
    Ok("closed")
}

pub fn restart(install: &Install, p: &Profile) -> Result<&'static str, String> {
    close(install, p)?;
    std::thread::sleep(std::time::Duration::from_millis(400));
    launch(install, p)
}

/// Waits for the account's Claude to appear and stay up for a moment.
#[cfg(target_os = "macos")]
fn confirm_running(install: &Install, p: &Profile) -> Result<(), String> {
    let key = path_key(&p.path);
    let pid = || profiles::running_map(install).get(&key).copied();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(8);
    while std::time::Instant::now() < until {
        if pid().is_some() {
            std::thread::sleep(std::time::Duration::from_millis(1500));
            if pid().is_some() { return Ok(()); }
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    Err("Claude 启动后立即退出了。请检查环境配置中的额外启动参数，或先关闭该账号后重试。".into())
}

pub fn add_account(install: &Install) -> Result<Profile, String> {
    let created = profiles::create(install, &profiles::next_account_name(install))?;
    launch(install, &created)?;
    Ok(created)
}

/// Creates a native desktop shortcut pointing directly to the Claude-Hydra executable.
pub fn create_shortcut(p: &Profile) -> Result<PathBuf, String> {
    let desktop = paths::desktop_dir().ok_or("无法定位桌面目录。")?;
    std::fs::create_dir_all(&desktop).map_err(|e| format!("无法访问桌面目录：{e}"))?;
    let exe = std::env::current_exe().map_err(|e| format!("无法获取当前程序路径：{e}"))?;
    let ext = if cfg!(windows) { "lnk" } else { "command" };
    let safe_name: String = p
        .name
        .chars()
        .map(|c| if "\\/:*?\"<>|".contains(c) { '_' } else { c })
        .collect();
    let lnk_path = desktop.join(format!("Claude - {safe_name}.{ext}"));
    let args = format!("--launch \"{}\"", p.id);
    let desc = format!("启动 Claude-Hydra 账号：{}", p.name);
    sys::write_shortcut(&lnk_path, &exe, &args, &desc)?;
    #[cfg(windows)]
    crate::taskbar::shortcut(p, &lnk_path)?;
    Ok(lnk_path)
}

fn redact_user(s: &str) -> String {
    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default();
    if user.len() >= 2 {
        s.replace(&user, "<user>")
    } else {
        s.to_string()
    }
}

/// Generates a redacted diagnostic report in pure Rust.
pub fn diagnostics(install_res: Result<Install, String>) -> String {
    let mut lines = Vec::new();
    lines.push(format!("Claude-Hydra v{} ({})", env!("CARGO_PKG_VERSION"), std::env::consts::OS));
    lines.push(format!("Storage Root: {}", redact_user(&paths::storage_root().to_string_lossy())));
    lines.push(format!("Settings File: {}", redact_user(&paths::settings_path().to_string_lossy())));
    lines.push(format!(
        "Claude Egress Policy: {}",
        sys::claude_egress_policy().unwrap_or_else(|| "none".into())
    ));
    match install_res {
        Ok(inst) => {
            lines.push(format!("Claude Install Kind: {}", inst.kind));
            if let Some(exe) = &inst.exe {
                lines.push(format!("Claude Exe: {}", redact_user(&exe.to_string_lossy())));
            }
            if let Some(fam) = &inst.family {
                lines.push(format!("Claude Package Family: {fam}"));
            }
            lines.push(format!(
                "Default Profile: {}",
                redact_user(&inst.default_profile.to_string_lossy())
            ));
            lines.push(String::new());
            lines.push("Accounts:".into());
            for p in profiles::list(&inst, true) {
                let status = p.pid.map(|pid| format!("running (pid {pid})")).unwrap_or_else(|| "stopped".into());
                lines.push(format!(
                    "  - {} [{}]: {} | keep_running={} | path={}",
                    p.name,
                    p.id,
                    status,
                    p.keep_running,
                    redact_user(&p.path)
                ));
            }
        }
        Err(e) => {
            lines.push(format!("Claude Install Error: {}", redact_user(&e)));
        }
    }
    let err_log = paths::read_log_tail("hydra-error.log", 4096);
    if !err_log.trim().is_empty() {
        lines.push(String::new());
        lines.push("Recent Errors:".into());
        lines.push(redact_user(err_log.trim()));
    }
    lines.join("\n")
}
