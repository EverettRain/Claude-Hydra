//! Copy Desktop Claude Code session indexes between accounts.
//!
//! Only `claude-code-sessions/<account>/<org>/local_*.json` index files are copied; the
//! transcripts themselves stay where Claude Code wrote them. Originals are never moved or
//! overwritten, and both accounts must be closed so no index is copied mid-write.
use crate::{install::Install, profiles::{self, Profile}};
use serde::Serialize;
use serde_json::Value;
use std::{fs::{self, OpenOptions}, io::Write, path::{Path, PathBuf}};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    id: String,
    title: String,
    cwd: Option<String>,
    cwd_exists: bool,
    last_activity_at: Option<u64>,
    /// Whether the target account already has an index with the same file name.
    in_target: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    sessions: Vec<Session>,
    /// Why the target cannot receive sessions yet; `None` when copying is possible.
    target_problem: Option<String>,
    running: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferResult { copied: usize, skipped: usize, failed: usize }

struct Entry { session: Session, path: PathBuf, account: String, org: String }

fn root(install: &Install, p: &Profile) -> PathBuf {
    let path = PathBuf::from(&p.path);
    #[cfg(windows)]
    if install.is_msix() {
        // Store builds redirect %LOCALAPPDATA% writes into the package's LocalCache.
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
    out.sort();
    Ok(out)
}

fn stores(path: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    for account in dirs(path)? { out.extend(dirs(&account)?); }
    Ok(out)
}

fn text(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| v.get(*k).and_then(Value::as_str).filter(|x| !x.trim().is_empty()).map(str::to_owned))
}

fn collect(path: &Path) -> Result<Vec<Entry>, String> {
    let mut out = Vec::new();
    for store in stores(path)? {
        let Some(account) = store.parent().and_then(Path::file_name).and_then(|x| x.to_str()) else { continue };
        let Some(org) = store.file_name().and_then(|x| x.to_str()) else { continue };
        for entry in fs::read_dir(&store).map_err(|_| "无法读取会话目录。")? {
            let entry = entry.map_err(|_| "无法读取会话目录。")?;
            if !entry.file_type().map_err(|_| "无法读取会话文件。")?.is_file() { continue; }
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("local_") || !name.ends_with(".json") { continue; }
            let json = fs::read(entry.path()).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).unwrap_or(Value::Null);
            let cwd = text(&json, &["cwd", "originCwd"]);
            out.push(Entry {
                session: Session {
                    id: format!("{account}/{org}/{name}"),
                    title: text(&json, &["title", "name", "summary", "customTitle", "displayName"]).unwrap_or_else(|| name.clone()),
                    cwd_exists: cwd.as_deref().map(|c| Path::new(c).is_dir()).unwrap_or(false),
                    cwd,
                    last_activity_at: json.get("lastActivityAt").and_then(Value::as_u64),
                    in_target: false,
                },
                path: entry.path(),
                account: account.to_owned(),
                org: org.to_owned(),
            });
        }
    }
    out.sort_by(|a, b| b.session.last_activity_at.cmp(&a.session.last_activity_at).then_with(|| a.session.title.cmp(&b.session.title)));
    Ok(out)
}

/// The single identity store in the target account, or a user-facing reason why there isn't one.
fn target_store(install: &Install, target: &Profile) -> Result<PathBuf, String> {
    let mut found = stores(&root(install, target))?;
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err("目标账号还没有会话目录。请先在目标账号的 Claude Code 中新建一个会话，再回来迁移。".into()),
        _ => Err("目标账号包含多个登录身份，无法确定会话应归属哪一个，Hydra 不会猜测。".into()),
    }
}

fn is_running(install: &Install, profiles: &[&Profile]) -> bool {
    let running = profiles::running_map(install);
    profiles.iter().any(|p| running.contains_key(&profiles::path_key(&p.path)))
}

pub fn preview(install: &Install, source: &Profile, target: &Profile) -> Result<Preview, String> {
    if source.id == target.id { return Err("来源和目标账号不能相同。".into()); }
    let mut sessions: Vec<Session> = collect(&root(install, source))?.into_iter().map(|e| e.session).collect();
    let store = target_store(install, target);
    if let Ok(dest) = &store {
        for s in &mut sessions {
            let name = s.id.rsplit('/').next().unwrap_or_default();
            s.in_target = dest.join(name).exists();
        }
    }
    Ok(Preview { sessions, target_problem: store.err(), running: is_running(install, &[source, target]) })
}

