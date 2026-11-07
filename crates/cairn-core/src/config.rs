use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::model::{MachineInfo, SaveMode, SessionMeta};
use crate::paths::expand_tilde;

pub const CONFIG_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreConfig {
    pub path: String,
    #[serde(default)]
    pub remote: Option<String>,
    #[serde(default = "default_branch")]
    pub branch: String,
}

fn default_branch() -> String {
    "main".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentsConfig {
    #[serde(default = "default_true")]
    pub codex: bool,
    #[serde(default = "default_true")]
    pub claude: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveConfig {
    #[serde(default)]
    pub mode: SaveMode,
    #[serde(default)]
    pub attachments: bool,
    #[serde(default = "default_max_text")]
    pub max_text_block_bytes: usize,
}

fn default_max_text() -> usize {
    8192
}

impl Default for SaveConfig {
    fn default() -> Self {
        Self {
            mode: SaveMode::Standard,
            attachments: false,
            max_text_block_bytes: default_max_text(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WslSource {
    pub distro: String,
    /// Linux home path inside the distro, e.g. `/home/mace/.codex`.
    #[serde(default)]
    pub codex_home: Option<String>,
    /// Linux home path inside the distro, e.g. `/home/mace/.claude`.
    #[serde(default)]
    pub claude_home: Option<String>,
    /// Override the UNC root. Defaults to `\\wsl.localhost\<distro>`.
    /// Useful for tests and unusual WSL setups.
    #[serde(default)]
    pub unc_root: Option<String>,
}

impl WslSource {
    pub fn root(&self) -> PathBuf {
        match &self.unc_root {
            Some(root) => expand_tilde(root),
            None => PathBuf::from(format!(r"\\wsl.localhost\{}", self.distro)),
        }
    }

    pub fn codex_home(&self) -> Option<PathBuf> {
        self.codex_home
            .as_deref()
            .map(|path| join_linux_path(&self.root(), path))
    }

    pub fn claude_home(&self) -> Option<PathBuf> {
        self.claude_home
            .as_deref()
            .map(|path| join_linux_path(&self.root(), path))
    }
}

/// Join a Linux path onto a host path (typically a WSL UNC root).
pub fn join_linux_path(root: &Path, linux: &str) -> PathBuf {
    let mut out = root.to_path_buf();
    for part in linux.trim_matches('/').split('/') {
        if !part.is_empty() {
            out.push(part);
        }
    }
    out
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub schema: u32,
    pub machine_id: String,
    pub machine_name: String,
    pub store: StoreConfig,
    pub agents: AgentsConfig,
    pub save: SaveConfig,
    #[serde(default)]
    pub paths: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wsl: Vec<WslSource>,
    #[serde(default)]
    pub codex_home: Option<String>,
    #[serde(default)]
    pub claude_home: Option<String>,
}

impl Config {
    pub fn default_for(
        store_path: impl Into<String>,
        remote: Option<String>,
        machine_name: Option<String>,
    ) -> Self {
        let machine_name = machine_name.unwrap_or_else(detect_hostname);
        Self {
            schema: CONFIG_SCHEMA,
            machine_id: uuid::Uuid::new_v4().to_string(),
            machine_name,
            store: StoreConfig {
                path: store_path.into(),
                remote,
                branch: default_branch(),
            },
            agents: AgentsConfig {
                codex: true,
                claude: true,
            },
            save: SaveConfig::default(),
            paths: BTreeMap::new(),
            wsl: Vec::new(),
            codex_home: None,
            claude_home: None,
        }
    }

    pub fn config_path() -> PathBuf {
        if let Ok(path) = std::env::var("CAIRN_CONFIG") {
            return PathBuf::from(path);
        }
        let base = dirs::config_dir().unwrap_or_else(|| home_dir().join(".config"));
        base.join("cairn").join("config.toml")
    }

    pub fn state_path() -> PathBuf {
        if let Ok(path) = std::env::var("CAIRN_STATE") {
            return PathBuf::from(path);
        }
        let base = dirs::state_dir()
            .or_else(dirs::data_local_dir)
            .unwrap_or_else(|| home_dir().join(".local").join("state"));
        base.join("cairn").join("state.json")
    }

    pub fn load() -> Result<Self> {
        Self::load_from(&Self::config_path())
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| Error::Config(format!("cannot read {}: {e}", path.display())))?;
        let config: Config = toml::from_str(&raw)?;
        if config.schema != CONFIG_SCHEMA {
            return Err(Error::Config(format!(
                "unsupported config schema {} (expected {})",
                config.schema, CONFIG_SCHEMA
            )));
        }
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::config_path())
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = toml::to_string_pretty(self)?;
        write_atomic(path, raw.as_bytes())
    }

    pub fn store_path(&self) -> PathBuf {
        expand_tilde(&self.store.path)
    }

    pub fn machine(&self) -> MachineInfo {
        MachineInfo {
            id: self.machine_id.clone(),
            name: self.machine_name.clone(),
        }
    }

    pub fn codex_home(&self) -> PathBuf {
        if let Ok(v) = std::env::var("CAIRN_CODEX_HOME") {
            return expand_tilde(&v);
        }
        if let Ok(v) = std::env::var("CODEX_HOME") {
            return expand_tilde(&v);
        }
        if let Some(v) = &self.codex_home {
            return expand_tilde(v);
        }
        home_dir().join(".codex")
    }

    pub fn claude_home(&self) -> PathBuf {
        if let Ok(v) = std::env::var("CAIRN_CLAUDE_HOME") {
            return expand_tilde(&v);
        }
        if let Ok(v) = std::env::var("CLAUDE_CONFIG_DIR") {
            return expand_tilde(&v);
        }
        if let Some(v) = &self.claude_home {
            return expand_tilde(v);
        }
        home_dir().join(".claude")
    }

    /// Resolve the local directory where a restored session should live.
    pub fn resolve_cwd(&self, meta: &SessionMeta) -> PathBuf {
        if let Some(local) = self.paths.get(&meta.project_key) {
            return expand_tilde(local);
        }
        if let Some(remote) = &meta.git_remote {
            if let Some(local) = self.paths.get(remote) {
                return expand_tilde(local);
            }
            if let Some(local) = self.paths.get(&format!("git:{remote}")) {
                return expand_tilde(local);
            }
        }
        let original = PathBuf::from(&meta.cwd);
        if original.exists() {
            return original;
        }
        original
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineState {
    pub store_hash: String,
    pub local_hash: String,
    pub synced_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncState {
    pub schema: u32,
    #[serde(default)]
    pub timelines: BTreeMap<String, TimelineState>,
    /// Local-only mapping from a native session id to the timeline that owns
    /// it on this machine. Needed when two timelines share one native id.
    #[serde(default)]
    pub local_timelines: BTreeMap<String, String>,
}

impl Default for SyncState {
    fn default() -> Self {
        Self {
            schema: 1,
            timelines: BTreeMap::new(),
            local_timelines: BTreeMap::new(),
        }
    }
}

impl SyncState {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)?;
        let state: SyncState = serde_json::from_str(&raw)?;
        Ok(state)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_vec_pretty(self)?;
        write_atomic(path, &raw)
    }

    pub fn key(agent: impl std::fmt::Display, timeline_id: &str) -> String {
        format!("{agent}/{timeline_id}")
    }

    pub fn native_key(agent: impl std::fmt::Display, native_id: &str) -> String {
        format!("{agent}:{native_id}")
    }
}

pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

fn detect_hostname() -> String {
    if let Ok(v) = std::env::var("HOSTNAME") {
        if !v.trim().is_empty() {
            return v;
        }
    }
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown-machine".to_string())
}

pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::InvalidPath(path.to_path_buf(), "no parent directory".into()))?;
    std::fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("cairn"),
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&tmp, data)?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(err) => {
            // Windows does not replace an existing file with rename.
            if path.exists() {
                std::fs::remove_file(path)?;
                std::fs::rename(&tmp, path)?;
                Ok(())
            } else {
                let _ = std::fs::remove_file(&tmp);
                Err(Error::Io(err))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_wsl_linux_paths() {
        let source = WslSource {
            distro: "Ubuntu".into(),
            codex_home: Some("/home/mace/.codex".into()),
            claude_home: Some("/home/mace/.claude".into()),
            unc_root: Some("/tmp/fake-wsl".into()),
        };
        assert_eq!(
            source.codex_home().unwrap(),
            PathBuf::from("/tmp/fake-wsl/home/mace/.codex")
        );
        assert_eq!(
            source.claude_home().unwrap(),
            PathBuf::from("/tmp/fake-wsl/home/mace/.claude")
        );
    }

    #[test]
    fn defaults_wsl_root_to_unc() {
        let source = WslSource {
            distro: "Ubuntu".into(),
            codex_home: None,
            claude_home: None,
            unc_root: None,
        };
        assert!(source.root().to_string_lossy().contains("wsl.localhost"));
    }
}
