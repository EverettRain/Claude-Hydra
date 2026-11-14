//! The account browser: a Chromium browser the user already has (Chrome, Edge, Brave or Chromium)
//! run as its own instance per account and driven over the DevTools protocol through a pipe
//! (`--remote-debugging-pipe`: no port another local program could reach).
//! - its own profile folder per account, the network switches through the account's relay;
//! - the account's time zone from `Emulation.setTimezoneOverride` on each page, and off Windows
//!   also from the `TZ` variable, which every process of the browser inherits (workers included);
//! - every page, frame and worker is paused when it appears (auto-attach waiting for the
//!   debugger) until the page script is registered and the zone set, so no page script runs
//!   before them;
//! - after each load the zone the page reports is checked; a mismatch replaces the page with a
//!   warning;
//! - optionally, `claude://` links (sign-in callbacks) are taken from the pages and handed to
//!   the account instead of to whatever the system opens them with.
use base64::Engine;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{LazyLock, Mutex};

/// Browsers in order of preference. macOS: app bundle names (also what the user sees).
const MAC_APPS: [&str; 4] = ["Google Chrome", "Microsoft Edge", "Brave Browser", "Chromium"];
/// Windows: the name shown, and the executable below a base folder (Program Files, Program Files
/// (x86) or the per-user Local AppData, where each of them can be installed).
const WINDOWS_APPS: [(&str, &str); 4] = [
    ("Google Chrome", r"Google\Chrome\Application\chrome.exe"),
    ("Microsoft Edge", r"Microsoft\Edge\Application\msedge.exe"),
    ("Brave Browser", r"BraveSoftware\Brave-Browser\Application\brave.exe"),
    ("Chromium", r"Chromium\Application\chrome.exe"),
];
const OTHER_NAMES: [&str; 4] = ["google-chrome", "chromium", "chromium-browser", "microsoft-edge"];

/// The Chromium browsers installed here, in order of preference: name and executable.
pub fn installed() -> Vec<(String, PathBuf)> {
    if cfg!(windows) {
        let bases: Vec<PathBuf> = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"].iter()
            .filter_map(|v| std::env::var_os(v).map(PathBuf::from)).collect();
        WINDOWS_APPS.iter().filter_map(|(name, exe)| {
            bases.iter().map(|b| b.join(exe)).find(|p| p.is_file()).map(|p| (name.to_string(), p))
        }).collect()
    } else if cfg!(target_os = "macos") {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        MAC_APPS.iter().filter_map(|app| {
            [PathBuf::from("/Applications"), home.join("Applications")].into_iter()
                .map(|root| root.join(format!("{app}.app/Contents/MacOS/{app}")))
                .find(|p| p.is_file())
                .map(|p| (app.to_string(), p))
        }).collect()
    } else {
        let path = std::env::var_os("PATH").unwrap_or_default();
        OTHER_NAMES.iter().filter_map(|n| {
            std::env::split_paths(&path).map(|d| d.join(n)).find(|p| p.is_file()).map(|p| (n.to_string(), p))
        }).collect()
    }
}

/// The browser to run: the account's `choice`, or the first one installed. A chosen browser
/// that is gone is an error rather than another browser, which sites would see as another device.
pub fn pick(installed: &[(String, PathBuf)], choice: Option<&str>) -> Result<(String, PathBuf), String> {
    match choice {
        Some(name) => installed.iter().find(|(n, _)| n == name).cloned()
            .ok_or_else(|| format!("这个账号设置使用 {name}，但本机没有找到它。请重新安装，或在「网页端 → 配置」中改选其他浏览器。")),
        None => installed.first().cloned()
            .ok_or_else(|| "账号浏览器需要本机装有 Google Chrome、Microsoft Edge、Brave 或 Chromium 之一，请先安装其中一个。".into()),
    }
}

/// The browser's own profile folder inside the account's: browsers cannot share one (each
/// encrypts its cookies with its own key), and switching browsers starts afresh anyway.
pub fn profile_dir(account_dir: &std::path::Path, browser: &str) -> PathBuf {
    account_dir.join(browser.to_ascii_lowercase().replace(' ', "-"))
}

/// How one account's browser is started. A running one is reused only while these hold.
#[derive(Clone, PartialEq, Debug)]
pub struct Launch {
    pub exe: PathBuf,
    pub data_dir: PathBuf,
    pub relay_port: Option<u16>,
    pub timezone: Option<String>,
    pub locale: String,
    /// More switches (tests run headless, and as root without the sandbox).
    pub extra: Vec<String>,
}