fn replace_values(v: &mut Value, old_account: &str, old_org: &str, account: &str, org: &str) {
    match v {
        Value::String(s) => { if s == old_account { *s = account.to_owned(); } else if s == old_org { *s = org.to_owned(); } }
        Value::Array(a) => for x in a { replace_values(x, old_account, old_org, account, org); },
        Value::Object(o) => for x in o.values_mut() { replace_values(x, old_account, old_org, account, org); },
        _ => (),
    }
}

fn copy_into(available: &[Entry], dest: &Path, ids: &[String]) -> Result<TransferResult, String> {
    let account = dest.parent().and_then(Path::file_name).and_then(|v| v.to_str()).ok_or("目标账号路径无效。")?;
    let org = dest.file_name().and_then(|v| v.to_str()).ok_or("目标组织路径无效。")?;
    let mut result = TransferResult { copied: 0, skipped: 0, failed: 0 };
    for id in ids {
        let Some(entry) = available.iter().find(|x| &x.session.id == id) else { result.failed += 1; continue };
        let Some(name) = entry.path.file_name() else { result.failed += 1; continue };
        let Ok(bytes) = fs::read(&entry.path) else { result.failed += 1; continue };
        let Ok(mut json) = serde_json::from_slice::<Value>(&bytes) else { result.failed += 1; continue };
        replace_values(&mut json, &entry.account, &entry.org, account, org);
        let Ok(data) = serde_json::to_vec_pretty(&json) else { result.failed += 1; continue };
        let file = dest.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&file) {
            Ok(mut f) => {
                if f.write_all(&data).and_then(|_| f.sync_all()).is_ok() { result.copied += 1 } else { drop(f); let _ = fs::remove_file(&file); result.failed += 1; }
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => result.skipped += 1,
            Err(_) => result.failed += 1,
        }
    }
    Ok(result)
}

pub fn copy(install: &Install, source: &Profile, target: &Profile, ids: &[String]) -> Result<TransferResult, String> {
    if source.id == target.id { return Err("来源和目标账号不能相同。".into()); }
    if ids.is_empty() { return Err("请选择会话。".into()); }
    if is_running(install, &[source, target]) {
        return Err("为避免正在写入的会话损坏，请先自行关闭来源和目标账号再迁移。Hydra 不会自动关闭它们。".into());
    }
    let dest = target_store(install, target)?;
    copy_into(&collect(&root(install, source))?, &dest, ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hydra-session-{name}-{}-{}", std::process::id(), crate::paths::now_millis()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

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
        let root = temp("detect");
        let store = root.join("account").join("org");
        fs::create_dir_all(&store).unwrap();
        fs::write(store.join("local_1.json"), br#"{"title":"Example","cwd":"/nope","lastActivityAt":5}"#).unwrap();
        fs::write(store.join("other.json"), b"{}").unwrap();
        let sessions = collect(&root).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session.title, "Example");
        assert_eq!(sessions[0].session.last_activity_at, Some(5));
        assert!(!sessions[0].session.cwd_exists);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn copy_rewrites_identity_and_never_overwrites() {
        let src = temp("src");
        let dst = temp("dst");
        let from = src.join("acc-a").join("org-a");
        let to = dst.join("acc-b").join("org-b");
        fs::create_dir_all(&from).unwrap();
        fs::create_dir_all(&to).unwrap();
        fs::write(from.join("local_1.json"), br#"{"title":"One","account":"acc-a","org":"org-a"}"#).unwrap();
        fs::write(from.join("local_2.json"), br#"{"title":"Two"}"#).unwrap();
        fs::write(to.join("local_2.json"), br#"{"title":"Keep"}"#).unwrap();
        let entries = collect(&src).unwrap();
        let ids: Vec<String> = entries.iter().map(|e| e.session.id.clone()).chain(["acc-a/org-a/missing.json".to_string()]).collect();
        let r = copy_into(&entries, &to, &ids).unwrap();
        assert_eq!((r.copied, r.skipped, r.failed), (1, 1, 1));
        let copied: Value = serde_json::from_slice(&fs::read(to.join("local_1.json")).unwrap()).unwrap();
        assert_eq!(copied["account"], "acc-b");
        assert_eq!(copied["org"], "org-b");
        assert!(from.join("local_1.json").exists(), "source must be kept");
        let kept: Value = serde_json::from_slice(&fs::read(to.join("local_2.json")).unwrap()).unwrap();
        assert_eq!(kept["title"], "Keep");
        fs::remove_dir_all(src).unwrap();
        fs::remove_dir_all(dst).unwrap();
    }
}
