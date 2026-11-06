use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::error::{Error, Result};

pub const BUNDLE_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    Codex,
    Claude,
}

impl Agent {
    pub fn as_str(self) -> &'static str {
        match self {
            Agent::Codex => "codex",
            Agent::Claude => "claude",
        }
    }
}

impl fmt::Display for Agent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Agent {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "codex" => Ok(Agent::Codex),
            "claude" | "claude-code" | "claudecode" => Ok(Agent::Claude),
            other => Err(Error::Config(format!("unknown agent: {other}"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SaveMode {
    /// Metadata and a small preview only. Not resumable.
    Index,
    /// Resumable transcript with large/internal payloads trimmed.
    #[default]
    Standard,
    /// Byte-for-byte session content.
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineInfo {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub schema: u32,
    pub agent: Agent,
    /// Logical session identity. All timelines of one session share this.
    pub family_id: String,
    /// Physical timeline identity (stored directory name).
    pub timeline_id: String,
    /// The native session id understood by Codex / Claude Code.
    pub native_id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    /// Absolute working directory on the machine that created the session.
    pub cwd: String,
    /// Canonical project identity, normally `git:host/owner/repo`.
    pub project_key: String,
    pub git_remote: Option<String>,
    pub git_branch: Option<String>,
    pub source: Option<String>,
    pub model: Option<String>,
    pub archived: bool,
    pub pinned: bool,
    pub machine_id: String,
    pub machine_name: String,
    pub content_hash: String,
    pub size_bytes: u64,
    /// For Codex: rollout path relative to `<codex_home>/sessions/`.
    pub source_path: Option<String>,
    /// Set when this timeline was forked from another timeline.
    pub parent_timeline: Option<String>,
    /// WSL distro when the session lives inside WSL rather than the native host.
    #[serde(default)]
    pub wsl_distro: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidecarFile {
    pub relative_path: String,
    #[serde(skip)]
    pub content: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct SessionRecord {
    pub meta: SessionMeta,
    pub content: Vec<u8>,
    pub sidecars: Vec<SidecarFile>,
}

impl SessionRecord {
    pub fn new(meta: SessionMeta, content: Vec<u8>) -> Self {
        Self {
            meta,
            content,
            sidecars: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ImportOutcome {
    pub imported: bool,
    pub native_id: String,
    pub path: std::path::PathBuf,
    pub note: Option<String>,
}