fn args(l: &Launch) -> Vec<String> {
    let mut a = vec![
        format!("--user-data-dir={}", l.data_dir.display()),
        // Remote debugging also makes `navigator.webdriver` true; the account's script answers
        // false (see `browser::PAGE_JS`). A switch that turns it off makes Chrome show an
        // "unsupported command-line flag" warning bar instead.
        "--remote-debugging-pipe".into(),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
    ];
    if let Some(port) = l.relay_port { a.extend(crate::node::chromium_args(port)); }
    if !l.locale.is_empty() {
        let lang = l.locale.split('-').next().unwrap_or_default();
        a.push(format!("--lang={}", l.locale));
        a.push(format!("--accept-lang={},{lang}", l.locale));
    }
    a.extend(l.extra.iter().cloned());
    // The first page waits on a blank tab until it is set up (see `Driver::attached`).
    a.push("about:blank".into());
    a
}

enum Event {
    Message(Value),
    Open(String),
    Close,
    Closed,
}

struct Running {
    launch: Launch,
    events: Sender<Event>,
}

static RUNNING: LazyLock<Mutex<HashMap<String, Running>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Receives a `claude://` link a page of account `id`'s browser opened: the account id and link.
pub type ClaudeLinks = fn(String, String);

/// Opens `url` in account `id`'s browser: a new tab when it runs, else a new browser instance.
/// `script` runs in every document before the page's own scripts. With `claude_links`, the
/// `claude://` links pages open go there instead of to the system.
pub fn open(id: &str, launch: Launch, url: &str, script: String, claude_links: Option<ClaudeLinks>) -> Result<(), String> {
    let mut running = RUNNING.lock().map_err(|_| "账号浏览器状态异常，请重启 Claude-Hydra。")?;
    if let Some(r) = running.get(id) {
        if r.launch != launch {
            return Err("这个账号的浏览器、网络或时区设置已变化，正在运行的账号浏览器还在用旧的设置。请先关闭它的浏览器窗口，再重新打开。".into());
        }
        if r.events.send(Event::Open(url.to_string())).is_ok() { return Ok(()); }
    }
    std::fs::create_dir_all(&launch.data_dir).map_err(|e| format!("无法创建浏览器数据目录：{e}"))?;
    let (mut child, to_browser, from_browser) = os::spawn(&launch).map_err(|e| format!("无法启动账号浏览器：{e}"))?;
    let (events, inbox) = channel();
    let reader = events.clone();
    std::thread::Builder::new().name("account-browser-read".into())
        .spawn(move || read_messages(from_browser, reader)).map_err(|e| e.to_string())?;
    // The browser's exit ends the driver even while another process still holds a copy of the
    // pipe (one spawned at the same moment could inherit it), which would keep the read open.
    let waiter = events.clone();
    std::thread::Builder::new().name("account-browser-wait".into())
        .spawn(move || { child.wait(); let _ = waiter.send(Event::Closed); }).map_err(|e| e.to_string())?;
    let driver = Driver {
        out: to_browser, next: 0, script, timezone: launch.timezone.clone(), pending: VecDeque::from([url.to_string()]),
        checks: HashMap::new(), navigations: Vec::new(), owner: id.to_string(), claude_links, handed: None,
    };
    std::thread::Builder::new().name("account-browser".into())
        .spawn(move || driver.run(inbox)).map_err(|e| e.to_string())?;
    running.insert(id.to_string(), Running { launch, events });
    Ok(())
}

/// Closes account `id`'s browser, if it runs.
pub fn close(id: &str) {
    if let Some(r) = RUNNING.lock().ok().and_then(|mut m| m.remove(id)) { let _ = r.events.send(Event::Close); }
}

/// A Windows command-line argument as the C runtime splits it back (backslashes count only
/// before a quote).
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) { return arg.to_string(); }
    let (mut out, mut slashes) = (String::from('"'), 0);
    for c in arg.chars() {
        match c {
            '\\' => { slashes += 1; continue; }
            '"' => out.push_str(&"\\".repeat(slashes * 2 + 1)),
            _ => out.push_str(&"\\".repeat(slashes)),
        }
        out.push(c);
        slashes = 0;
    }
    out.push_str(&"\\".repeat(slashes * 2));
    out.push('"');
    out
}

#[cfg(unix)]
mod os {
    use super::{args, Launch};
    use std::fs::File;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    pub struct Child(std::process::Child);

    impl Child {
        pub fn wait(&mut self) { let _ = self.0.wait(); }
    }

