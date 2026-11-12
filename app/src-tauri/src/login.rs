//! One-shot routing of browser sign-in callbacks to an explicitly selected profile.
use crate::{install, paths, profiles};
use std::process::Command;
#[cfg(windows)]
use std::os::windows::process::CommandExt;

fn marker() -> std::path::PathBuf { paths::storage_root().join("pending-login.json") }

/// Marks `id` as the account the next sign-in callback belongs to (valid for 10 minutes).
/// `signed_in` is the account's state now, so a completed sign-in can be recognised later.
pub fn arm(id: &str, signed_in: bool) -> Result<(), String> {
    std::fs::create_dir_all(paths::storage_root()).map_err(|e| e.to_string())?;
    let data = serde_json::json!({"id": id, "expires": paths::now_secs() + 600, "signedIn": signed_in});
    std::fs::write(marker(), data.to_string()).map_err(|e| format!("无法准备登录：{e}"))
}

/// The account a sign-in is pending for, without consuming it.
pub fn pending() -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(marker()).ok()?).ok()?;
    (v["expires"].as_u64()? >= paths::now_secs()).then(|| v["id"].as_str().map(str::to_owned)).flatten()
}

/// Hands a `claude://` link straight to account `id` (from its account browser), dropping a
/// sign-in pending for it.
#[cfg(not(target_os = "macos"))]
pub fn deliver(id: &str, url: &str) -> Result<(), String> {
    if !valid_url(url) { return Err("无效的 Claude 回调地址。".into()); }
    let i = install::resolve()?;
    let p = profiles::resolve(&i, id, false).ok_or("登录目标账号已不存在。")?;
    if pending().as_deref() == Some(id) { let _ = std::fs::remove_file(marker()); }
    relaunch_with_url(&i, &p, url)
}

fn valid_url(url: &str) -> bool {
    url.starts_with("claude://") && url.len() <= 16384 && !url.chars().any(|c| c.is_control() || c.is_whitespace() || c == '"')
}

/// Consumes the pending sign-in and returns the account id it was armed for.
/// The marker is taken before anything is launched so a callback cannot be replayed.
fn claim() -> Result<String, String> {
    let path = marker();
    let claimed = path.with_extension(format!("claimed-{}", std::process::id()));
    std::fs::rename(&path, &claimed).map_err(|_| "没有待处理的登录请求或请求已被处理。请在账号菜单中重新选择登录。".to_string())?;
    let bytes = std::fs::read(&claimed).map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&claimed);
    let data: serde_json::Value = serde_json::from_slice(&bytes?).map_err(|_| "登录请求数据无效。")?;
    let expiry = data["expires"].as_u64().ok_or("登录请求数据无效。")?;
    if paths::now_secs() > expiry { return Err("登录请求已过期，请重新选择登录。".into()); }
    data["id"].as_str().map(str::to_owned).ok_or_else(|| "登录请求数据无效。".to_string())
}

pub fn route(url: &str) -> Result<(), String> {
    if !valid_url(url) { return Err("无效的 Claude 回调地址。".into()); }
    let id = claim()?;
    let i = install::resolve()?;
    let p = profiles::resolve(&i, &id, false).ok_or("登录目标账号已不存在。")?;
    #[cfg(target_os = "macos")]
    { mac::forward(&i, &p, url) }
    #[cfg(not(target_os = "macos"))]
    { relaunch_with_url(&i, &p, url) }
}

