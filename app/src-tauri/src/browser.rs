//! Account browser: a browser per account that leaves through the account's relay, with the
//! account's timezone and language, so web sign-in (Google, or a container's `claude /login`
//! link) happens on the same exit IP and with the same time zone as the account's Claude.
//! It is a Chromium browser the user has installed (Chrome, Edge, Brave or Chromium; each account
//! may choose), run as its own instance and driven over the DevTools protocol (`chromium`): a
//! real browser with its own window, address bar and tabs, which pages cannot tell from one opened
//! by hand (an embedded WebView2 names itself in `navigator.userAgentData`).
//!
//! - Network: the same Chromium switches as Claude (SOCKS5 to the relay, names resolved at the
//!   node, no QUIC, WebRTC limited to proxied paths). Exit rules are checked before it opens,
//!   and the relay's kill switch applies to it as well.
//! - Time zone: set before the first page loads and checked after every load; a page that
//!   reports another zone is replaced by a warning.
//! - Storage: its own profile folder per account and browser (cookies never mix between accounts).
//! - Page fingerprint: the browser's own. Canvas, WebGL, audio, screen and the rest stay real,
//!   because any altered reading is something fingerprint checks can find; only the automation
//!   flag that remote debugging raises is answered as in a browser opened by hand (`PAGE_JS`).
//! - Windows: `claude://` links (the sign-in callback) go straight to the account's Claude instead
//!   of to the system's handler, usually the default account's Claude. (macOS routes them through
//!   the scheme Hydra holds while a sign-in is pending, see `login::mac`.)
//!
//! Link routing (`--open-url`, Hydra registered as a browser): links that Claude opens for
//! claude.ai / anthropic.com go to the opening account's browser; everything else goes to the
//! browser the user had before.
use crate::{paths, settings, sys};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::time::Duration;

/// Where an account's browser starts: a page that shows the exit IP, its location, the time
/// zone, WebRTC and the other channels a page can read, as sites see them from this browser.
const START_URL: &str = "https://who.1ms.dog/zh-cn/";
#[cfg(windows)]
const URL_PROGID: &str = "ClaudeHydra.URL";

fn hex_id(id: &str) -> String { crate::sandbox::box_name(id).trim_start_matches("Hydra_").to_string() }

fn data_dir(id: &str) -> std::path::PathBuf { paths::storage_root().join("account-browser").join(hex_id(id)) }

/// Closes a deleted account's browser and removes its data (cookies, sign-in).
pub fn forget(id: &str) {
    crate::chromium::close(id);
    let _ = std::fs::remove_dir_all(data_dir(id));
}

/// The script the account browser runs in every document (every frame, its own realm included)
/// before the page's own scripts. A browser with remote debugging on (how Hydra sets the zone and
/// this script up) reports `navigator.webdriver` true, which pages read as a browser driven by a
/// program; the account's own user is at the keyboard, so it reads false, as in any browser opened
/// by hand. Nothing else is changed.
///
/// The replacements are written as methods (no `prototype`, not constructible, native parameter
/// count) and `Function.prototype.toString` reports the browser's own source text for them. In a frame it also
/// asks the parent's, so reading it through a fresh iframe's `toString` shows nothing either.
/// Every step falls back to the untouched browser on failure.
pub(crate) const PAGE_JS: &str = r#"(function () {
  'use strict';
  var realToString = Function.prototype.toString;
  var natives = new WeakMap();
  var parentToString = null;
  try { if (window.parent !== window) { parentToString = window.parent.Function.prototype.toString; } } catch (e) {}
  // A method, not a Proxy: a proxied toString is itself what stealth checks look for.
  var toStringHook = {
    toString() {
      var text = natives.get(this);
      if (text !== undefined) { return text; }
      if (parentToString && typeof this === 'function') { try { return parentToString.call(this); } catch (e) {} }
      return Reflect.apply(realToString, this, arguments);
    }
  }.toString;
  try {
    natives.set(toStringHook, realToString.call(realToString));
    Function.prototype.toString = toStringHook;
  } catch (e) {}
  try {
    var navigatorProto = window.Navigator && Navigator.prototype;
    var webdriver = navigatorProto && Object.getOwnPropertyDescriptor(navigatorProto, 'webdriver');
    if (webdriver && webdriver.get) {
      var realWebdriver = webdriver.get;
      // The real getter runs first, so a call on anything but a navigator throws as it would.
      var notDriven = Object.getOwnPropertyDescriptor({ get webdriver() { realWebdriver.call(this); return false; } }, 'webdriver').get;
      natives.set(notDriven, realToString.call(webdriver.get));
      Object.defineProperty(navigatorProto, 'webdriver', { get: notDriven, set: webdriver.set, enumerable: webdriver.enumerable, configurable: webdriver.configurable });
    }
  } catch (e) {}
})();
"#;