    /// A pipe whose ends sit above the standard descriptors and close on exec: the browser gets
    /// only the copies `spawn` places at 3 and 4.
    fn pipe() -> std::io::Result<(OwnedFd, OwnedFd)> {
        let mut fds = [0; 2];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 { return Err(std::io::Error::last_os_error()); }
        let lift = |fd: i32| {
            let high = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 10) };
            unsafe { libc::close(fd) };
            if high < 0 { Err(std::io::Error::last_os_error()) } else { Ok(unsafe { OwnedFd::from_raw_fd(high) }) }
        };
        Ok((lift(fds[0])?, lift(fds[1])?))
    }

    /// Starts the browser; returns it with the pipe ends to write commands to and read answers from.
    pub fn spawn(l: &Launch) -> std::io::Result<(Child, File, File)> {
        use std::os::unix::process::CommandExt;
        // DevTools protocol over a pipe: the browser reads commands on fd 3 and answers on fd 4.
        let (commands_read, commands_write) = pipe()?;
        let (answers_read, answers_write) = pipe()?;
        let (into, out) = (commands_read.as_raw_fd(), answers_write.as_raw_fd());
        let mut c = std::process::Command::new(&l.exe);
        c.args(args(l)).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        if let Some(tz) = &l.timezone { c.env("TZ", tz); }
        unsafe {
            c.pre_exec(move || {
                if libc::dup2(into, 3) < 0 || libc::dup2(out, 4) < 0 { return Err(std::io::Error::last_os_error()); }
                Ok(())
            });
        }
        let child = c.spawn()?;
        drop((commands_read, answers_write));
        Ok((Child(child), File::from(commands_write), File::from(answers_read)))
    }
}

/// Windows passes the pipe to the browser as two inherited handles named in
/// `--remote-debugging-io-pipes` (as ChromeDriver does). The handle list keeps the browser from
/// inheriting any other handle Hydra has open.
#[cfg(windows)]
mod os {
    use super::{args, windows_arg, Launch};
    use std::fs::File;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::Foundation::{CloseHandle, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT};
    use windows_sys::Win32::System::Pipes::CreatePipe;
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, DeleteProcThreadAttributeList, InitializeProcThreadAttributeList, UpdateProcThreadAttribute,
        WaitForSingleObject, EXTENDED_STARTUPINFO_PRESENT, INFINITE, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
        STARTUPINFOEXW,
    };

    pub struct Child(OwnedHandle);

    impl Child {
        pub fn wait(&mut self) { unsafe { WaitForSingleObject(self.0.as_raw_handle() as HANDLE, INFINITE) }; }
    }

    fn pipe() -> std::io::Result<(OwnedHandle, OwnedHandle)> {
        let (mut read, mut write): (HANDLE, HANDLE) = (std::ptr::null_mut(), std::ptr::null_mut());
        if unsafe { CreatePipe(&mut read, &mut write, std::ptr::null(), 0) } == 0 { return Err(std::io::Error::last_os_error()); }
        Ok(unsafe { (OwnedHandle::from_raw_handle(read as _), OwnedHandle::from_raw_handle(write as _)) })
    }

    /// Starts the browser; returns it with the pipe ends to write commands to and read answers from.
    pub fn spawn(l: &Launch) -> std::io::Result<(Child, File, File)> {
        let fail = || std::io::Error::last_os_error();
        let (commands_read, commands_write) = pipe()?;
        let (answers_read, answers_write) = pipe()?;
        let inherited: [HANDLE; 2] = [commands_read.as_raw_handle() as HANDLE, answers_write.as_raw_handle() as HANDLE];
        for h in inherited {
            if unsafe { SetHandleInformation(h, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) } == 0 { return Err(fail()); }
        }
        let mut line = windows_arg(&l.exe.to_string_lossy());
        for a in args(l) {
            line.push(' ');
            line.push_str(&windows_arg(&a));
        }
        line.push_str(&format!(" --remote-debugging-io-pipes={},{}", inherited[0] as usize as u32, inherited[1] as usize as u32));
        let mut wide: Vec<u16> = std::ffi::OsStr::new(&line).encode_wide().chain([0]).collect();

        let mut size = 0usize;
        unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size) };
        // usize words: the list holds pointers and must be aligned for them.
        let mut list_buf = vec![0usize; size.div_ceil(std::mem::size_of::<usize>())];
        let list = list_buf.as_mut_ptr() as _;
        if unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut size) } == 0 { return Err(fail()); }
        let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        let created = unsafe {
            let ok = UpdateProcThreadAttribute(
                list, 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                inherited.as_ptr() as *const _, std::mem::size_of_val(&inherited), std::ptr::null_mut(), std::ptr::null(),
            ) != 0 && {
                let mut startup: STARTUPINFOEXW = std::mem::zeroed();
                startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
                startup.lpAttributeList = list;
                CreateProcessW(
                    std::ptr::null(), wide.as_mut_ptr(), std::ptr::null(), std::ptr::null(), 1,
                    EXTENDED_STARTUPINFO_PRESENT, std::ptr::null(), std::ptr::null(), &startup.StartupInfo, &mut info,
                ) != 0
            };
            let error = (!ok).then(fail);
            DeleteProcThreadAttributeList(list);
            error.map_or(Ok(()), Err)
        };
        created?;
        unsafe { CloseHandle(info.hThread) };
        drop((commands_read, answers_write));
        let process = unsafe { OwnedHandle::from_raw_handle(info.hProcess as _) };
        Ok((Child(process), File::from(commands_write), File::from(answers_read)))
    }
}

