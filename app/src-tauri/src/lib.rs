//! Claude-Hydra application entry, system tray management, and Tauri IPC commands.

pub mod cli;
pub mod install;
pub mod launcher;
pub mod login;
pub mod paths;
pub mod profiles;
mod proxy;
pub mod quota;
pub mod settings;
pub mod sys;
#[cfg(windows)]
mod taskbar;
pub mod usage;
pub mod watchdog;

use install::Install;
use profiles::{Profile, ProfileOptions};
use serde::Serialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewWindowBuilder};

const TRAY_ID: &str = "hydra";

#[derive(Default)]
struct AppState {
    install: Mutex<Option<Install>>,
}

fn current_install(app: &AppHandle) -> Result<Install, String> {
    let state = app.state::<AppState>();
    let mut guard = state.install.lock().map_err(|_| "内部状态错误")?;
    if let Some(i) = guard.as_ref() {
        if i.launch_exe().is_some() || i.is_msix() {
            return Ok(i.clone());
        }
    }
    let found = install::resolve()?;
    *guard = Some(found.clone());
    Ok(found)
}

fn find(app: &AppHandle, id: &str) -> Result<(Install, Profile), String> {
    let i = current_install(app)?;
    let p = profiles::resolve(&i, id, false).ok_or("找不到这个账号。")?;
    Ok((i, p))
}

fn after_change(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || refresh_tray(&app));
}

#[tauri::command]
fn list_profiles(app: AppHandle) -> Result<Vec<Profile>, String> {
    let i = current_install(&app)?;
    Ok(profiles::list(&i, true))
}

#[tauri::command]
async fn launch_profile(app: AppHandle, id: String) -> Result<String, String> {
    let a = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        let (i, p) = find(&a, &id)?;
        launcher::launch(&i, &p).map(str::to_string)
    })
    .await
    .map_err(|e| e.to_string())?;
    after_change(&app);
    r
}

#[tauri::command]
async fn close_profile(app: AppHandle, id: String) -> Result<String, String> {
    let a = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        let (i, p) = find(&a, &id)?;
        launcher::close(&i, &p).map(str::to_string)
    })
    .await
    .map_err(|e| e.to_string())?;
    after_change(&app);
    r
}

#[tauri::command]
async fn restart_profile(app: AppHandle, id: String) -> Result<String, String> {
    let a = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        let (i, p) = find(&a, &id)?;
        launcher::restart(&i, &p).map(str::to_string)
    })
    .await
    .map_err(|e| e.to_string())?;
    after_change(&app);
    r
}

#[tauri::command]
async fn add_account(app: AppHandle) -> Result<Profile, String> {
    let a = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || launcher::add_account(&current_install(&a)?))
        .await
        .map_err(|e| e.to_string())?;
    after_change(&app);
    r
}

#[tauri::command]
fn next_account_name(app: AppHandle) -> Result<String, String> {
    Ok(profiles::next_account_name(&current_install(&app)?))
}

fn valid_proxy(proxy: &str) -> bool {
    let Ok(url) = url::Url::parse(proxy) else { return false };
    proxy.chars().all(|c| c.is_ascii_alphanumeric() || ":/.-[]".contains(c))
        && matches!(url.scheme(), "http" | "https" | "socks5") && url.host_str().is_some() && url.port_or_known_default().is_some()
        && url.username().is_empty() && url.password().is_none() && matches!(url.path(), "" | "/") && url.query().is_none() && url.fragment().is_none()
}

#[cfg(test)]
mod proxy_tests {
    #[test]
    fn accepts_only_simple_proxy_urls() {
        for value in ["http://127.0.0.1:7890", "socks5://localhost:1080", "http://[::1]:8080"] { assert!(super::valid_proxy(value), "{value}"); }
        for value in ["http://user:pass@host:80", "http://host:80/path", "http://host:80\" --no-proxy-server", "socks5://host", "file://host:80"] { assert!(!super::valid_proxy(value), "{value}"); }
    }
}