/// Hosts whose links belong to the account (sign-in, account pages).
fn is_claude_host(host: &str) -> bool {
    ["claude.ai", "claude.com", "anthropic.com"].iter().any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

fn valid_web_url(url: &str) -> Option<url::Url> {
    if url.len() > 16384 || url.chars().any(|c| c.is_control() || c.is_whitespace() || c == '"') { return None; }
    url::Url::parse(url).ok().filter(|u| matches!(u.scheme(), "http" | "https"))
}

/// What the account browser needs, decided off the UI thread (it may wait for the node).
pub struct Plan {
    id: String,
    relay_port: Option<u16>,
    timezone: Option<String>,
    locale: String,
    url: String,
    exit: Option<crate::node::ExitInfo>,
    /// The installed browser the account chose (see `chromium::pick`).
    browser: Option<String>,
    /// A desktop account (not a container one), whose Claude takes `claude://` sign-in callbacks.
    #[cfg_attr(not(windows), allow(dead_code))]
    desktop: bool,
}

/// What a route check found for an account's browser.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    /// `None`: no node, the browser would use this machine's network.
    exit: Option<crate::node::ExitInfo>,
    /// The zone pages will see; `None` = the system's.
    timezone: Option<String>,
    locale: String,
    /// Network type and risk record of the exit IP; only a standalone test looks them up, so
    /// opening the browser does not wait for those services.
    quality: Option<String>,
    risk: Option<String>,
}

impl Plan {
    pub fn check(&self) -> Check {
        Check { exit: self.exit.clone(), timezone: self.timezone.clone(), locale: self.locale.clone(), quality: None, risk: None }
    }
}

/// Checks the account's route exactly as opening its browser does, without opening it.
pub fn test(id: &str) -> Result<Check, String> {
    let plan = plan(id, None)?;
    let mut check = plan.check();
    if let (Some(port), Some(exit)) = (plan.relay_port, &plan.exit) {
        let client = crate::node::relay_client(port, Duration::from_secs(15))?;
        check.quality = crate::node::network_quality(&client, &exit.ip);
        check.risk = Some(crate::node::reputation(&client, &exit.ip).to_string());
    }
    Ok(check)
}

/// Checks the account's route like a launch does and returns how to open its browser.
pub fn plan(id: &str, url: Option<&str>) -> Result<Plan, String> {
    let url = url.unwrap_or(START_URL);
    valid_web_url(url).ok_or("只能打开 http 或 https 链接。")?;
    let s = settings::load();
    // Container accounts have no desktop profile (and may exist without Claude.app installed).
    let desktop = s.accounts.get(id).is_none_or(|a| a.container.is_none());
    let is_default = desktop && {
        let install = crate::install::resolve()?;
        crate::profiles::resolve(&install, id, false).ok_or("找不到这个账号。")?.is_default
    };
    let eff = s.effective(id);
    // A soft cut is re-checked by `port_for_launch` below instead of being refused here.
    if let Some(reason) = s.account(id).blocked.filter(|_| !s.account(id).blocked_soft) {
        return Err(format!("出口保护已切断这个账号的网络——{reason}\n请先“恢复网络”或重新绑定出口。"));
    }
    if eff.unusable_node { return Err("这个账号的代理节点无法读取（密码不能解密或配置损坏），为避免直连已拒绝打开。请重新保存它的环境配置。".into()); }
    if eff.node.is_none() && !is_default && s.requires_environment() {
        return Err("这个账号没有绑定带代理节点的环境，账号浏览器会用本机网络直连，已拒绝打开。请先在“账号 → 编辑”中选择环境。".into());
    }
    let mut timezone = match eff.timezone.as_str() { "" | "proxy" | "system" => None, tz => Some(tz.to_string()) };
    let (mut relay_port, mut checked) = (None, None);
    if let Some(node) = &eff.node {
        let (port, exit) = crate::launcher::verify_route(id, eff.environment.as_deref(), node)?;
        if matches!(eff.timezone.as_str(), "" | "proxy") {
            if exit.timezone.is_empty() { return Err("IP 查询服务没有返回出口时区，无法让浏览器时区与出口一致。请换一个 IP 查询渠道或手动指定时区。".into()); }
            timezone = Some(exit.timezone.clone());
        }
        relay_port = Some(port);
        checked = Some(exit);
    }
    let locale = locale_of(&s, eff.environment.as_deref());
    let browser = s.account(id).browser;
    Ok(Plan { id: id.to_string(), relay_port, timezone, locale, url: url.to_string(), exit: checked, browser, desktop })
}

