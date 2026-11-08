//! Account profile discovery, process state matching, and per-account configuration.

use crate::install::Install;
use crate::paths::{self, DEFAULT_ID};
use crate::settings::{self, Settings};
use crate::sys;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

pub const PALETTE_LEN: usize = 8;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub path: String,
    pub is_default: bool,
    pub pid: Option<u32>,
    pub last_used: Option<u64>,
    pub color: usize,
    pub keep_running: bool,
    pub signed_in: Option<bool>,
    /// Name of the assigned environment config.
    pub environment: Option<String>,
}

pub fn profile_path(install: &Install, id: &str) -> PathBuf {
    if id == DEFAULT_ID {
        install.default_profile.clone()
    } else {
        paths::extra_profile_dir(id)
    }
}

pub fn path_key(p: &str) -> String {
    p.trim_end_matches(['\\', '/']).replace('/', "\\").to_lowercase()
}

pub fn user_data_dir_arg(cmd: &str) -> Option<String> {
    const FLAG: &str = "--user-data-dir=";
    let i = cmd.find(FLAG)?;
    let after = &cmd[i + FLAG.len()..];
    let quoted_whole = i > 0 && cmd.as_bytes()[i - 1] == b'"';
    let value = if quoted_whole {
        after.split('"').next()?
    } else if let Some(rest) = after.strip_prefix('"') {
        rest.split('"').next()?
    } else {
        after.split_whitespace().next()?
    };
    (!value.is_empty()).then(|| value.to_string())
}

pub fn running_map(install: &Install) -> HashMap<String, u32> {
    let mut map = HashMap::new();
    let default_key = path_key(&install.default_profile.to_string_lossy());
    let exe_name = if cfg!(windows) { "claude.exe" } else { "claude" };
    for p in sys::processes() {
        if !p.name.eq_ignore_ascii_case(exe_name) {
            continue;
        }
        let Some(image) = sys::image_path(p.pid) else { continue };
        if !install.is_desktop_exe(&image) {
            continue;
        }
        let Some(cmd) = sys::command_line(p.pid) else { continue };
        if cmd.contains("--type=") {
            continue;
        }
        let key = user_data_dir_arg(&cmd).map(|d| path_key(&d)).unwrap_or_else(|| default_key.clone());
        map.entry(key).or_insert(p.pid);
    }
    map
}

fn last_used(path: &Path) -> Option<u64> {
    ["Preferences", "Local State", "config.json", "window-state.json", "Network/Cookies"]
        .iter()
        .filter_map(|f| std::fs::metadata(path.join(f)).ok()?.modified().ok())
        .max()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
}

fn signed_in(dir: &Path) -> Option<bool> {
    let bytes = std::fs::read(dir.join("config.json")).ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    if let Some(b) = v.get("windowSizeWasSignedIn").and_then(Value::as_bool) {
        return Some(b);
    }
    Some(
        v.get("oauth:tokenCacheV2")
            .and_then(Value::as_str)
            .map(|s| s.len() > 100)
            .unwrap_or(false),
    )
}

pub fn ids(install: &Install) -> Vec<String> {
    let _ = install;
    let mut found: Vec<String> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(paths::profiles_dir()) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.starts_with('.') && e.path().is_dir() && !name.is_empty() {
                found.push(name);
            }
        }
    }
    found.retain(|id| id != DEFAULT_ID);
    found.sort_by_key(|a| a.to_lowercase());
    found.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    let mut all = vec![DEFAULT_ID.to_string()];
    all.extend(found);
    all
}

fn resolve_colors(ids: &[String], s: &mut Settings) -> (HashMap<String, usize>, bool) {
    let mut changed = false;
    for id in ids {
        let acc = s.account(id);
        if acc.custom_color.is_some() || acc.assigned_color.is_some() {
            continue;
        }
        let mut use_count = [0usize; PALETTE_LEN];
        for existing_id in ids {
            if let Some(c) = s.accounts.get(existing_id).and_then(|a| a.assigned_color) {
                use_count[c % PALETTE_LEN] += 1;
            }
        }
        let pick = (0..PALETTE_LEN).min_by_key(|&i| (use_count[i], i)).unwrap_or(0);
        s.account_mut(id).assigned_color = Some(pick);
        changed = true;
    }
    let mut out = HashMap::new();
    for id in ids {
        let acc = s.account(id);
        let c = acc.custom_color.or(acc.assigned_color).unwrap_or(0) % PALETTE_LEN;
        out.insert(id.clone(), c);
    }
    (out, changed)
}