#[tauri::command]
async fn create_configured_account(app: AppHandle, name: String, proxy: String, locale: String) -> Result<Profile, String> {
    let a = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let install = current_install(&a)?;
        let name = name.trim();
        profiles::validate_name(&install, name, None)?;
        let proxy = proxy.trim();
        if !proxy.is_empty() && !valid_proxy(proxy) {
            return Err("请输入不含用户名、密码或路径的 HTTP、HTTPS 或 SOCKS5 代理地址。".to_string());
        }
        if !["", "en-US", "en-GB", "zh-CN", "zh-TW", "ja-JP", "ko-KR"].contains(&locale.as_str()) {
            return Err("不支持此语言设置。".to_string());
        }
        let p = profiles::create(&install, name)?;
        let mut args = Vec::new();


        if !locale.is_empty() { args.push(format!("--lang={locale}")); }
        if let Err(e) = settings::update(|s| { let acc = s.account_mut(&p.id); acc.args = args.join(" "); acc.proxy_endpoint = proxy.to_string(); }) {
            let _ = std::fs::remove_dir(&p.path);
            return Err(e);
        }
        if let Err(e) = launcher::launch(&install, &p) { return Err(format!("账号已创建，但启动失败：{e}")); }
        Ok(p)
    }).await.map_err(|e| e.to_string())?;
    after_change(&app);
    result
}

#[tauri::command]
async fn check_proxy(endpoint: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || proxy::verify(&endpoint)).await.map_err(|_| "检测任务失败。".to_string())?
}

#[tauri::command]
fn profile_options(app: AppHandle, id: String) -> Result<ProfileOptions, String> {
    profiles::options(&current_install(&app)?, &id)
}

#[tauri::command]
fn save_profile_options(app: AppHandle, id: String, options: ProfileOptions) -> Result<(), String> {
    profiles::save_options(&current_install(&app)?, &id, &options)?;
    after_change(&app);
    Ok(())
}

#[tauri::command]
fn delete_profile(app: AppHandle, id: String) -> Result<(), String> {
    profiles::remove(&current_install(&app)?, &id)?;
    after_change(&app);
    Ok(())
}

#[tauri::command]
fn open_profile_folder(app: AppHandle, id: String) -> Result<(), String> {
    let (_, p) = find(&app, &id)?;
    std::fs::create_dir_all(&p.path).ok();
    let opener = if cfg!(windows) { "explorer.exe" } else { "open" };
    std::process::Command::new(opener)
        .arg(&p.path)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn create_shortcut(app: AppHandle, id: String) -> Result<String, String> {
    let (_, p) = find(&app, &id)?;
    let path = launcher::create_shortcut(&p)?;
    Ok(format!("已在桌面创建快捷方式：{}", path.file_name().unwrap_or_default().to_string_lossy()))
}

#[tauri::command]
fn setup_login_router() -> Result<(), String> { login::register() }

#[tauri::command]
fn prepare_login(app: AppHandle, id: String) -> Result<(), String> {
    let (i, p) = find(&app, &id)?;
    login::arm(&p.id)?;
    launcher::launch(&i, &p)?;
    Ok(())
}

#[tauri::command]
fn diagnostics(app: AppHandle) -> String {
    launcher::diagnostics(current_install(&app))
}

#[tauri::command]
async fn local_usage() -> Result<usage::Usage, String> {
    tauri::async_runtime::spawn_blocking(usage::read).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn profile_quota(app: AppHandle, id: String) -> Result<quota::Quota, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (_, profile) = find(&app, &id)?;
        quota::fetch(&profile)
    }).await.map_err(|_| "额度查询任务失败。".to_string())?
}

