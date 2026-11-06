use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::adapters::AgentAdapter;
use crate::config::{write_atomic, SaveConfig};
use crate::error::{Error, Result};
use crate::hash::sha256_hex;
use crate::model::{
    Agent, ImportOutcome, MachineInfo, SaveMode, SessionMeta, SessionRecord, SidecarFile,
    BUNDLE_SCHEMA,
};
use crate::paths::{
    claude_project_dir_for, clean_title, file_mtime, is_command_text, now_ts, parse_timestamp,
    project_identity, rewrite_cwd, CWD_PLACEHOLDER,
};

const DROP_TYPES: &[&str] = &[
    "api-request",
    "api-request-blob",
    "api-request-shape",
    "cost-state",
    "atis-latch",
    "queue-operation",
    "file-history-snapshot",
    "file-history-delta",
    "last-prompt",
    "ai-title",
    "mode",
    "permission-mode",
    "tools-used",
];

#[derive(Debug, Clone)]
pub struct ClaudeAdapter {
    pub home: PathBuf,
    pub save: SaveConfig,
    pub machine: MachineInfo,
}

impl ClaudeAdapter {
    pub fn new(home: PathBuf, save: SaveConfig, machine: MachineInfo) -> Self {
        Self {
            home,
            save,
            machine,
        }
    }

    fn projects_dir(&self) -> PathBuf {
        self.home.join("projects")
    }

    fn parse_record(&self, path: &Path) -> Result<Option<SessionRecord>> {
        let raw = std::fs::read(path)?;
        if raw.is_empty() {
            return Ok(None);
        }

        let mut session_id: Option<String> = None;
        let mut cwd: Option<String> = None;
        let mut git_branch: Option<String> = None;
        let mut created_at: Option<i64> = None;
        let mut updated_at: Option<i64> = None;
        let mut title: Option<String> = None;
        let mut model: Option<String> = None;
        let mut source: Option<String> = None;

        for line in raw.split(|b| *b == b'\n') {
            if line.is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_slice::<Value>(line) else {
                continue;
            };
            if session_id.is_none() {
                if let Some(id) = value.get("sessionId").and_then(Value::as_str) {
                    session_id = Some(id.to_string());
                }
            }
            if cwd.is_none() {
                if let Some(v) = value.get("cwd").and_then(Value::as_str) {
                    cwd = Some(v.to_string());
                }
            }
            if git_branch.is_none() {
                if let Some(v) = value.get("gitBranch").and_then(Value::as_str) {
                    git_branch = Some(v.to_string());
                }
            }
            if source.is_none() {
                if let Some(v) = value.get("entrypoint").and_then(Value::as_str) {
                    source = Some(v.to_string());
                }
            }
            if model.is_none() {
                if let Some(v) = value.get("requestedModel").and_then(Value::as_str) {
                    model = Some(v.to_string());
                } else if let Some(v) = value
                    .get("message")
                    .and_then(|m| m.get("model"))
                    .and_then(Value::as_str)
                {
                    model = Some(v.to_string());
                }
            }
            if let Some(ts) = value.get("timestamp").and_then(Value::as_str) {
                if let Some(secs) = parse_timestamp(ts) {
                    created_at = Some(created_at.map_or(secs, |c| c.min(secs)));
                    updated_at = Some(updated_at.map_or(secs, |u| u.max(secs)));
                }
            }
            if title.is_none() && value.get("type").and_then(Value::as_str) == Some("user") {
                if let Some(text) = extract_user_text(&value) {
                    if !text.trim().is_empty() {
                        let cleaned = clean_title(&text);
                        if cleaned != "Untitled session" {
                            title = Some(cleaned);
                        }
                    }
                }
            }
        }

        let Some(native_id) = session_id.or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        }) else {
            return Ok(None);
        };
        let Some(cwd) = cwd else {
            return Ok(None);
        };

        let (project_key, git_remote, detected_branch) = project_identity(Path::new(&cwd));
        let git_branch = git_branch.or(detected_branch);
        let created_at = created_at.unwrap_or_else(|| file_mtime(path));
        let updated_at = updated_at
            .unwrap_or_else(|| file_mtime(path))
            .max(file_mtime(path));
        let title = title.unwrap_or_else(|| native_id.clone());

        let canonical_raw = rewrite_cwd(&raw, &cwd, CWD_PLACEHOLDER);
        let content = match self.save.mode {
            SaveMode::Index => Vec::new(),
            SaveMode::Standard => standardize(
                &canonical_raw,
                self.save.max_text_block_bytes,
                self.save.attachments,
            ),
            SaveMode::Full => canonical_raw,
        };

        let sidecars = if self.save.mode == SaveMode::Full {
            self.collect_sidecars(path, &native_id)?
        } else {
            Vec::new()
        };

        let content_hash = sha256_hex(&content);

        let meta = SessionMeta {
            schema: BUNDLE_SCHEMA,
            agent: Agent::Claude,
            family_id: native_id.clone(),
            timeline_id: native_id.clone(),
            native_id,
            title,
            created_at,
            updated_at,
            cwd,
            project_key,
            git_remote,
            git_branch,
            source,
            model,
            archived: false,
            pinned: false,
            machine_id: self.machine.id.clone(),
            machine_name: self.machine.name.clone(),
            content_hash,
            size_bytes: content.len() as u64,
            source_path: None,
            parent_timeline: None,
        };

        Ok(Some(SessionRecord {
            meta,
            content,
            sidecars,
        }))
    }

    fn collect_sidecars(&self, session_path: &Path, native_id: &str) -> Result<Vec<SidecarFile>> {
        let dir = session_path
            .parent()
            .map(|p| p.join(native_id))
            .unwrap_or_default();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in WalkDir::new(&dir).into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(&dir)
                .map_err(|_| Error::Adapter("invalid sidecar path".into()))?;
            out.push(SidecarFile {
                relative_path: rel.to_string_lossy().replace('\\', "/"),
                content: std::fs::read(entry.path())?,
            });
        }
        Ok(out)
    }
}