fn locale_of(s: &settings::Settings, environment: Option<&str>) -> String {
    environment.and_then(|e| s.environments.get(e)).map(|e| e.locale.clone()).unwrap_or_default()
}

/// What account `id`'s browser presents to pages, as set up now.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Fingerprint {
    /// The browser that shows the pages and where it is installed, or why there is none.
    browser: Option<String>,
    browser_path: Option<String>,
    browser_error: Option<String>,
    /// The browsers the account can choose from, and its choice; `None` = the first of them.
    browsers: Vec<String>,
    browser_choice: Option<String>,
    /// The environment's zone setting: "" / "proxy" follow the exit, "system", or an IANA name.
    timezone: String,
    /// The zone of the last verified exit, for a zone that follows it.
    exit_timezone: Option<String>,
    locale: String,
    /// Whether it leaves through a node (the proxy, DNS, QUIC and WebRTC switches apply).
    proxied: bool,
    data_dir: String,
}

pub fn fingerprint(id: &str) -> Fingerprint {
    let s = settings::load();
    let (eff, account) = (s.effective(id), s.account(id));
    let installed = crate::chromium::installed();
    let browsers = installed.iter().map(|(n, _)| n.clone()).collect();
    let (browser, browser_path, browser_error, dir) = match crate::chromium::pick(&installed, account.browser.as_deref()) {
        Ok((name, exe)) => {
            let dir = crate::chromium::profile_dir(&data_dir(id), &name);
            (Some(name), Some(exe.display().to_string()), None, dir)
        }
        Err(e) => (None, None, Some(e), data_dir(id)),
    };
    Fingerprint {
        browser,
        browser_path,
        browser_error,
        browsers,
        browser_choice: account.browser.clone(),
        timezone: eff.timezone.clone(),
        exit_timezone: account.last_exit.as_ref().map(|e| e.timezone.clone()).filter(|tz| !tz.is_empty()),
        locale: locale_of(&s, eff.environment.as_deref()),
        proxied: eff.node.is_some(),
        data_dir: dir.display().to_string(),
    }
}

/// Sets which installed browser account `id` runs; `None` = the first one found. A running
/// browser keeps going until it is closed (see `chromium::open`).
pub fn set_browser(id: &str, choice: Option<String>) -> Result<(), String> {
    if let Some(name) = &choice {
        if !crate::chromium::installed().iter().any(|(n, _)| n == name) { return Err(format!("本机没有找到 {name}。")); }
    }
    settings::update_with(|s| {
        s.accounts.get_mut(id).ok_or("找不到这个账号。")?.browser = choice;
        Ok(())
    })
}