#[tauri::command]
async fn save_transcript(app: AppHandle, text: String) -> Result<String, String> {
    use std::{fs::OpenOptions, io::Write};
    let dir = app.path().download_dir().map_err(|_| "无法定位下载目录。")?;
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::create_dir_all(&dir).map_err(|_| "无法访问下载目录。")?;
        let stamp = paths::now_millis();
        for i in 0..100 {
            let file = dir.join(format!("Claude-chat-{stamp}-{i}.txt"));
            match OpenOptions::new().write(true).create_new(true).open(&file) {
                Ok(mut out) => {
                    if out.write_all(text.as_bytes()).is_err() {
                        drop(out);
                        let _ = std::fs::remove_file(&file);
                        return Err("续聊附件写入失败。".to_string());
                    }
                    return Ok(file.to_string_lossy().into_owned());
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => return Err("无法在下载目录创建附件。".to_string()),
            }
        }
        Err("文件名冲突，请重新保存。".to_string())
    }).await.map_err(|_| "保存任务失败。".to_string())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    version: String,
    platform: String,
    install: Option<Install>,
    install_error: Option<String>,
    claude_path: Option<String>,
    storage_root: String,
    settings_path: String,
    autostart: bool,
}

#[tauri::command]
fn app_info(app: AppHandle) -> AppInfo {
    let (install, install_error) = match current_install(&app) {
        Ok(i) => (Some(i), None),
        Err(e) => (None, Some(e)),
    };
    let s = settings::load();
    AppInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        platform: std::env::consts::OS.into(),
        install,
        install_error,
        claude_path: s.claude_path,
        storage_root: paths::storage_root().to_string_lossy().into(),
        settings_path: paths::settings_path().to_string_lossy().into(),
        autostart: sys::autostart_enabled(),
    }
}

#[tauri::command]
fn set_autostart(enabled: bool) -> Result<(), String> {
    if enabled {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        sys::set_autostart(Some(&format!("\"{}\" --tray", exe.display())))
    } else {
        sys::set_autostart(None)
    }
}

#[tauri::command]
fn set_claude_path(app: AppHandle, path: Option<String>) -> Result<(), String> {
    let path = path
        .map(|p| p.trim().trim_matches('"').to_string())
        .filter(|p| !p.is_empty());
    if let Some(p) = &path {
        if !std::path::Path::new(p).exists() {
            return Err(format!("路径不存在：{p}"));
        }
    }
    settings::update(|s| {
        s.claude_path = path.clone();
        s.install_cache = None;
    })?;
    if let Ok(mut g) = app.state::<AppState>().install.lock() {
        *g = None;
    }
    current_install(&app).map(|_| ())
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let Some(cfg) = app.config().app.windows.iter().find(|w| w.label == "main").cloned() else { return };
    let Ok(builder) = WebviewWindowBuilder::from_config(app, &cfg) else { return };
    #[cfg(target_os = "macos")]
    let builder = builder
        .decorations(true)
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);
    if let Ok(w) = builder.build() {
        fit_to_screen(&w);
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1500));
            if let Some(w) = app.get_webview_window("main") {
                if !w.is_visible().unwrap_or(true) {
                    let _ = w.show();
                    let _ = w.unminimize();
                    let _ = w.set_focus();
                }
            }
        });
    }
}

fn fit_to_screen(w: &tauri::WebviewWindow) {
    let (Ok(Some(mon)), Ok(size)) = (w.current_monitor(), w.outer_size()) else { return };
    let area = mon.work_area().size;
    let max_w = (area.width as f64 * 0.9) as u32;
    let max_h = (area.height as f64 * 0.9) as u32;
    if size.width > max_w || size.height > max_h {
        let _ = w.set_size(tauri::PhysicalSize::new(size.width.min(max_w), size.height.min(max_h)));
        let _ = w.center();
    }
}

fn build_menu(app: &AppHandle, list: &[Profile]) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    for p in list {
        let mark = if p.pid.is_some() { "●" } else { "○" };
        menu.append(&MenuItem::with_id(app, format!("p:{}", p.id), format!("{mark}  {}", p.name), true, None::<&str>)?)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, "add", "添加账号…", true, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "show", "打开 Claude-Hydra", true, None::<&str>)?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?)?;
    Ok(menu)
}