pub fn list(install: &Install, with_status: bool) -> Vec<Profile> {
    let ids = ids(install);
    let mut s = settings::load();
    let (colors, changed) = resolve_colors(&ids, &mut s);
    if changed {
        let _ = settings::save(&s);
    }
    let running = if with_status { running_map(install) } else { HashMap::new() };
    ids.into_iter()
        .map(|id| {
            let path = profile_path(install, &id);
            let pstr = path.to_string_lossy().to_string();
            let acc = s.account(&id);
            let name = acc
                .label
                .filter(|l| !l.trim().is_empty())
                .unwrap_or_else(|| id.clone());
            Profile {
                is_default: id == DEFAULT_ID,
                pid: running.get(&path_key(&pstr)).copied(),
                last_used: if with_status { last_used(&path) } else { None },
                signed_in: if with_status { signed_in(&path) } else { None },
                color: colors.get(&id).copied().unwrap_or(0),
                keep_running: acc.keep_running,
                environment: s.environment_name(&id),
                name,
                path: pstr,
                id,
            }
        })
        .collect()
}

pub fn resolve(install: &Install, name: &str, with_status: bool) -> Option<Profile> {
    let all = list(install, with_status);
    all.iter()
        .find(|p| p.id.eq_ignore_ascii_case(name))
        .or_else(|| all.iter().find(|p| p.name.eq_ignore_ascii_case(name)))
        .cloned()
}

pub fn validate_name(install: &Install, name: &str, except_id: Option<&str>) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("名称不能为空。".into());
    }
    if name.chars().any(|c| "\\/:*?\"<>|".contains(c) || (c as u32) < 0x20) {
        return Err("名称不能包含 \\ / : * ? \" < > |".into());
    }
    if name.starts_with('.') || name.ends_with('.') || name.ends_with(' ') {
        return Err("名称不能以句点开头，也不能以句点或空格结尾。".into());
    }
    if name.chars().count() > 40 {
        return Err("名称过长（最多 40 个字符）。".into());
    }
    for p in list(install, false) {
        if Some(p.id.as_str()) == except_id {
            continue;
        }
        if p.id.eq_ignore_ascii_case(name) || p.name.eq_ignore_ascii_case(name) {
            return Err(format!("名为“{name}”的账号已存在。"));
        }
    }
    if except_id.is_none() {
        let target = paths::extra_profile_dir(name);
        if target.exists() {
            return Err(format!("“{name}”已被 {} 占用。", target.display()));
        }
        let legacy = paths::storage_root().join(format!("ClaudeProfile-{name}"));
        let legacy_nested = paths::storage_root().join("ClaudeProfiles").join(name);
        if legacy.exists() || legacy_nested.exists() {
            return Err(format!("“{name}”已存在旧版数据目录，请使用其他名称。"));
        }
    }
    Ok(())
}

pub fn next_account_name(install: &Install) -> String {
    let current = list(install, false).iter().flat_map(|p| [&p.id, &p.name]).filter_map(|s| s.strip_prefix("Account ")?.parse::<u64>().ok()).max().unwrap_or(1);
    (current.saturating_add(1).max(2)..)
        .map(|n| format!("Account {n}"))
        .find(|n| validate_name(install, n, None).is_ok())
        .unwrap()
}

pub fn create(install: &Install, name: &str) -> Result<Profile, String> {
    validate_name(install, name, None)?;
    let path = profile_path(install, name);
    std::fs::create_dir_all(&path).map_err(|e| format!("无法创建账号目录：{e}"))?;
    resolve(install, name, false).ok_or_else(|| "账号已创建但未能读取。".into())
}