/// Messages are JSON texts each ended by a NUL byte.
fn read_messages(mut from: File, events: Sender<Event>) {
    let (mut buf, mut chunk) = (Vec::new(), [0u8; 64 * 1024]);
    loop {
        match from.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
        while let Some(end) = buf.iter().position(|b| *b == 0) {
            let message: Vec<u8> = buf.drain(..=end).collect();
            if let Ok(v) = serde_json::from_slice(&message[..end]) {
                if events.send(Event::Message(v)).is_err() { return; }
            }
        }
    }
    let _ = events.send(Event::Closed);
}

fn message_page(title: &str, text: &str, color: &str) -> String {
    format!("<!doctype html><meta charset=utf-8><title>{title}</title><body style=\"font:15px system-ui;padding:40px;color:{color}\"><h3>{title}</h3><p>{text}</p></body>")
}

fn data_url(page: &str) -> String {
    let encoded: String = page.bytes().map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    format!("data:text/html;charset=utf-8,{encoded}")
}

fn warning_url(text: &str) -> String { data_url(&message_page("账号浏览器已停止加载", text, "#b42318")) }

/// What a tab shows instead of a `claude://` link it handed to the account.
fn handed_page() -> String {
    message_page("已交给这个账号的 Claude", "登录回调已交给这个账号的 Claude，可以关闭这个页面。", "inherit")
}

const AUTO_ATTACH: &str = "Target.setAutoAttach";

struct Driver {
    out: File,
    next: u64,
    script: String,
    timezone: Option<String>,
    /// Links waiting for a blank tab to be set up.
    pending: VecDeque<String>,
    /// Zone checks in flight: command id → session.
    checks: HashMap<u64, String>,
    /// Tabs to navigate once Chrome confirmed their setup: the commands still unanswered, the
    /// session, target and link. Script registration runs in the page's renderer while
    /// navigation starts in the browser process, so sending both at once lets the page win.
    navigations: Vec<(Vec<u64>, String, String, String)>,
    owner: String,
    claude_links: Option<ClaudeLinks>,
    /// The last `claude://` link handed over: one link can arrive through two events.
    handed: Option<String>,
}

impl Driver {
    fn send(&mut self, session: Option<&str>, method: &str, params: Value) -> u64 {
        self.next += 1;
        let mut m = json!({ "id": self.next, "method": method, "params": params });
        if let Some(s) = session { m["sessionId"] = json!(s); }
        let mut bytes = m.to_string().into_bytes();
        bytes.push(0);
        let _ = self.out.write_all(&bytes);
        self.next
    }

