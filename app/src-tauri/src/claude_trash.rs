//! Hydra's recycle bin for things deleted from a Claude Code data folder (sessions, skills,
//! agents, commands). Kept in Hydra's data directory rather than in the folder itself, so
//! Claude Code never sees it; each entry remembers where its files came from.
use crate::claude_folders::{self, Folder};
use serde::{Deserialize, Serialize};
use std::{fs, path::{Path, PathBuf}};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Trashed {
    #[serde(default)]
    pub id: String,
    /// `session`, `skill`, `agent` or `command`.
    pub kind: String,
    pub name: String,
    /// The session's working folder, or the item's path in the data folder.
    pub detail: Option<String>,
    pub deleted_at: u64,
    /// Paths relative to the data folder.
    paths: Vec<String>,
}

const MANIFEST: &str = "hydra-trash.json";

fn root(f: &Folder) -> PathBuf {
    // Tests must not touch the real data directory, and setting `CLAUDE_HYDRA_ROOT` would leak
    // into tests running in parallel.
    #[cfg(test)]
    let base = std::env::temp_dir().join(format!("hydra-test-data-{}", std::process::id()));
    #[cfg(not(test))]
    let base = crate::paths::storage_root();
    base.join("claude-trash").join(claude_folders::key(f))
}

/// A relative path made only of plain names: nothing can climb out of the folder.
pub fn plain(rel: &str) -> bool {
    !rel.is_empty() && rel.split('/').all(|p| !p.is_empty() && p != "." && p != ".." && !p.contains('\\') && !p.contains(':'))
}

/// Renames `from` to `to`, copying across volumes when a rename cannot.
pub fn move_path(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(parent) = to.parent() { fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
    if fs::rename(from, to).is_ok() { return Ok(()); }
    if let Err(e) = copy_tree(from, to) {
        let _ = if to.is_dir() { fs::remove_dir_all(to) } else { fs::remove_file(to) };
        return Err(e.to_string());
    }
    if from.is_dir() { fs::remove_dir_all(from) } else { fs::remove_file(from) }.map_err(|e| e.to_string())
}

/// Copies a file or folder; symbolic links are skipped rather than followed.
pub fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    let meta = fs::symlink_metadata(from)?;
    if meta.is_dir() {
        fs::create_dir_all(to)?;
        for e in fs::read_dir(from)? {
            let e = e?;
            copy_tree(&e.path(), &to.join(e.file_name()))?;
        }
    } else if meta.is_file() {
        if let Some(parent) = to.parent() { fs::create_dir_all(parent)?; }
        fs::copy(from, to)?;
    }
    Ok(())
}

/// Moves `paths` (relative to the folder; missing ones are ignored) into a new trash entry.
pub fn put(f: &Folder, kind: &str, name: &str, detail: Option<String>, paths: &[String]) -> Result<(), String> {
    let present: Vec<String> = paths.iter().filter(|p| plain(p) && fs::symlink_metadata(f.dir.join(p)).is_ok()).cloned().collect();
    if present.is_empty() { return Err("要删除的内容已不存在，请刷新。".into()); }
    let entry = root(f).join(format!("{}-{}", crate::paths::now_millis(), uuid::Uuid::new_v4().simple()));
    fs::create_dir_all(&entry).map_err(|e| format!("无法创建回收站：{e}"))?;
    let record = Trashed { id: String::new(), kind: kind.into(), name: name.into(), detail, deleted_at: crate::paths::now_millis(), paths: present.clone() };
    fs::write(entry.join(MANIFEST), serde_json::to_vec_pretty(&record).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut moved = Vec::new();
    for p in &present {
        if let Err(e) = move_path(&f.dir.join(p), &entry.join("files").join(p)) {
            // Put back what already moved, so a half-deleted item is not left behind.
            for done in moved { let _ = move_path(&entry.join("files").join(done), &f.dir.join(done)); }
            let _ = fs::remove_dir_all(&entry);
            return Err(format!("无法移到回收站：{e}"));
        }
        moved.push(p.as_str());
    }
    Ok(())
}

pub fn list(f: &Folder) -> Vec<Trashed> {
    let mut out: Vec<Trashed> = fs::read_dir(root(f)).into_iter().flatten().flatten().filter_map(|e| {
        let mut t: Trashed = serde_json::from_slice(&fs::read(e.path().join(MANIFEST)).ok()?).ok()?;
        t.id = e.file_name().to_string_lossy().into_owned();
        Some(t)
    }).collect();
    out.sort_by_key(|t| std::cmp::Reverse(t.deleted_at));
    out
}

fn entry(f: &Folder, id: &str) -> Result<PathBuf, String> {
    if !plain(id) || id.contains('/') { return Err("回收站里没有这一项。".into()); }
    let dir = root(f).join(id);
    if dir.join(MANIFEST).is_file() { Ok(dir) } else { Err("回收站里没有这一项，请刷新。".into()) }
}

/// Moves an entry back; refuses when anything is in the way, and never overwrites.
pub fn restore(f: &Folder, id: &str) -> Result<(), String> {
    let dir = entry(f, id)?;
    let t: Trashed = serde_json::from_slice(&fs::read(dir.join(MANIFEST)).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if !t.paths.iter().all(|p| plain(p)) { return Err("回收站记录无效。".into()); }
    if let Some(p) = t.paths.iter().find(|p| fs::symlink_metadata(f.dir.join(p)).is_ok()) {
        return Err(format!("原位置已有同名内容（{p}），没有恢复。"));
    }
    for p in &t.paths { move_path(&dir.join("files").join(p), &f.dir.join(p))?; }
    fs::remove_dir_all(&dir).map_err(|e| e.to_string())
}

pub fn empty(f: &Folder) -> Result<(), String> {
    let root = root(f);
    if !root.exists() { return Ok(()); }
    fs::remove_dir_all(&root).map_err(|e| format!("无法清空回收站：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude_folders::Paths;

    #[test]
    fn plain_paths_cannot_climb_out() {
        // Arrange
        let bad = ["", "..", "a/../b", "/abs", "a//b", "a\\b", "C:x", "./a"];

        // Act
        let rejected = bad.iter().all(|p| !plain(p));

        // Assert
        assert!(rejected);
        assert!(plain("projects/-Users-u-app/1.jsonl"));
    }

    #[test]
    fn put_then_restore_returns_the_files() {
        // Arrange
        let base = std::env::temp_dir().join(format!("hydra-trash-{}", uuid::Uuid::new_v4().simple()));
        let dir = base.join("claude");
        fs::create_dir_all(dir.join("skills/s")).unwrap();
        fs::write(dir.join("skills/s/SKILL.md"), "x").unwrap();
        let f = Folder { id: "local".into(), kind: "local", name: "t".into(), path: String::new(), shared_with_local: false, profile: None,
            global_config: dir.join(".claude.json"), dir: dir.clone(), paths: Paths::Host };

        // Act
        put(&f, "skill", "s", None, &["skills/s".into()]).unwrap();
        let gone = !dir.join("skills/s").exists();
        let listed = list(&f);
        restore(&f, &listed[0].id).unwrap();

        // Assert
        assert!(gone);
        assert_eq!(listed.len(), 1);
        assert_eq!(fs::read_to_string(dir.join("skills/s/SKILL.md")).unwrap(), "x");
        assert!(list(&f).is_empty());
        let _ = fs::remove_dir_all(base);
    }
}