/// Opens the link in the account's browser, starting it if it does not run. It may wait for the
/// browser's setup, so not on the UI thread.
pub fn open(plan: Plan) -> Result<(), String> {
    let (name, exe) = crate::chromium::pick(&crate::chromium::installed(), plan.browser.as_deref())?;
    let launch = crate::chromium::Launch {
        exe,
        data_dir: crate::chromium::profile_dir(&data_dir(&plan.id), &name),
        relay_port: plan.relay_port,
        timezone: plan.timezone,
        locale: plan.locale,
        extra: Vec::new(),
    };
    #[cfg(windows)]
    let claude_links: Option<crate::chromium::ClaudeLinks> = plan.desktop.then_some(hand_to_claude as crate::chromium::ClaudeLinks);
    #[cfg(not(windows))]
    let claude_links = None;
    crate::chromium::open(&plan.id, launch, &plan.url, PAGE_JS.to_string(), claude_links)
}

/// A sign-in callback from account `id`'s browser goes to that account's Claude.
#[cfg(windows)]
fn hand_to_claude(id: String, url: String) {
    std::thread::spawn(move || if let Err(e) = crate::login::deliver(&id, &url) { sys::error_box(&e) });
}

// ---- link routing ----------------------------------------------------------------------

/// The account whose Claude started this process (Claude opens links with ShellExecute, so it
/// is the parent, possibly behind a launcher).
#[cfg(windows)]
fn caller_account() -> Option<String> {
    let procs = sys::processes();
    let install = crate::install::resolve().ok()?;
    let mut pid = std::process::id();
    for _ in 0..4 {
        let parent = procs.iter().find(|p| p.pid == pid)?.ppid;
        let proc = procs.iter().find(|p| p.pid == parent)?;
        if proc.name.eq_ignore_ascii_case("claude.exe") {
            let cmd = sys::command_line(parent)?;
            let key = crate::profiles::user_data_dir_arg(&cmd)
                .map(|d| crate::profiles::path_key(&d))
                .unwrap_or_else(|| crate::profiles::path_key(&install.default_profile.to_string_lossy()));
            return crate::profiles::list(&install, false).into_iter().find(|p| crate::profiles::path_key(&p.path) == key).map(|p| p.id);
        }
        pid = parent;
    }
    None
}

#[cfg(not(windows))]
fn caller_account() -> Option<String> { None }

/// Whether account `id` should sign in through its own browser.
fn needs_account_browser(id: &str) -> bool {
    let eff = settings::load().effective(id);
    eff.node.is_some() || eff.isolation == "sandboxie"
}

/// `--open-url`: decides where a link goes. Returns once it is handed off.
pub fn route_external(url: &str) -> Result<(), String> {
    let u = valid_web_url(url).ok_or("只能打开 http 或 https 链接。")?;
    let claude = u.host_str().is_some_and(is_claude_host);
    let account = caller_account().or_else(|| claude.then(crate::login::pending).flatten());
    let to_account = claude && account.as_deref().is_some_and(needs_account_browser);
    if to_account || crate::sandbox::inside() {
        // Inside a box this process has no network and its own copy of everything; the host
        // Hydra picks the request up from the drop folder (opened to the box).
        drop_request(account.as_deref(), url)?;
        if !crate::sandbox::inside() { start_host(); }
        return Ok(());
    }
    open_fallback(url)
}

fn drop_request(id: Option<&str>, url: &str) -> Result<(), String> {
    let dir = crate::sandbox::drop_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let data = serde_json::json!({ "id": id, "url": url, "at": paths::now_millis() });
    let tmp = dir.join(format!("{}-{}.tmp", paths::now_millis(), std::process::id()));
    std::fs::write(&tmp, data.to_string()).map_err(|e| format!("无法转交链接：{e}"))?;
    std::fs::rename(&tmp, tmp.with_extension("json")).map_err(|e| format!("无法转交链接：{e}"))
}

/// Starts the tray host if it is not running (a running one swallows the new instance).
fn start_host() {
    if let Ok(exe) = std::env::current_exe() {
        let mut cmd = std::process::Command::new(exe);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);
        let _ = cmd.arg("--tray").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
    }
}

