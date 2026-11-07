use serde_json::Value;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::adapters::AgentAdapter;
use crate::config::write_atomic;
use crate::config::SaveConfig;
use crate::error::{Error, Result};
use crate::hash::sha256_hex;
use crate::model::{Agent, ImportOutcome, MachineInfo, SessionMeta, SessionRecord, BUNDLE_SCHEMA};
use crate::paths::{
    clean_title, file_mtime, normalize_git_remote, now_ts, parse_timestamp, project_key,
    restore_cwd, rewrite_cwd, split_line_ending, wsl_git_info, CWD_PLACEHOLDER,
};

use super::codex::canonicalize_history_mode;

/// File-based Codex adapter for sessions inside WSL.
///
/// It deliberately does not spawn `codex app-server` and never touches the
/// SQLite index: Windows cannot safely lock WSL SQLite databases. Rollout
/// files are read and written directly through the UNC path; Codex rebuilds
/// its index on the next start.
#[derive(Debug, Clone)]
pub struct CodexFileAdapter {
    pub home: PathBuf,
    pub save: SaveConfig,
    pub machine: MachineInfo,
    pub wsl_distro: String,
}

impl CodexFileAdapter {
    pub fn new(
        home: PathBuf,
        save: SaveConfig,
        machine: MachineInfo,
        wsl_distro: impl Into<String>,
    ) -> Self {
        Self {
            home,
            save,
            machine,
            wsl_distro: wsl_distro.into(),
        }
    }

    fn sessions_dir(&self) -> PathBuf {
        self.home.join("sessions")
    }

    fn parse_rollout(&self, path: &Path) -> Result<Option<SessionRecord>> {
        let raw = std::fs::read(path)?;
        if raw.is_empty() {
            return Ok(None);
        }

        let mut native_id: Option<String> = None;
        let mut cwd: Option<String> = None;
        let mut created_at: Option<i64> = None;
        let mut title: Option<String> = None;
        let mut model: Option<String> = None;
        let mut source: Option<String> = None;

        for chunk in raw.split_inclusive(|b| *b == b'\n') {
            let (line, _, _) = split_line_ending(chunk);
            if line.is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_slice::<Value>(line) else {
                continue;
            };
            match value.get("type").and_then(Value::as_str) {
                Some("session_meta") => {
                    let Some(payload) = value.get("payload") else {
                        continue;
                    };
                    if native_id.is_none() {
                        native_id = payload
                            .get("id")
                            .or_else(|| payload.get("session_id"))
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned);
                    }
                    if cwd.is_none() {
                        cwd = payload
                            .get("cwd")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned);
                    }
                    if created_at.is_none() {
                        created_at = payload
                            .get("timestamp")
                            .and_then(Value::as_str)
                            .and_then(parse_timestamp);
                    }
                    if model.is_none() {
                        model = payload
                            .get("model_provider")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned);
                    }
                    if source.is_none() {
                        source = payload
                            .get("source")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned);
                    }
                }
                Some("response_item") if title.is_none() => {
                    if let Some(text) = codex_user_text(&value) {
                        let cleaned = clean_title(&text);
                        if cleaned != "Untitled session" {
                            title = Some(cleaned);
                        }
                    }
                }
                _ => {}
            }
        }

        let Some(native_id) = native_id.or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .map(ToOwned::to_owned)
        }) else {
            return Ok(None);
        };
        let Some(cwd) = cwd else {
            return Ok(None);
        };

        let (project_key_value, git_remote, git_branch) = {
            let info = wsl_git_info(&self.wsl_distro, &cwd);
            let remote = info.remote.as_deref().and_then(normalize_git_remote);
            (
                project_key(Path::new(&cwd), remote.as_deref()),
                remote,
                info.branch,
            )
        };

        let content = rewrite_cwd(&canonicalize_history_mode(&raw), &cwd, CWD_PLACEHOLDER);
        let source_path = path
            .strip_prefix(self.sessions_dir())
            .ok()
            .map(|p| p.to_string_lossy().replace('\\', "/"));
        let created_at = created_at.unwrap_or_else(|| file_mtime(path));
        let updated_at = file_mtime(path).max(created_at);

        let meta = SessionMeta {
            schema: BUNDLE_SCHEMA,
            agent: Agent::Codex,
            family_id: native_id.clone(),
            timeline_id: native_id.clone(),
            native_id: native_id.clone(),
            title: title.unwrap_or_else(|| native_id.clone()),
            created_at,
            updated_at,
            cwd,
            project_key: project_key_value,
            git_remote,
            git_branch,
            source,
            model,
            archived: false,
            pinned: false,
            machine_id: self.machine.id.clone(),
            machine_name: self.machine.name.clone(),
            content_hash: sha256_hex(&content),
            size_bytes: content.len() as u64,
            source_path,
            parent_timeline: None,
            merged_from: Vec::new(),
            fork_point: None,
            wsl_distro: Some(self.wsl_distro.clone()),
        };

        Ok(Some(SessionRecord {
            meta,
            content,
            sidecars: Vec::new(),
        }))
    }
}

