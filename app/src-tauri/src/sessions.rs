//! Copy Desktop Claude Code session indexes, preserving originals and collisions.
use crate::{install::Install, profiles::{self, Profile}};
use serde::Serialize;
use std::{fs::{self, OpenOptions}, io::Write, path::{Path, PathBuf}};

#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct Session { id: String, title: String }
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct TransferResult { copied: usize, skipped: usize, failed: usize }

fn root(install: &Install, p: &Profile) -> PathBuf {
    let path = PathBuf::from(&p.path);
    #[cfg(windows)]
    if install.is_msix() {
        if let (Some(local), Some(cache)) = (std::env::var_os("LOCALAPPDATA"), install.default_profile.parent().and_then(Path::parent)) {
            if let Ok(relative) = path.strip_prefix(local) {
                let redirected = cache.join("Local").join(relative).join("claude-code-sessions");
                if redirected.is_dir() { return redirected; }
            }
        }
    }
    let _ = install;
    path.join("claude-code-sessions")
}
fn dirs(path: &Path) -> Result<Vec<PathBuf>, String> {
    if !path.exists() { return Ok(Vec::new()); }
    let mut out = Vec::new();
    for e in fs::read_dir(path).map_err(|_| "无法读取会话目录。")? {
        let e = e.map_err(|_| "无法读取会话目录。")?;
        if e.file_type().map_err(|_| "无法读取会话目录。")?.is_dir() { out.push(e.path()); }
    }
    out.sort(); Ok(out)
}
fn stores(path: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out=Vec::new(); for account in dirs(path)? { out.extend(dirs(&account)?); } Ok(out)
}
fn collect(path: &Path) -> Result<Vec<(Session, PathBuf, String, String)>, String> {
    let mut out = Vec::new();
    for store in stores(path)? {
        let Some(account) = store.parent().and_then(Path::file_name).and_then(|x| x.to_str()) else { continue };
        let Some(org) = store.file_name().and_then(|x| x.to_str()) else { continue };
        for entry in fs::read_dir(&store).map_err(|_| "无法读取会话目录。")? {
            let entry = entry.map_err(|_| "无法读取会话目录。")?;
            if !entry.file_type().map_err(|_| "无法读取会话文件。")?.is_file() { continue; }
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("local_") || !name.ends_with(".json") { continue; }
            let title = fs::read(entry.path()).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()).and_then(|v| ["title", "name", "summary", "customTitle", "displayName"].iter().find_map(|k| v.get(*k).and_then(|x| x.as_str()).filter(|x| !x.trim().is_empty()).map(str::to_owned))).unwrap_or_else(|| name.clone());
            out.push((Session { id: format!("{account}/{org}/{name}"), title }, entry.path(), account.to_owned(), org.to_owned()));
        }
    }
    out.sort_by(|a,b| a.0.title.cmp(&b.0.title)); Ok(out)
}
pub fn list(install: &Install, source: &Profile) -> Result<Vec<Session>, String> {
    Ok(collect(&root(install, source))?.into_iter().map(|x| x.0).collect())
}
fn replace_values(v: &mut serde_json::Value, old_account: &str, old_org: &str, account: &str, org: &str) {
    match v {
        serde_json::Value::String(s) => { if s == old_account { *s = account.to_owned(); } else if s == old_org { *s = org.to_owned(); } }
        serde_json::Value::Array(a) => for x in a { replace_values(x, old_account, old_org, account, org); },
        serde_json::Value::Object(o) => for x in o.values_mut() { replace_values(x, old_account, old_org, account, org); },
        _ => ()
    }
}
pub fn copy(install: &Install, source: &Profile, target: &Profile, ids: &[String]) -> Result<TransferResult,String> {
    if source.id == target.id { return Err("来源和目标账号不能相同。".into()); }
    if ids.is_empty() { return Err("请选择会话。".into()); }
    let running = profiles::running_map(install);
    if running.contains_key(&profiles::path_key(&source.path)) || running.contains_key(&profiles::path_key(&target.path)) { return Err("为避免正在写入的会话损坏，请自行关闭来源和目标账号后再迁移。Hydra 不会自动关闭它们。".into()); }
    let target_stores = stores(&root(install,target))?;
    if target_stores.len() != 1 { return Err("目标账号尚无会话目录或包含多个身份，无法确定目标。请先在目标账号创建一个 Claude Code 会话，并确认只有一个身份。".into()); }
    let dest = &target_stores[0];
    let account = dest.parent().and_then(Path::file_name).and_then(|v|v.to_str()).ok_or("目标账号路径无效。")?;
    let org = dest.file_name().and_then(|v|v.to_str()).ok_or("目标组织路径无效。")?;
    let available = collect(&root(install,source))?;
    let mut result=TransferResult {copied:0,skipped:0,failed:0};
    for id in ids {
        let Some((_,path,old_account,old_org)) = available.iter().find(|x| &x.0.id == id) else { result.failed+=1; continue };
        let Some(name)=path.file_name() else { result.failed+=1; continue };
        let Ok(bytes)=fs::read(path) else { result.failed+=1; continue };
        let Ok(mut json)=serde_json::from_slice::<serde_json::Value>(&bytes) else { result.failed+=1; continue };
        replace_values(&mut json,old_account,old_org,account,org);
        let Ok(data)=serde_json::to_vec_pretty(&json) else { result.failed+=1; continue };
        match OpenOptions::new().write(true).create_new(true).open(dest.join(name)) {
            Ok(mut f) => { if f.write_all(&data).is_ok() {result.copied+=1} else { drop(f); let _=fs::remove_file(dest.join(name)); result.failed+=1; } },
            Err(e) if e.kind()==std::io::ErrorKind::AlreadyExists => result.skipped+=1,
            Err(_) => result.failed+=1,
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_is_limited_to_complete_json_values() {
        let mut v = serde_json::json!({"account":"a", "nested":["org-a", "path/a/org-a"], "a":"a"});
        replace_values(&mut v, "a", "org-a", "b", "org-b");
        assert_eq!(v["account"], "b");
        assert_eq!(v["nested"][0], "org-b");
        assert_eq!(v["nested"][1], "path/a/org-a");
        assert_eq!(v["a"], "b");
    }
    #[test]
    fn detects_only_session_files_in_store() {
        let root = std::env::temp_dir().join(format!("hydra-session-test-{}", std::process::id()));
        let store = root.join("account").join("org");
        fs::create_dir_all(&store).unwrap();
        fs::write(store.join("local_1.json"), br#"{"title":"Example"}"#).unwrap();
        fs::write(store.join("other.json"), b"{}").unwrap();
        let sessions = collect(&root).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].0.title, "Example");
        fs::remove_dir_all(root).unwrap();
    }
}
