//! Persistent application settings stored in `settings.json`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CachedInstall {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exe: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aumid: Option<String>,
    pub default_profile_path: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_color: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_color: Option<usize>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub args: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub proxy_endpoint: String,
    #[serde(default)]
    pub keep_running: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_cache: Option<CachedInstall>,
    #[serde(default)]
    pub accounts: BTreeMap<String, AccountConfig>,
}

impl Settings {
    pub fn account(&self, id: &str) -> AccountConfig {
        self.accounts.get(id).cloned().unwrap_or_default()
    }

    pub fn account_mut(&mut self, id: &str) -> &mut AccountConfig {
        self.accounts.entry(id.to_string()).or_default()
    }

    pub fn remove_account(&mut self, id: &str) {
        self.accounts.remove(id);
    }
}

pub fn load() -> Settings {
    load_from(&crate::paths::settings_path())
}

pub fn load_from(path: &Path) -> Settings {
    let Ok(bytes) = std::fs::read(path) else {
        return Settings::default();
    };
    serde_json::from_slice::<Settings>(&bytes).unwrap_or_default()
}

pub fn save(settings: &Settings) -> Result<(), String> {
    save_to(&crate::paths::settings_path(), settings)
}

pub fn save_to(path: &Path, settings: &Settings) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("无法创建配置目录：{e}"))?;
    }
    let json = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &json).map_err(|e| format!("无法写入设置文件：{e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("无法保存设置文件：{e}"))
}

pub fn update(f: impl FnOnce(&mut Settings)) -> Result<(), String> {
    let mut s = load();
    f(&mut s);
    save(&s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_typed_settings() {
        let dir = std::env::temp_dir().join(format!("hydra-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("settings.json");

        let mut s = Settings::default();
        let acc = s.account_mut("Account 2");
        acc.label = Some("工作".into());
        acc.custom_color = Some(3);
        acc.keep_running = true;
        save_to(&file, &s).unwrap();

        let loaded = load_from(&file);
        let loaded_acc = loaded.account("Account 2");
        assert_eq!(loaded_acc.label.as_deref(), Some("工作"));
        assert_eq!(loaded_acc.custom_color, Some(3));
        assert!(loaded_acc.keep_running);

        let _ = std::fs::remove_dir_all(dir);
    }
}