/// Windows: a second Claude process with the same data dir hands its argv (with the URL) to
/// the running instance through Electron's single-instance lock.
/// Only a running instance is targeted: a fresh start from here would miss the account's
/// proxy switches.
#[cfg(not(target_os = "macos"))]
fn relaunch_with_url(i: &install::Install, p: &profiles::Profile, url: &str) -> Result<(), String> {
    if !profiles::running_map(i).contains_key(&profiles::path_key(&p.path)) {
        return Err(format!("“{}”的 Claude 没有在运行。请先启动这个账号，再重新登录。", p.name));
    }
    let settings = crate::settings::load();
    let sandboxed = settings.effective(&p.id).isolation == "sandboxie";
    let exe = if sandboxed { crate::sandbox::launch_exe(i)? } else { i.launch_exe().ok_or("找不到 Claude 启动程序。")? };
    let mut line = Vec::new();
    if !p.is_default { line.push(format!("--user-data-dir=\"{}\"", p.path)); }
    let extra = profiles::extra_args(&settings, &p.id);
    if !extra.is_empty() { line.push(extra); }
    line.push(format!("\"{url}\""));
    let env = profiles::env_vars(&settings, &p.id);
    // A sandboxed account's Claude only sees a second instance started in the same box.
    let mut cmd = if sandboxed {
        crate::sandbox::command(&p.id, &exe, &line.join(" "), &env)?
    } else {
        let mut cmd = Command::new(exe);
        #[cfg(windows)]
        cmd.raw_arg(line.join(" "));
        #[cfg(not(windows))]
        cmd.args(line.iter().map(|a| a.trim_matches('"')));
        cmd.envs(env);
        cmd
    };
    cmd.spawn().map_err(|e| format!("无法转发登录回调：{e}"))?;
    Ok(())
}

/// Registers Hydra for `claude://` (sign-in callbacks) and as a browser for http/https, so the
/// links Claude opens (Google sign-in) reach the account browser on the account's own exit.
/// Windows only lets the user pick the defaults, so the Default Apps page is opened. The
/// current https browser is remembered and receives every link that is not for Claude.
#[cfg(windows)]
pub fn register() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    crate::browser::remember_fallback();
    let base = r"HKCU\Software\Classes\ClaudeHydra.Callback";
    let url_class = r"HKCU\Software\Classes\ClaudeHydra.URL";
    let capabilities = r"Software\ClaudeHydra\Capabilities";
    let client = r"HKCU\Software\Clients\StartMenuInternet\ClaudeHydra";
    let command = format!("\"{}\" --login-callback \"%1\"", exe.display());
    let open_url = format!("\"{}\" --open-url \"%1\"", exe.display());
    let mut entries = vec![
        (base.to_string(), "/ve", "Claude-Hydra 登录回调".to_string()),
        (base.to_string(), "/v URL Protocol", String::new()),
        (format!(r"{base}\shell\open\command"), "/ve", command),
        (url_class.to_string(), "/ve", "Claude-Hydra 链接".to_string()),
        (url_class.to_string(), "/v URL Protocol", String::new()),
        (format!(r"{url_class}\DefaultIcon"), "/ve", format!("\"{}\",0", exe.display())),
        (format!(r"{url_class}\shell\open\command"), "/ve", open_url),
        (format!(r"HKCU\{capabilities}"), "/v ApplicationName", "Claude-Hydra".into()),
        (format!(r"HKCU\{capabilities}"), "/v ApplicationDescription", "把 Claude 打开的登录链接交给对应账号的专属浏览器，其余链接转给原来的浏览器。".into()),
        (format!(r"HKCU\{capabilities}\URLAssociations"), "/v claude", "ClaudeHydra.Callback".into()),
        (r"HKCU\Software\RegisteredApplications".into(), "/v ClaudeHydra", capabilities.into()),
        (client.to_string(), "/ve", "Claude-Hydra".into()),
        (format!(r"{client}\DefaultIcon"), "/ve", format!("\"{}\",0", exe.display())),
        (format!(r"{client}\shell\open\command"), "/ve", format!("\"{}\"", exe.display())),
        (format!(r"{client}\Capabilities"), "/v ApplicationName", "Claude-Hydra".into()),
        (format!(r"{client}\Capabilities"), "/v ApplicationDescription", "Claude-Hydra 账号浏览器路由".into()),
    ];
    for scheme in ["http", "https"] {
        entries.push((format!(r"HKCU\{capabilities}\URLAssociations"), if scheme == "http" { "/v http" } else { "/v https" }, "ClaudeHydra.URL".into()));
        entries.push((format!(r"{client}\Capabilities\URLAssociations"), if scheme == "http" { "/v http" } else { "/v https" }, "ClaudeHydra.URL".into()));
    }
    for (key, name, value) in entries {
        let mut cmd = Command::new("reg.exe");
        cmd.arg("add").arg(key);
        if name == "/ve" { cmd.arg("/ve"); } else { cmd.args(["/v", name.trim_start_matches("/v ")]); }
        cmd.creation_flags(0x0800_0000);
        let status = cmd.args(["/t", "REG_SZ", "/d", &value, "/f"]).status().map_err(|e| e.to_string())?;
        if !status.success() { return Err("无法注册登录回调程序。".into()); }
    }
    Command::new("explorer.exe").arg("ms-settings:defaultapps").spawn().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(not(windows))]