    fn run(mut self, inbox: Receiver<Event>) {
        // Every target, existing or new, is attached and paused until `attached` set it up.
        self.send(None, AUTO_ATTACH, json!({ "autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true }));
        for event in inbox {
            match event {
                Event::Message(m) => self.handle(&m),
                Event::Open(url) => {
                    self.pending.push_back(url);
                    self.send(None, "Target.createTarget", json!({ "url": "about:blank" }));
                }
                Event::Close => { self.send(None, "Browser.close", json!({})); }
                Event::Closed => break,
            }
        }
        // A newer browser for the same account may already be registered; leave that one.
        if let Ok(mut m) = RUNNING.lock() {
            if m.get(&self.owner).is_some_and(|r| r.events.send(Event::Message(Value::Null)).is_err()) { m.remove(&self.owner); }
        }
    }

    fn handle(&mut self, m: &Value) {
        let session = m["sessionId"].as_str();
        match m["method"].as_str() {
            Some("Target.attachedToTarget") => self.attached(&m["params"]),
            // Links a page script, link or form opens.
            Some("Page.frameRequestedNavigation") => if let Some(s) = session {
                let url = m["params"]["url"].as_str().unwrap_or_default();
                if url.starts_with("claude:") { self.hand_over(s, url.to_string(), None); }
            },
            // Main documents' responses, for redirects to a `claude://` link (a sign-in callback
            // often arrives as one).
            Some("Fetch.requestPaused") => if let Some(s) = session { self.paused(s, &m["params"]) },
            Some("Page.loadEventFired") => if let (Some(s), Some(_)) = (session, &self.timezone) {
                let id = self.send(Some(s), "Runtime.evaluate", json!({ "expression": "Intl.DateTimeFormat().resolvedOptions().timeZone", "returnByValue": true }));
                self.checks.insert(id, s.to_string());
            },
            None if m["id"].is_u64() => self.answered(m),
            _ => {}
        }
    }

    fn answered(&mut self, m: &Value) {
        let Some(id) = m["id"].as_u64() else { return };
        if let Some(i) = self.navigations.iter().position(|(ids, ..)| ids.contains(&id)) {
            self.navigations[i].0.retain(|x| *x != id);
            let failed = m["error"]["message"].as_str().map(str::to_owned);
            if failed.is_some() || self.navigations[i].0.is_empty() {
                let (_, s, target, url) = self.navigations.remove(i);
                let url = match failed {
                    Some(e) => warning_url(&format!("无法设置账号浏览器（{e}），为避免与账号的时区或指纹不一致，已停止加载。")),
                    None => url,
                };
                self.send(Some(&s), "Page.navigate", json!({ "url": url }));
                self.send(None, "Target.activateTarget", json!({ "targetId": target }));
            }
        }
        if let Some(s) = self.checks.remove(&id) {
            let seen = m["result"]["result"]["value"].as_str().unwrap_or_default().to_string();
            let expected = self.timezone.clone().unwrap_or_default();
            if !seen.is_empty() && seen != expected {
                let url = warning_url(&format!("页面看到的时区是 {seen}，与账号时区 {expected} 不一致。请关闭这个浏览器后重新打开。"));
                self.send(Some(&s), "Page.navigate", json!({ "url": url }));
            }
        }
    }

    fn paused(&mut self, session: &str, p: &Value) {
        let request = p["requestId"].clone();
        let redirect = p["responseStatusCode"].as_u64().is_some_and(|c| (300..400).contains(&c));
        let location = p["responseHeaders"].as_array().and_then(|h| {
            h.iter().find(|x| x["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case("location")))
        }).and_then(|x| x["value"].as_str()).filter(|v| redirect && v.starts_with("claude:")).map(str::to_owned);
        match location {
            Some(url) => self.hand_over(session, url, Some(request)),
            None => { self.send(Some(session), "Fetch.continueRequest", json!({ "requestId": request })); }
        }
    }

    /// Hands a `claude://` link to the account and shows the tab a note in its place, which also
    /// ends the "open Claude?" prompt the browser may be showing for it.
    fn hand_over(&mut self, session: &str, url: String, request: Option<Value>) {
        let page = handed_page();
        match request {
            Some(request) => {
                let body = base64::engine::general_purpose::STANDARD.encode(&page);
                let headers = json!([{ "name": "Content-Type", "value": "text/html; charset=utf-8" }]);
                self.send(Some(session), "Fetch.fulfillRequest", json!({ "requestId": request, "responseCode": 200, "responseHeaders": headers, "body": body }));
            }
            None => { self.send(Some(session), "Page.navigate", json!({ "url": data_url(&page) })); }
        }
        if self.handed.as_deref() == Some(url.as_str()) { return; }
        self.handed = Some(url.clone());
        if let Some(deliver) = self.claude_links { deliver(self.owner.clone(), url); }
    }

    /// Sets up a target the browser paused for us, then lets it run.
    fn attached(&mut self, p: &Value) {
        let Some(s) = p["sessionId"].as_str().map(str::to_owned) else { return };
        let kind = p["targetInfo"]["type"].as_str().unwrap_or_default();
        let mut setup = Vec::new();
        if matches!(kind, "page" | "iframe") {
            self.send(Some(&s), "Page.enable", json!({}));
            let script = self.script.clone();
            setup.push(self.send(Some(&s), "Page.addScriptToEvaluateOnNewDocument", json!({ "source": script })));
            if let Some(tz) = self.timezone.clone() { setup.push(self.send(Some(&s), "Emulation.setTimezoneOverride", json!({ "timezoneId": tz }))); }
            if kind == "page" && self.claude_links.is_some() {
                let documents = json!([{ "urlPattern": "*", "resourceType": "Document", "requestStage": "Response" }]);
                setup.push(self.send(Some(&s), "Fetch.enable", json!({ "patterns": documents })));
            }
            // Frames in other processes and workers are separate targets: pause them too.
            self.send(Some(&s), AUTO_ATTACH, json!({ "autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true }));
        }
        self.send(Some(&s), "Runtime.runIfWaitingForDebugger", json!({}));
        let blank = matches!(p["targetInfo"]["url"].as_str(), Some("" | "about:blank"));
        if kind == "page" && blank {
            if let Some(url) = self.pending.pop_front() {
                let target = p["targetInfo"]["targetId"].as_str().unwrap_or_default().to_string();
                self.navigations.push((setup, s, target, url));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;
    use std::path::Path;
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    fn launch(dir: &Path) -> Launch {
        Launch { exe: PathBuf::from("/x/chrome"), data_dir: dir.to_path_buf(), relay_port: Some(4321), timezone: Some("Asia/Tokyo".into()), locale: "ja-JP".into(), extra: vec![] }
    }

    #[test]
    fn arguments_route_through_the_relay_in_the_accounts_profile() {
        // Arrange
        let l = launch(Path::new("/data/acct a"));

        // Act
        let a = args(&l);

        // Assert
        for expected in ["--user-data-dir=/data/acct a", "--remote-debugging-pipe", "--proxy-server=socks5://127.0.0.1:4321", "--disable-quic", "--accept-lang=ja-JP,ja"] {
            assert!(a.iter().any(|x| x == expected), "{expected}");
        }
        assert!(!a.iter().any(|x| x.starts_with("--remote-debugging-port")), "never a port other programs could reach");
        assert!(!a.iter().any(|x| x.starts_with("--disable-blink-features")), "Chrome warns about it in a bar");
        assert_eq!(a.last().unwrap(), "about:blank", "the first page waits until it is set up");
        assert!(!a.iter().any(|x| x == "--headless=new" || x == "--no-sandbox"));
    }

    #[test]
    fn an_account_gets_the_browser_it_chose() {
        // Arrange
        let here = [("Google Chrome".to_string(), PathBuf::from("/c")), ("Brave Browser".to_string(), PathBuf::from("/b"))];

        // Act
        let chosen = pick(&here, Some("Brave Browser"));
        let automatic = pick(&here, None);
        let gone = pick(&here, Some("Microsoft Edge"));

        // Assert
        assert_eq!(chosen.unwrap().1, PathBuf::from("/b"));
        assert_eq!(automatic.unwrap().1, PathBuf::from("/c"), "without a choice the first installed one runs");
        assert!(gone.is_err(), "a browser that is gone must not be swapped for another one");
        assert!(pick(&[], None).is_err());
    }

    #[test]
    fn each_browser_keeps_its_own_profile() {
        // Arrange
        let account = Path::new("/data/acct");

        // Act
        let (chrome, brave) = (profile_dir(account, "Google Chrome"), profile_dir(account, "Brave Browser"));

        // Assert
        assert_ne!(chrome, brave);
        assert!(chrome.starts_with(account) && brave.starts_with(account), "deleting the account folder removes every browser's data");
    }

    #[test]
    fn warning_pages_are_inert_data_urls() {
        // Arrange
        let text = "时区 <script>x</script> & \"q\"";

        // Act
        let url = warning_url(text);

        // Assert
        assert!(url.starts_with("data:text/html;charset=utf-8,"));
        assert!(!url[29..].contains(['<', '>', '"', '&', ' ']));
    }

    #[test]
    fn windows_arguments_survive_the_command_line() {
        // Arrange
        let cases = [
            ("--lang=ja-JP", "--lang=ja-JP"),
            (r"--user-data-dir=C:\Users\A B\account-browser\google-chrome", r#""--user-data-dir=C:\Users\A B\account-browser\google-chrome""#),
            (r"C:\dir with space\", r#""C:\dir with space\\""#),
            (r#"say "hi""#, r#""say \"hi\"""#),
            ("", r#""""#),
        ];

        // Act
        let quoted: Vec<String> = cases.iter().map(|(arg, _)| windows_arg(arg)).collect();

        // Assert
        for ((arg, expected), got) in cases.iter().zip(&quoted) { assert_eq!(got, expected, "{arg}"); }
    }

    /// Taking `claude://` links (as on Windows) must not hold up ordinary pages.
    fn ignore_links(_: String, _: String) {}

    static HANDED: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
    fn record_handed(id: String, url: String) { HANDED.lock().unwrap().push((id, url)); }

    /// Sign-in callbacks reach the account whether a page script opens them or a redirect does,
    /// and the tab shows a note in their place.
    #[test]
    fn claude_links_go_to_the_account() {
        // Arrange
        let Ok((_, exe)) = pick(&installed(), None) else { return };
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        std::thread::spawn(move || for stream in server.incoming().flatten() {
            let mut line = String::new();
            let _ = std::io::BufReader::new(&stream).read_line(&mut line);
            let response = if line.contains("/redirect") {
                "HTTP/1.1 302 Found\r\nlocation: claude://claude.ai/login/app-google-auth?code=redirected\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_string()
            } else {
                let body = "<!doctype html><script>location.href = 'claude://claude.ai/login/app-google-auth?code=scripted';</script>";
                format!("HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len())
            };
            let _ = (&stream).write_all(response.as_bytes());
        });
        let dir = std::env::temp_dir().join(format!("hydra-chromium-{}", uuid::Uuid::new_v4().simple()));
        let l = real_launch(exe, &dir, "UTC");

        // Act
        open("test-claude-links", l.clone(), &format!("http://127.0.0.1:{port}/scripted"), String::new(), Some(record_handed)).unwrap();
        open("test-claude-links", l, &format!("http://127.0.0.1:{port}/redirect"), String::new(), Some(record_handed)).unwrap();
        let until = Instant::now() + Duration::from_secs(30);
        while HANDED.lock().unwrap().len() < 2 && Instant::now() < until { std::thread::sleep(Duration::from_millis(100)); }
        close("test-claude-links");

        // Assert
        let mut got = HANDED.lock().unwrap().clone();
        got.sort();
        assert_eq!(got, [
            ("test-claude-links".to_string(), "claude://claude.ai/login/app-google-auth?code=redirected".to_string()),
            ("test-claude-links".to_string(), "claude://claude.ai/login/app-google-auth?code=scripted".to_string()),
        ]);
        std::thread::sleep(Duration::from_millis(500));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A local page that reports what it sees to `reports`; returns its port.
    fn reporting_server(reports: std::sync::Arc<Mutex<Vec<String>>>) -> u16 {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        let page = r#"<!doctype html><script>
          const mark = typeof window.__hydra;
          const tz = Intl.DateTimeFormat().resolvedOptions().timeZone;
          const w = new Worker(URL.createObjectURL(new Blob(["postMessage(Intl.DateTimeFormat().resolvedOptions().timeZone)"])));
          const getter = Object.getOwnPropertyDescriptor(Navigator.prototype, 'webdriver').get;
          const native = Function.prototype.toString.call(getter) === 'function get webdriver() { [native code] }';
          w.onmessage = (e) => fetch('/report' + location.pathname + '?mark=' + mark + '&tz=' + tz + '&worker=' + e.data + '&webdriver=' + navigator.webdriver + '&native=' + native);
        </script>"#;
        std::thread::spawn(move || for stream in server.incoming().flatten() {
            let mut line = String::new();
            let _ = std::io::BufReader::new(&stream).read_line(&mut line);
            let path = line.split_whitespace().nth(1).unwrap_or_default().to_string();
            let report = path.starts_with("/report");
            if report { reports.lock().unwrap().push(path); }
            let body = if report { "" } else { page };
            let _ = (&stream).write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes());
        });
        port
    }

    fn real_launch(exe: PathBuf, dir: &Path, timezone: &str) -> Launch {
        let extra = vec!["--headless=new".to_string(), "--no-sandbox".to_string()];
        Launch { exe, data_dir: dir.to_path_buf(), relay_port: None, timezone: Some(timezone.into()), locale: "en-US".into(), extra }
    }

    /// The page script in a real browser leaves the canvas as the browser draws it: any altered
    /// reading is something fingerprint checks can find.
    #[test]
    fn the_page_script_leaves_the_canvas_real() {
        // Arrange
        let Ok((_, exe)) = pick(&installed(), None) else { return };
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        let reports = std::sync::Arc::new(Mutex::new(Vec::<String>::new()));
        let seen = reports.clone();
        let page = r#"<!doctype html><canvas id=c width=200 height=60></canvas><script>
          const read = () => { const c = document.getElementById('c'), g = c.getContext('2d');
            g.clearRect(0, 0, 200, 60); g.font = '20px sans-serif'; g.fillStyle = '#e33'; g.fillText('Hydra 指纹', 4, 30);
            let h = 0; for (const ch of c.toDataURL()) { h = (Math.imul(h, 31) + ch.charCodeAt(0)) | 0; } return h; };
          fetch('/report' + location.pathname + '?a=' + read() + '&b=' + read());
        </script>"#;
        std::thread::spawn(move || for stream in server.incoming().flatten() {
            let mut line = String::new();
            let _ = std::io::BufReader::new(&stream).read_line(&mut line);
            let path = line.split_whitespace().nth(1).unwrap_or_default().to_string();
            let report = path.starts_with("/report");
            if report { seen.lock().unwrap().push(path); }
            let body = if report { "" } else { page };
            let _ = (&stream).write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes());
        });
        let dirs: Vec<PathBuf> = (0..2).map(|_| std::env::temp_dir().join(format!("hydra-chromium-{}", uuid::Uuid::new_v4().simple()))).collect();

        // Act
        for (i, (account, script)) in [("fp-scripted", crate::browser::PAGE_JS), ("fp-plain", "")].into_iter().enumerate() {
            open(account, real_launch(exe.clone(), &dirs[i], "UTC"), &format!("http://127.0.0.1:{port}/{account}"), script.to_string(), None).unwrap();
        }
        let until = Instant::now() + Duration::from_secs(30);
        while reports.lock().unwrap().len() < 2 && Instant::now() < until { std::thread::sleep(Duration::from_millis(100)); }
        for account in ["fp-scripted", "fp-plain"] { close(account); }

        // Assert
        let got = reports.lock().unwrap().clone();
        let canvas = |account: &str| -> (String, String) {
            let line = got.iter().find(|r| r.contains(account)).unwrap_or_else(|| panic!("{account} did not report: {got:?}"));
            let q: HashMap<String, String> = url::Url::parse(&format!("http://x{line}")).unwrap().query_pairs().into_owned().collect();
            (q["a"].clone(), q["b"].clone())
        };
        let (scripted, plain) = (canvas("fp-scripted"), canvas("fp-plain"));
        assert_eq!(scripted.0, scripted.1, "one canvas read twice must agree");
        assert_eq!(scripted.0, plain.0, "the page script must not change the canvas");
        std::thread::sleep(Duration::from_millis(500));
        for d in dirs { let _ = std::fs::remove_dir_all(d); }
    }

    /// A zone the browser refuses stops the page instead of loading it in another zone.
    #[test]
    fn a_zone_the_browser_refuses_stops_loading() {
        // Arrange
        let Ok((_, exe)) = pick(&installed(), None) else { return };
        let reports = std::sync::Arc::new(Mutex::new(Vec::<String>::new()));
        let port = reporting_server(reports.clone());
        let dir = std::env::temp_dir().join(format!("hydra-chromium-{}", uuid::Uuid::new_v4().simple()));

        // Act
        open("test-refused-zone", real_launch(exe, &dir, "Not/AZone"), &format!("http://127.0.0.1:{port}/page"), String::new(), None).unwrap();
        std::thread::sleep(Duration::from_secs(4));
        close("test-refused-zone");

        // Assert
        assert!(reports.lock().unwrap().is_empty(), "the page never ran");
        std::thread::sleep(Duration::from_millis(500));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Runs the installed browser for real: the page, a frame-less worker and a second tab all
    /// see the account's zone and the injected script. Skipped where no Chromium is installed.
    #[test]
    fn a_real_browser_gets_the_zone_and_script_before_page_scripts() {
        // Arrange
        let Ok((_, exe)) = pick(&installed(), None) else { return };
        let reports = std::sync::Arc::new(Mutex::new(Vec::<String>::new()));
        let port = reporting_server(reports.clone());
        let dir = std::env::temp_dir().join(format!("hydra-chromium-{}", uuid::Uuid::new_v4().simple()));
        let l = real_launch(exe, &dir, "Pacific/Honolulu");
        let wait_for = |n: usize| {
            let until = Instant::now() + Duration::from_secs(30);
            while reports.lock().unwrap().len() < n && Instant::now() < until { std::thread::sleep(Duration::from_millis(100)); }
        };

        // Act
        let script = crate::browser::PAGE_JS.to_string() + "window.__hydra = 1;";
        open("test-account", l.clone(), &format!("http://127.0.0.1:{port}/first"), script, Some(ignore_links)).unwrap();
        wait_for(1);
        open("test-account", l.clone(), &format!("http://127.0.0.1:{port}/second"), "ignored: the running browser keeps its script".into(), Some(ignore_links)).unwrap();
        wait_for(2);
        let moved = open("test-account", Launch { relay_port: Some(1), ..l }, "http://127.0.0.1:1/", String::new(), None);
        close("test-account");

        // Assert
        let mut got = reports.lock().unwrap().clone();
        got.sort();
        assert_eq!(got, [
            "/report/first?mark=number&tz=Pacific/Honolulu&worker=Pacific/Honolulu&webdriver=false&native=true",
            "/report/second?mark=number&tz=Pacific/Honolulu&worker=Pacific/Honolulu&webdriver=false&native=true",
        ]);
        assert!(moved.unwrap_err().contains("设置已变化"));
        std::thread::sleep(Duration::from_millis(500));
        let _ = std::fs::remove_dir_all(dir);
    }
}