/// Host process: picks up links dropped by `--open-url` (also from inside boxes).
pub fn start_watcher() {
    let _ = std::thread::Builder::new().name("link-router".into()).spawn(move || loop {
        let dir = crate::sandbox::drop_dir();
        let files = std::fs::read_dir(&dir).map(|d| d.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect::<Vec<_>>()).unwrap_or_default();
        for file in files {
            let data = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
            let _ = std::fs::remove_file(&file);
            let Some(v) = data else { continue };
            let fresh = v["at"].as_u64().is_some_and(|at| paths::now_millis().saturating_sub(at) < 60_000);
            if !fresh { paths::append_log("hydra-error.log", "link router: dropped a link that waited over 60 s for the host"); }
            let Some(url) = v["url"].as_str().filter(|_| fresh).map(str::to_owned) else { continue };
            let id = v["id"].as_str().map(str::to_owned);
            std::thread::spawn(move || if let Err(e) = handle_request(id.as_deref(), &url) { sys::error_box(&e) });
        }
        std::thread::sleep(Duration::from_millis(700));
    });
}

fn handle_request(id: Option<&str>, url: &str) -> Result<(), String> {
    let u = valid_web_url(url).ok_or("链接无效。")?;
    match id {
        Some(id) if u.host_str().is_some_and(is_claude_host) && needs_account_browser(id) => open(plan(id, Some(url))?),
        _ => open_fallback(url),
    }
}

/// Remembers the user's current https browser (unless it is Hydra itself).
#[cfg(windows)]
pub fn remember_fallback() {
    let current = sys::reg_string(false, r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoice", "ProgId");
    if let Some(prog) = current.filter(|p| !p.eq_ignore_ascii_case(URL_PROGID)) {
        let _ = settings::update(|s| s.fallback_browser = Some(prog));
    }
}

#[cfg(not(windows))]
pub fn remember_fallback() {}

/// Opens a link in the user's previous browser (Edge when unknown), never in Hydra.
#[cfg(windows)]
pub fn open_fallback(url: &str) -> Result<(), String> {
    valid_web_url(url).ok_or("链接无效。")?;
    let own = std::env::current_exe().ok().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let command = settings::load().fallback_browser.and_then(|prog| {
        [false, true].into_iter().find_map(|machine| sys::reg_string(machine, &format!(r"Software\Classes\{prog}\shell\open\command"), ""))
    }).filter(|c| !own.is_empty() && !c.to_lowercase().contains(&own) && !c.contains(URL_PROGID));
    if let Some(command) = command {
        let (exe, rest) = match command.strip_prefix('"').and_then(|c| c.split_once('"')) {
            Some((exe, rest)) => (exe.to_string(), rest.to_string()),
            None => match command.split_once(' ') { Some((e, r)) => (e.to_string(), format!(" {r}")), None => (command.clone(), String::new()) },
        };
        let args = if rest.contains("%1") || rest.contains("%L") { rest.replace("%1", url).replace("%L", url) } else { format!("{rest} \"{url}\"") };
        if std::process::Command::new(&exe).raw_arg(args.trim()).spawn().is_ok() { return Ok(()); }
    }
    std::process::Command::new("explorer.exe").arg(format!("microsoft-edge:{url}")).spawn().map(|_| ()).map_err(|e| format!("无法打开链接：{e}"))
}

#[cfg(not(windows))]
pub fn open_fallback(_url: &str) -> Result<(), String> { Err("链接路由目前只支持 Windows。".into()) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_hosts_and_urls() {
        assert!(is_claude_host("claude.ai") && is_claude_host("www.anthropic.com") && is_claude_host("platform.claude.com"));
        assert!(!is_claude_host("notclaude.ai") && !is_claude_host("accounts.google.com"));
        assert!(valid_web_url("https://claude.ai/login/app-google-auth?hop_nonce=x").is_some());
        assert!(valid_web_url("file:///c:/x").is_none());
        assert!(valid_web_url("https://a.com/\" --evil").is_none());
    }

    #[test]
    fn page_script_parses_as_javascript() {
        let path = std::env::temp_dir().join("hydra-page-script-check.js");
        std::fs::write(&path, PAGE_JS).unwrap();
        let out = std::process::Command::new("node").args(["--check"]).arg(&path).output()
            .expect("Node.js is required for the browser script tests");
        let _ = std::fs::remove_file(&path);
        assert!(out.status.success(), "node --check: {}", String::from_utf8_lossy(&out.stderr));
    }
}