pub fn register() -> Result<(), String> { Err("登录回调路由目前仅支持 Windows。".into()) }

/// macOS: Hydra declares the `claude` scheme in its Info.plist but is its default handler only
/// while a sign-in is pending. That has two effects:
/// - Claude starts Google sign-in in an ASWebAuthenticationSession whenever it is *not* the
///   `claude://` handler, so the callback returns straight into the signing-in process;
/// - any callback that still goes through LaunchServices (fallback, magic link, SSO) reaches
///   Hydra, which sends it as a `GURL` Apple event to that account's Claude process by pid.
/// Claude claims the handler again whenever its main window opens, so Hydra re-claims it for
/// a while after launching, and hands it back once the sign-in is done, expired or Hydra exits.
#[cfg(target_os = "macos")]
pub mod mac {
    use super::*;
    use crate::{install::Install, mac_url, profiles::Profile};
    use std::time::{Duration, Instant};

    const SCHEME: &str = "claude";
    const CLAUDE_ID: &str = "com.anthropic.claudefordesktop";

    fn own_id() -> Option<String> { mac_url::own_bundle_id() }

    /// Claude's bundle id, read from the installed app (falls back to the known id).
    fn claude_id(i: &Install) -> String {
        i.aumid.as_deref()
            .and_then(|app| Command::new("/usr/bin/plutil").args(["-extract", "CFBundleIdentifier", "raw", "-o", "-"]).arg(format!("{app}/Contents/Info.plist")).output().ok())
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| CLAUDE_ID.into())
    }

    fn we_handle() -> bool {
        matches!((own_id(), mac_url::default_handler(SCHEME)), (Some(o), Some(d)) if o.eq_ignore_ascii_case(&d))
    }

    /// Gives `claude://` back to Claude if Hydra currently holds it.
    pub fn restore(i: &Install) {
        if we_handle() { let _ = mac_url::set_default_handler(SCHEME, &claude_id(i)); }
    }

    /// Takes `claude://` while a sign-in is pending, re-claiming it after Claude's own startup claim.
    pub fn hold_during_login() -> Result<(), String> {
        let own = own_id().ok_or("请从 Claude-Hydra.app 启动 Hydra 后再登录。")?;
        mac_url::set_default_handler(SCHEME, &own)?;
        std::thread::spawn(move || {
            let until = Instant::now() + Duration::from_secs(60);
            while Instant::now() < until && marker().exists() {
                std::thread::sleep(Duration::from_millis(500));
                if mac_url::default_handler(SCHEME).is_some_and(|d| !d.eq_ignore_ascii_case(&own)) {
                    let _ = mac_url::set_default_handler(SCHEME, &own);
                }
            }
        });
        Ok(())
    }

    /// A freshly created account opens straight on Claude's sign-in screen: route its callback.
    pub fn expect_sign_in(id: &str) -> Result<(), String> {
        arm(id, false)?;
        hold_during_login()
    }

    /// Delivers a claimed callback to the account's running Claude, then releases the scheme.
    pub fn forward(i: &Install, p: &Profile, url: &str) -> Result<(), String> {
        let pid = profiles::running_map(i).get(&profiles::path_key(&p.path)).copied();
        let result = match pid {
            Some(pid) => mac_url::open_url_in(pid, url),
            None => Err(format!("“{}”的 Claude 已不在运行，请重新选择登录。", p.name)),
        };
        restore(i);
        result
    }

    /// Sign-in callbacks among `claude://` links (Claude's own login/SSO/magic-link routes).
    pub(super) fn is_sign_in_url(url: &str) -> bool {
        let Ok(u) = url::Url::parse(url) else { return false };
        let path = u.path();
        ["/login/", "/sso-callback", "/magic-link", "/partner-callback"].iter().any(|p| path.starts_with(p))
    }

    /// A `claude://` link arrived while Hydra holds the scheme.
    pub fn handle(url: &str) -> Result<(), String> {
        if !valid_url(url) { return Err("无效的 Claude 回调地址。".into()); }
        if is_sign_in_url(url) && marker().exists() { return route(url); }
        // An ordinary link (or no sign-in pending): give it to the default account's Claude,
        // never to the account that is signing in.
        let i = install::resolve()?;
        let default_pid = profiles::running_map(&i).get(&profiles::path_key(&i.default_profile.to_string_lossy())).copied();
        if let Some(pid) = default_pid { return mac_url::open_url_in(pid, url); }
        restore(&i);
        Command::new("/usr/bin/open").args(["-b", &claude_id(&i), url]).status()
            .map_err(|e| format!("无法打开链接：{e}"))
            .and_then(|s| if s.success() { Ok(()) } else { Err("无法把链接交给 Claude。".into()) })
    }

    /// The account a sign-in callback must belong to when nobody chose one: the only extra
    /// account that is running and not signed in. Ambiguous or none → `None`.
    pub(super) fn auto_target(list: &[Profile]) -> Option<&str> {
        let mut waiting = list.iter().filter(|p| !p.is_default && p.pid.is_some() && p.signed_in != Some(true));
        let first = waiting.next()?;
        waiting.next().is_none().then_some(first.id.as_str())
    }

    /// Periodic housekeeping, every few seconds:
    /// - end a pending sign-in once it completed (e.g. in-process via ASWebAuth), its account
    ///   quit, or it expired;
    /// - with none pending, arm one automatically for the only running signed-out extra account,
    ///   so signing in there just works without "准备浏览器登录";
    /// - hold `claude://` exactly while a sign-in is pending (Claude re-claims it at window
    ///   creation), and hand it back otherwise.
    pub fn maintain(i: &Install, list: &[Profile]) {
        let pending = std::fs::read(marker()).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
        if let Some(v) = &pending {
            let target = v["id"].as_str().and_then(|id| list.iter().find(|p| p.id == id));
            let expired = v["expires"].as_u64().map_or(true, |e| paths::now_secs() > e);
            let was = v["signedIn"].as_bool().unwrap_or(false);
            let now_in = target.and_then(|p| p.signed_in) == Some(true);
            let gone = target.map_or(true, |p| p.pid.is_none()) && v["auto"].as_bool().unwrap_or(false);
            if expired || (!was && now_in) || gone { let _ = std::fs::remove_file(marker()); }
        }
        if !marker().exists() {
            if let Some(id) = auto_target(list) {
                if let Ok(data) = serde_json::to_vec(&serde_json::json!({"id": id, "expires": paths::now_secs() + 600, "signedIn": false, "auto": true})) {
                    let _ = std::fs::write(marker(), data);
                }
            }
        }
        if marker().exists() {
            if let Some(own) = own_id() {
                if !we_handle() { let _ = mac_url::set_default_handler(SCHEME, &own); }
            }
        } else {
            restore(i);
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    #[test]
    fn mac_sign_in_routing_rules() {
        use super::mac::{auto_target, is_sign_in_url};
        use crate::profiles::Profile;
        assert!(is_sign_in_url("claude://claude.ai/login/app-google-auth?code=x"));
        assert!(is_sign_in_url("claude://claude.ai/magic-link#abc"));
        assert!(!is_sign_in_url("claude://claude.ai/new?surface=chat"));
        assert!(!is_sign_in_url("claude://code/new"));
        let p = |id: &str, default: bool, pid: Option<u32>, signed: Option<bool>| Profile { id: id.into(), is_default: default, pid, signed_in: signed, ..Default::default() };
        let one = [p("Default", true, Some(1), Some(false)), p("2", false, Some(2), Some(false)), p("3", false, None, Some(false)), p("4", false, Some(4), Some(true))];
        assert_eq!(auto_target(&one), Some("2"));
        let two = [p("2", false, Some(2), None), p("5", false, Some(5), Some(false))];
        assert_eq!(auto_target(&two), None);
        assert_eq!(auto_target(&[]), None);
    }

    #[test]
    fn rejects_invalid_links() {
        assert!(super::valid_url("claude://sso-callback?code=abc"));
        assert!(!super::valid_url("https://example.com"));
        assert!(!super::valid_url("claude://sso-callback?x=\"bad"));
    }
}