impl AgentAdapter for CodexFileAdapter {
    fn agent(&self) -> Agent {
        Agent::Codex
    }

    fn discover(&self) -> Result<Vec<SessionRecord>> {
        let sessions = self.sessions_dir();
        if !sessions.exists() {
            return Ok(Vec::new());
        }

        let mut records = Vec::new();
        for entry in WalkDir::new(&sessions)
            .max_depth(6)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file()
                || entry.path().extension().and_then(|s| s.to_str()) != Some("jsonl")
            {
                continue;
            }
            if let Some(record) = self.parse_rollout(entry.path())? {
                records.push(record);
            }
        }
        records.sort_by_key(|a| std::cmp::Reverse(a.meta.updated_at));
        Ok(records)
    }

    fn import(&self, record: &SessionRecord, target_cwd: &Path) -> Result<ImportOutcome> {
        if record.meta.agent != Agent::Codex {
            return Err(Error::Adapter("not a Codex session".into()));
        }
        if record.content.is_empty() {
            return Err(Error::Adapter(
                "this session was saved in index mode and cannot be restored".into(),
            ));
        }

        let new_cwd = target_cwd.to_string_lossy().to_string();
        let content = restore_cwd(&record.content, &record.meta.cwd, &new_cwd);
        let content = canonicalize_history_mode(&content);

        let target = if let Some(rel) = &record.meta.source_path {
            self.sessions_dir().join(rel)
        } else {
            let stamp = chrono::DateTime::from_timestamp(record.meta.created_at, 0)
                .unwrap_or_else(chrono::Utc::now);
            self.sessions_dir()
                .join(stamp.format("%Y/%m/%d").to_string())
                .join(format!(
                    "rollout-{}-{}.jsonl",
                    stamp.format("%Y-%m-%dT%H-%M-%S"),
                    record.meta.native_id
                ))
        };

        let mut note = None;
        if target.exists() {
            let existing = std::fs::read(&target)?;
            if existing == content {
                return Ok(ImportOutcome {
                    imported: false,
                    native_id: record.meta.native_id.clone(),
                    path: target,
                    note: Some("already up to date".into()),
                });
            }
            let backup = target.with_extension(format!("jsonl.cairn-backup-{}", now_ts()));
            std::fs::copy(&target, &backup)?;
            note = Some(format!(
                "backed up existing rollout to {}",
                backup.display()
            ));
        }

        write_atomic(&target, &content)?;
        note = Some(match note {
            Some(prev) => format!(
                "{prev}; WSL Codex index was not modified; Codex will rebuild it on next start"
            ),
            None => "WSL Codex index was not modified; Codex will rebuild it on next start".into(),
        });

        Ok(ImportOutcome {
            imported: true,
            native_id: record.meta.native_id.clone(),
            path: target,
            note,
        })
    }

    fn resume_command(&self, native_id: &str) -> Vec<String> {
        vec!["codex".into(), "resume".into(), native_id.to_string()]
    }

    fn wsl_distro(&self) -> Option<&str> {
        Some(&self.wsl_distro)
    }
}

fn codex_user_text(value: &Value) -> Option<String> {
    let payload = value.get("payload")?;
    if payload.get("type").and_then(Value::as_str) != Some("message") {
        return None;
    }
    if payload.get("role").and_then(Value::as_str) != Some("user") {
        return None;
    }
    let content = payload.get("content")?.as_array()?;
    let mut out = String::new();
    for item in content {
        if item.get("type").and_then(Value::as_str) == Some("input_text") {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                if text.trim_start().starts_with("<environment_context>") {
                    continue;
                }
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(text);
            }
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}