pub fn remove(install: &Install, id: &str) -> Result<(), String> {
    if id == DEFAULT_ID {
        return Err("不能删除默认 (Default) 账号。".into());
    }
    let path = profile_path(install, id);
    if id.starts_with('.') || id.contains(['\\', '/']) || path != paths::extra_profile_dir(id) {
        return Err(format!("拒绝删除非受管路径 {}。", path.display()));
    }
    if running_map(install).contains_key(&path_key(&path.to_string_lossy())) {
        return Err("删除前请先关闭这个账号的 Claude 窗口。".into());
    }
    if path.exists() {
        std::fs::remove_dir_all(&path).map_err(|e| format!("删除失败：{e}"))?;
    }
    settings::update(|s| s.remove_account(id))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileOptions {
    pub name: String,
    /// -1 indicates automatic palette assignment.
    pub color: i32,
    pub keep_running: bool,
    /// Assigned environment id; "" = none (direct connection, system timezone).
    #[serde(default)]
    pub environment: String,
    /// Last exit seen at launch; read-only for the UI.
    #[serde(default, skip_deserializing)]
    pub last_exit: Option<crate::node::ExitInfo>,
}

fn valid_env_name(n: &str) -> bool {
    let mut c = n.chars();
    matches!(c.next(), Some(f) if f.is_ascii_alphabetic() || f == '_')
        && c.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

pub fn env_vars(s: &Settings, id: &str) -> Vec<(String, String)> {
    s.effective(id).env.into_iter().filter(|(k, _)| valid_env_name(k)).collect()
}

pub fn extra_args(s: &Settings, id: &str) -> String {
    s.effective(id).args
}

pub fn options(install: &Install, id: &str) -> Result<ProfileOptions, String> {
    let p = resolve(install, id, false).ok_or("找不到这个账号。")?;
    let s = settings::load();
    let acc = s.account(id);
    Ok(ProfileOptions {
        name: p.name,
        color: acc.custom_color.map(|v| (v % PALETTE_LEN) as i32).unwrap_or(-1),
        keep_running: p.keep_running,
        environment: acc.environment.clone().filter(|e| s.environments.contains_key(e)).unwrap_or_default(),
        last_exit: acc.last_exit.clone(),
    })
}

/// "" / "proxy" → follow the node; "system" → OS zone; otherwise a validated IANA name.
pub fn normalize_timezone(tz: &str) -> Result<String, String> {
    let tz = tz.trim();
    Ok(match tz {
        "" | "proxy" => String::new(),
        "system" => "system".into(),
        _ if crate::node::valid_timezone(tz) => tz.to_string(),
        _ => return Err(format!("时区“{tz}”无效，请使用 IANA 名称，例如 Asia/Tokyo。")),
    })
}

pub fn parse_env_lines(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut vars = BTreeMap::new();
    let mut bad = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        match l.split_once('=') {
            Some((k, v)) if valid_env_name(k.trim()) => {
                vars.insert(k.trim().to_string(), v.to_string());
            }
            _ => bad.push(l.to_string()),
        }
    }
    if bad.is_empty() {
        Ok(vars)
    } else {
        Err(format!("以下环境变量行无效（格式应为 名称=值）：\n{}", bad.join("\n")))
    }
}

/// Checks that an account may use an environment (store default accounts can't be customised).
pub fn check_environment(install: &Install, s: &Settings, account_id: &str, environment: &str) -> Result<Option<String>, String> {
    if environment.is_empty() { return Ok(None); }
    let env = s.environments.get(environment).ok_or("所选环境配置已不存在。")?;
    if account_id == DEFAULT_ID && install.is_msix() && env.customizes_launch() {
        return Err("商店版默认账号由系统启动，无法应用代理、时区或启动参数，不能使用此环境。".into());
    }
    Ok(Some(environment.to_string()))
}

pub fn save_options(install: &Install, id: &str, o: &ProfileOptions) -> Result<(), String> {
    let current = resolve(install, id, false).ok_or("找不到这个账号。")?;
    let name = o.name.trim();
    if name != current.name {
        validate_name(install, name, Some(id))?;
    }
    let s = settings::load();
    let environment = check_environment(install, &s, id, &o.environment)?;
    settings::update(|s| {
        let acc = s.account_mut(id);
        acc.label = (!name.is_empty() && name != id).then(|| name.to_string());
        acc.custom_color = (o.color >= 0).then(|| (o.color as usize) % PALETTE_LEN);
        acc.keep_running = o.keep_running;
        if acc.environment != environment { acc.last_exit = None; }
        acc.environment = environment.clone();
        // Launch settings now live in environments; drop any leftover per-account copy.
        acc.set_node(None);
        acc.args.clear();
        acc.env.clear();
        acc.timezone.clear();
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_user_data_dir_form() {
        assert_eq!(user_data_dir_arg(r#"Claude.exe --user-data-dir="D:\a b\c" x"#).as_deref(), Some(r"D:\a b\c"));
        assert_eq!(
            user_data_dir_arg(r#"Claude.exe "--user-data-dir=D:\a b\c" --restarted-by-app"#).as_deref(),
            Some(r"D:\a b\c")
        );
        assert_eq!(user_data_dir_arg(r"Claude.exe --user-data-dir=D:\abc --x").as_deref(), Some(r"D:\abc"));
        assert_eq!(user_data_dir_arg("Claude.exe"), None);
    }

    #[test]
    fn env_lines_skip_comments_and_report_bad_ones() {
        let ok = parse_env_lines("# c\nHTTPS_PROXY=http://x:1\n\nA_1=b=c").unwrap();
        assert_eq!(ok["HTTPS_PROXY"], "http://x:1");
        assert_eq!(ok["A_1"], "b=c");
        assert!(parse_env_lines("1BAD=x").is_err());
        assert!(parse_env_lines("novalue").is_err());
    }

    #[test]
    fn path_keys_ignore_case_and_trailing_slash() {
        assert_eq!(
            path_key(r"D:\ClaudeHydra\profiles\Work\"),
            path_key(r"d:\claudehydra\profiles\work")
        );
    }
}
