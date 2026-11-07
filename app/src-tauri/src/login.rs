//! One-shot routing of browser sign-in callbacks to an explicitly selected profile.
use crate::{install, paths, profiles};
use std::process::Command;

fn marker() -> std::path::PathBuf { paths::storage_root().join("pending-login.json") }

pub fn arm(id: &str) -> Result<(), String> {
    std::fs::create_dir_all(paths::storage_root()).map_err(|e| e.to_string())?;
    let data = serde_json::json!({"id": id, "expires": paths::now_secs() + 600});
    std::fs::write(marker(), data.to_string()).map_err(|e| format!("无法准备登录：{e}"))
}

fn valid_url(url: &str) -> bool {
    url.starts_with("claude://") && url.len() <= 16384 && !url.chars().any(|c| c.is_control() || c.is_whitespace() || c == '"')
}

pub fn route(url: &str) -> Result<(), String> {
    if !valid_url(url) { return Err("无效的 Claude 回调地址。".into()); }
    // Consume the marker before launching so that a callback cannot be replayed.
    let path = marker();
    let claimed = path.with_extension(format!("claimed-{}", std::process::id()));
    std::fs::rename(&path, &claimed).map_err(|_| "没有待处理的登录请求或请求已被处理。请在账号菜单中重新选择登录。".to_string())?;
    let bytes = std::fs::read(&claimed).map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&claimed);
    let data: serde_json::Value = serde_json::from_slice(&bytes?).map_err(|_| "登录请求数据无效。")?;
    let expiry = data["expires"].as_u64().ok_or("登录请求数据无效。")?;
    if paths::now_secs() > expiry { return Err("登录请求已过期，请重新选择登录。".into()); }
    let id = data["id"].as_str().ok_or("登录请求数据无效。")?;
    let i = install::resolve()?;
    let p = profiles::resolve(&i, id, false).ok_or("登录目标账号已不存在。")?;
    let exe = i.launch_exe().ok_or("找不到 Claude 启动程序。")?;
    let mut cmd = Command::new(exe);
    if !p.is_default { cmd.arg(format!("--user-data-dir={}", p.path)); }
    cmd.arg(url).spawn().map_err(|e| format!("无法转发登录回调：{e}"))?;
    Ok(())
}

#[cfg(windows)]
pub fn register() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let base = r"HKCU\Software\Classes\ClaudeHydra.Callback";
    let capabilities = r"Software\ClaudeHydra\Capabilities";
    let command = format!("\"{}\" --login-callback \"%1\"", exe.display());
    let entries = [
        (base.to_string(), "/ve", "Claude-Hydra 登录回调".to_string()),
        (base.to_string(), "/v URL Protocol", String::new()),
        (format!(r"{base}\shell\open\command"), "/ve", command),
        (format!(r"HKCU\{capabilities}"), "/v ApplicationName", "Claude-Hydra 登录回调".into()),
        (format!(r"HKCU\{capabilities}\URLAssociations"), "/v claude", "ClaudeHydra.Callback".into()),
        (r"HKCU\Software\RegisteredApplications".into(), "/v ClaudeHydra", capabilities.into()),
    ];
    for (key, name, value) in entries {
        let mut cmd = Command::new("reg.exe");
        cmd.arg("add").arg(key);
        if name == "/ve" { cmd.arg("/ve"); } else { cmd.args(["/v", name.trim_start_matches("/v ")]); }
        let status = cmd.args(["/t", "REG_SZ", "/d", &value, "/f"]).status().map_err(|e| e.to_string())?;
        if !status.success() { return Err("无法注册登录回调程序。".into()); }
    }
    Command::new("explorer.exe").arg("ms-settings:defaultapps").spawn().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(not(windows))]
pub fn register() -> Result<(), String> { Err("登录回调路由目前仅支持 Windows。".into()) }

#[cfg(test)]
mod tests {
    #[test]
    fn rejects_invalid_links() {
        assert!(super::valid_url("claude://sso-callback?code=abc"));
        assert!(!super::valid_url("https://example.com"));
        assert!(!super::valid_url("claude://sso-callback?x=\"bad"));
    }
}
