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
    /// Legacy (v3.0) unauthenticated proxy entrance; migrated into `node` on read.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub proxy_endpoint: String,
    /// Proxy node; its password is stored sealed (see `node::seal`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<crate::node::NodeConfig>,
    /// "" / "proxy" follow the node's exit region, "system" keeps the OS zone, else an IANA name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub timezone: String,
    /// Stable local relay port so a running Claude keeps working across Hydra restarts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_exit: Option<crate::node::ExitInfo>,
    /// Id of the assigned environment config; `None` = direct connection, system timezone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    #[serde(default)]
    pub keep_running: bool,
}

/// The launch configuration that actually applies to an account.
#[derive(Clone, Debug, Default)]
pub struct Effective {
    pub node: Option<crate::node::NodeConfig>,
    pub timezone: String,
    pub args: String,
    pub env: BTreeMap<String, String>,
}

impl AccountConfig {
    /// The account's node with its password unsealed, including a migrated legacy entrance.
    pub fn node(&self) -> Option<crate::node::NodeConfig> {
        if let Some(n) = &self.node {
            let mut n = n.clone();
            n.password = crate::node::unseal(&n.password);
            return n.validated().ok();
        }
        (!self.proxy_endpoint.is_empty()).then(|| crate::node::NodeConfig::from_legacy(&self.proxy_endpoint)).flatten()
    }

    /// Stores a node, sealing its password; `None` removes it.
    pub fn set_node(&mut self, node: Option<crate::node::NodeConfig>) {
        self.proxy_endpoint.clear();
        self.node = node.map(|mut n| { n.password = crate::node::seal(&n.password); n });
    }
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub environments: BTreeMap<String, crate::environment::Environment>,
}

impl Settings {
    pub fn account(&self, id: &str) -> AccountConfig {
        self.accounts.get(id).cloned().unwrap_or_default()
    }

    pub fn account_mut(&mut self, id: &str) -> &mut AccountConfig {
        self.accounts.entry(id.to_string()).or_default()
    }

    /// Assigned environment if any (a dangling id means none); otherwise legacy per-account fields.
    pub fn effective(&self, id: &str) -> Effective {
        let acc = self.account(id);
        if let Some(env_id) = &acc.environment {
            return match self.environments.get(env_id) {
                Some(e) => Effective { node: e.node(), timezone: e.timezone.clone(), args: e.launch_args(), env: e.env.clone() },
                None => Effective::default(),
            };
        }
        Effective { node: acc.node(), timezone: acc.timezone.clone(), args: acc.args.replace(['\r', '\n'], " ").trim().to_string(), env: acc.env.clone() }
    }

    pub fn environment_name(&self, id: &str) -> Option<String> {
        self.account(id).environment.and_then(|e| self.environments.get(&e).map(|e| e.name.clone()))
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
    // Owner-only: the file may hold proxy credentials.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
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

    #[test]
    fn node_password_is_sealed_and_legacy_migrates() {
        let mut acc = AccountConfig { proxy_endpoint: "http://127.0.0.1:8080".into(), ..Default::default() };
        assert_eq!(acc.node().unwrap().port, 8080);
        let node = crate::node::NodeConfig { kind: "socks5".into(), host: "h".into(), port: 1, username: "u".into(), password: "secret".into(), ..Default::default() };
        acc.set_node(Some(node));
        assert!(acc.proxy_endpoint.is_empty());
        assert_ne!(acc.node.as_ref().unwrap().password, "secret");
        assert_eq!(acc.node().unwrap().password, "secret");
    }
}