impl AgentAdapter for ClaudeAdapter {
    fn agent(&self) -> Agent {
        Agent::Claude
    }

    fn discover(&self) -> Result<Vec<SessionRecord>> {
        let projects = self.projects_dir();
        if !projects.exists() {
            return Ok(Vec::new());
        }

        let mut by_id: BTreeMap<String, SessionRecord> = BTreeMap::new();
        for entry in WalkDir::new(&projects)
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            if entry.path().extension().and_then(|s| s.to_str()) != Some("jsonl") {
                continue;
            }
            if let Some(record) = self.parse_record(entry.path())? {
                let id = record.meta.native_id.clone();
                match by_id.get(&id) {
                    Some(existing) if existing.meta.updated_at >= record.meta.updated_at => {}
                    _ => {
                        by_id.insert(id, record);
                    }
                }
            }
        }

        let mut records: Vec<_> = by_id.into_values().collect();
        records.sort_by_key(|a| std::cmp::Reverse(a.meta.updated_at));
        Ok(records)
    }

    fn import(&self, record: &SessionRecord, target_cwd: &Path) -> Result<ImportOutcome> {
        if record.meta.agent != Agent::Claude {
            return Err(Error::Adapter("not a Claude session".into()));
        }
        if record.content.is_empty() {
            return Err(Error::Adapter(
                "this session was saved in index mode and cannot be restored".into(),
            ));
        }

        let old_cwd = &record.meta.cwd;
        let new_cwd = target_cwd.to_string_lossy().to_string();
        let content = rewrite_cwd(
            &rewrite_cwd(&record.content, CWD_PLACEHOLDER, &new_cwd),
            old_cwd,
            &new_cwd,
        );

        let project_dir = claude_project_dir_for(&self.home, target_cwd);
        std::fs::create_dir_all(&project_dir)?;
        let target = project_dir.join(format!("{}.jsonl", record.meta.native_id));

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
                "backed up existing session to {}",
                backup.display()
            ));
        }

        write_atomic(&target, &content)?;

        for sidecar in &record.sidecars {
            let rel = PathBuf::from(&sidecar.relative_path);
            if rel.is_absolute()
                || rel
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                continue;
            }
            let path = project_dir.join(&record.meta.native_id).join(rel);
            write_atomic(&path, &sidecar.content)?;
        }

        Ok(ImportOutcome {
            imported: true,
            native_id: record.meta.native_id.clone(),
            path: target,
            note,
        })
    }

    fn resume_command(&self, native_id: &str) -> Vec<String> {
        vec!["claude".into(), "--resume".into(), native_id.to_string()]
    }
}

fn extract_user_text(value: &Value) -> Option<String> {
    let message = value.get("message")?;
    if message.get("role").and_then(Value::as_str) != Some("user") {
        return None;
    }
    match message.get("content") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(items)) => {
            let mut out = String::new();
            for item in items {
                if item.get("type").and_then(Value::as_str) == Some("text") {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(text);
                    }
                }
            }
            if out.is_empty() || is_command_text(&out) {
                None
            } else {
                Some(out)
            }
        }
        _ => None,
    }
}

fn standardize(raw: &[u8], max_text: usize, keep_attachments: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    for chunk in raw.split_inclusive(|b| *b == b'\n') {
        let has_newline = chunk.last() == Some(&b'\n');
        let line = if has_newline {
            &chunk[..chunk.len() - 1]
        } else {
            chunk
        };
        if line.is_empty() {
            if has_newline {
                out.push(b'\n');
            }
            continue;
        }
        let Ok(mut value) = serde_json::from_slice::<Value>(line) else {
            out.extend_from_slice(line);
            if has_newline {
                out.push(b'\n');
            }
            continue;
        };
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if DROP_TYPES.contains(&kind.as_str()) {
            continue;
        }
        if kind == "attachment" && !keep_attachments {
            // The attachment line is a node in Claude's parentUuid chain.
            // Keep the node, but strip the payload.
            if let Value::Object(map) = &mut value {
                map.insert("attachment".to_string(), json!({ "type": "cairn_omitted" }));
                map.insert(
                    "rendered".to_string(),
                    Value::String("[attachment omitted by cairn]".into()),
                );
            }
        }
        if kind == "user" || kind == "assistant" {
            truncate_value(&mut value, max_text);
        }
        if serde_json::to_writer(&mut out, &value).is_err() {
            out.extend_from_slice(line);
        }
        if has_newline {
            out.push(b'\n');
        }
    }
    out
}

fn truncate_value(value: &mut Value, limit: usize) {
    match value {
        Value::String(s) => {
            if s.len() > limit {
                let head: String = s.chars().take(limit / 2).collect();
                let tail: String = s
                    .chars()
                    .rev()
                    .take(limit / 4)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                *s = format!(
                    "{head}\n[... cairn truncated {} chars ...]\n{tail}",
                    s.chars()
                        .count()
                        .saturating_sub(head.chars().count() + tail.chars().count())
                );
            }
        }
        Value::Array(items) => {
            for item in items {
                truncate_value(item, limit);
            }
        }
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                // Keep identity fields intact even if they ever become long.
                if matches!(
                    key.as_str(),
                    "sessionId" | "uuid" | "parentUuid" | "promptId"
                ) {
                    continue;
                }
                truncate_value(item, limit);
            }
        }
        _ => {}
    }
}