fn signature(list: &[Profile]) -> String {
    list.iter().map(|p| format!("{}|{}|{:?};", p.id, p.name, p.pid)).collect()
}

fn refresh_tray(app: &AppHandle) -> Option<String> {
    let i = current_install(app).ok()?;
    let list = profiles::list(&i, true);
    if let (Some(tray), Ok(menu)) = (app.tray_by_id(TRAY_ID), build_menu(app, &list)) {
        let _ = tray.set_menu(Some(menu));
    }
    let _ = app.emit_to("main", "profiles-changed", &list);
    Some(signature(&list))
}

fn on_menu(app: &AppHandle, id: &str) {
    match id {
        "show" => show_main(app),
        "quit" => app.exit(0),
        "add" => {
            let app = app.clone();
            std::thread::spawn(move || {
                if let Err(e) = current_install(&app).and_then(|i| launcher::add_account(&i)) {
                    sys::error_box(&e);
                }
                refresh_tray(&app);
            });
        }
        other => {
            if let Some(pid) = other.strip_prefix("p:") {
                let (app, pid) = (app.clone(), pid.to_string());
                std::thread::spawn(move || {
                    if let Err(e) = find(&app, &pid).and_then(|(i, p)| launcher::launch(&i, &p)) {
                        sys::error_box(&e);
                    }
                    std::thread::sleep(Duration::from_secs(2));
                    refresh_tray(&app);
                });
            }
        }
    }
}

fn start_background(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last = String::new();
        let mut last_watch = Instant::now();
        loop {
            let open = app.get_webview_window("main").is_some();
            std::thread::sleep(Duration::from_secs(if open { 3 } else { 10 }));
            if let Ok(i) = current_install(&app) {
                let list = profiles::list(&i, true);
                #[cfg(windows)]
                for profile in &list {
                    if let Some(pid) = profile.pid {
                        taskbar::apply(profile, pid);
                    }
                }
                let sig = signature(&list);
                if sig != last {
                    if let (Some(tray), Ok(menu)) = (app.tray_by_id(TRAY_ID), build_menu(&app, &list)) {
                        let _ = tray.set_menu(Some(menu));
                    }
                    let _ = app.emit_to("main", "profiles-changed", &list);
                    last = sig;
                }
                if last_watch.elapsed() >= Duration::from_secs(60) {
                    last_watch = Instant::now();
                    if watchdog::any_kept() {
                        watchdog::run_once(&i);
                    }
                }
            }
        }
    });
}

pub fn run(start_in_tray: bool) {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            list_profiles,
            launch_profile,
            close_profile,
            restart_profile,
            add_account,
            next_account_name,
            create_configured_account,
            profile_options,
            check_proxy,
            save_profile_options,
            delete_profile,
            open_profile_folder,
            create_shortcut,
            setup_login_router,
            prepare_login,
            diagnostics,
            local_usage,
            profile_quota,
            save_transcript,
            app_info,
            set_autostart,
            set_claude_path,
            quit_app,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let list = current_install(&handle).map(|i| profiles::list(&i, true)).unwrap_or_default();
            let mut tray = TrayIconBuilder::with_id(TRAY_ID)
                .tooltip("Claude-Hydra")
                .menu(&build_menu(&handle, &list)?)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| on_menu(app, e.id().as_ref()))
                .on_tray_icon_event(|tray, e| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = e
                    {
                        show_main(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon().cloned() {
                tray = tray.icon(icon);
            }
            tray.build(app)?;
            start_background(handle.clone());
            if !start_in_tray {
                show_main(&handle);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Claude-Hydra")
        .run(|_app, event| {
            if let RunEvent::ExitRequested { code: None, api, .. } = event {
                api.prevent_exit();
            }
        });
}
