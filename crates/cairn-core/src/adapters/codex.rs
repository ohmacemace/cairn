use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use walkdir::WalkDir;

use crate::adapters::AgentAdapter;
use crate::config::{write_atomic, SaveConfig};
use crate::error::{Error, Result};
use crate::hash::sha256_hex;
use crate::model::{
    Agent, ImportOutcome, MachineInfo, SaveMode, SessionMeta, SessionRecord, BUNDLE_SCHEMA,
};
use crate::paths::{now_ts, project_identity, rewrite_cwd, CWD_PLACEHOLDER};

#[derive(Debug, Clone)]
pub struct CodexAdapter {
    pub home: PathBuf,
    pub bin: String,
    pub save: SaveConfig,
    pub machine: MachineInfo,
}

impl CodexAdapter {
    pub fn new(home: PathBuf, bin: String, save: SaveConfig, machine: MachineInfo) -> Self {
        Self {
            home,
            bin,
            save,
            machine,
        }
    }

    fn sessions_dir(&self) -> PathBuf {
        self.home.join("sessions")
    }
}

impl AgentAdapter for CodexAdapter {
    fn agent(&self) -> Agent {
        Agent::Codex
    }

    fn discover(&self) -> Result<Vec<SessionRecord>> {
        let sessions = self.sessions_dir();
        if !sessions.exists() || !has_rollouts(&sessions) {
            return Ok(Vec::new());
        }

        let mut client = AppServer::spawn(&self.home, &self.bin)?;
        client.initialize()?;
        let threads = client.list_threads()?;

        let mut records = Vec::new();
        for thread in threads {
            let Some(path) = thread.path.as_deref() else {
                continue;
            };
            let path = PathBuf::from(path);
            if !path.exists() {
                continue;
            }
            let raw = std::fs::read(&path)?;
            let canonical_raw = rewrite_cwd(&raw, &thread.cwd, CWD_PLACEHOLDER);
            let content = match self.save.mode {
                SaveMode::Index => Vec::new(),
                // Codex boilerplate (tool definitions, base instructions) is not
                // trimmed yet: it needs a live resume test to validate.
                SaveMode::Standard | SaveMode::Full => canonical_raw,
            };

            let (project_key, git_remote, git_branch) = project_identity(Path::new(&thread.cwd));
            let content_hash = sha256_hex(&content);
            let source_path = path
                .strip_prefix(self.sessions_dir())
                .ok()
                .map(|p| p.to_string_lossy().replace('\\', "/"));

            let meta = SessionMeta {
                schema: BUNDLE_SCHEMA,
                agent: Agent::Codex,
                family_id: thread.id.clone(),
                timeline_id: thread.id.clone(),
                native_id: thread.id.clone(),
                title: thread
                    .name
                    .or(thread.preview)
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| thread.id.clone()),
                created_at: thread.created_at,
                updated_at: thread.updated_at,
                cwd: thread.cwd,
                project_key,
                git_remote,
                git_branch,
                source: thread.source,
                model: thread.model,
                archived: thread.archived.unwrap_or(false),
                pinned: false,
                machine_id: self.machine.id.clone(),
                machine_name: self.machine.name.clone(),
                content_hash,
                size_bytes: content.len() as u64,
                source_path,
                parent_timeline: None,
            };

            records.push(SessionRecord {
                meta,
                content,
                sidecars: Vec::new(),
            });
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
        let content = rewrite_cwd(
            &rewrite_cwd(&record.content, CWD_PLACEHOLDER, &new_cwd),
            &record.meta.cwd,
            &new_cwd,
        );

        let target = if let Some(rel) = &record.meta.source_path {
            self.sessions_dir().join(rel)
        } else {
            let stamp = chrono::DateTime::from_timestamp(record.meta.created_at, 0)
                .unwrap_or_else(chrono::Utc::now);
            let date_dir = stamp.format("%Y/%m/%d").to_string();
            let file_stamp = stamp.format("%Y-%m-%dT%H-%M-%S").to_string();
            self.sessions_dir().join(date_dir).join(format!(
                "rollout-{}-{}.jsonl",
                file_stamp, record.meta.native_id
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

        // Let Codex rebuild its SQLite thread index from the rollout file.
        let output = Command::new(&self.bin)
            .args([
                "migrate-rollouts",
                "--apply",
                "--thread",
                &record.meta.native_id,
                "--json",
            ])
            .env("CODEX_HOME", &self.home)
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                if !stdout.trim().is_empty() {
                    note = Some(match note {
                        Some(prev) => format!("{prev}; index: {}", stdout.trim()),
                        None => format!("index: {}", stdout.trim()),
                    });
                }
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                note = Some(match note {
                    Some(prev) => format!("{prev}; index rebuild skipped: {}", stderr.trim()),
                    None => format!("index rebuild skipped: {}", stderr.trim()),
                });
            }
            Err(err) => {
                note = Some(match note {
                    Some(prev) => format!("{prev}; index rebuild skipped: {err}"),
                    None => format!("index rebuild skipped: {err}"),
                });
            }
        }

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
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexThread {
    id: String,
    #[serde(default)]
    path: Option<String>,
    cwd: String,
    #[serde(default)]
    preview: Option<String>,
    #[serde(default)]
    name: Option<String>,
    created_at: i64,
    updated_at: i64,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    archived: Option<bool>,
}

struct AppServer {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl AppServer {
    fn spawn(home: &Path, bin: &str) -> Result<Self> {
        let mut child = Command::new(bin)
            .arg("app-server")
            .env("CODEX_HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                Error::Adapter(format!(
                    "could not start `{bin} app-server`: {e}. Is Codex CLI installed?"
                ))
            })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Adapter("codex app-server stdin unavailable".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Adapter("codex app-server stdout unavailable".into()))?;

        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        })
    }

    fn initialize(&mut self) -> Result<()> {
        let response = self.request(
            "initialize",
            json!({
                "clientInfo": { "name": "cairn", "version": env!("CARGO_PKG_VERSION") }
            }),
        )?;
        let _ = response;
        self.notify("initialized", json!({}))
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let message = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        self.write_message(&message)
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        self.write_message(&message)?;

        loop {
            let mut line = String::new();
            let n = self
                .stdout
                .read_line(&mut line)
                .map_err(|e| Error::Adapter(format!("codex app-server read failed: {e}")))?;
            if n == 0 {
                return Err(Error::Adapter(
                    "codex app-server closed its output unexpectedly".into(),
                ));
            }
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = value.get("error") {
                return Err(Error::Adapter(format!(
                    "codex app-server `{method}` failed: {error}"
                )));
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    fn write_message(&mut self, value: &Value) -> Result<()> {
        let raw = serde_json::to_string(value)?;
        self.stdin
            .write_all(raw.as_bytes())
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .map_err(|e| Error::Adapter(format!("codex app-server write failed: {e}")))
    }

    fn list_threads(&mut self) -> Result<Vec<CodexThread>> {
        let source_kinds = [
            "cli",
            "vscode",
            "exec",
            "appServer",
            "subAgent",
            "subAgentReview",
            "subAgentCompact",
            "subAgentThreadSpawn",
            "subAgentOther",
            "unknown",
        ];
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;

        for _ in 0..40 {
            let mut params = json!({
                "limit": 200,
                "sourceKinds": source_kinds,
                "useStateDbOnly": false
            });
            if let Some(cursor) = &cursor {
                params["cursor"] = Value::String(cursor.clone());
            }
            let result = self.request("thread/list", params)?;
            let data = result
                .get("data")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for item in data {
                if let Ok(thread) = serde_json::from_value::<CodexThread>(item) {
                    out.push(thread);
                }
            }
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
            if cursor.is_none() {
                break;
            }
        }

        Ok(out)
    }
}

impl Drop for AppServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn has_rollouts(dir: &Path) -> bool {
    WalkDir::new(dir)
        .max_depth(6)
        .into_iter()
        .filter_map(|e| e.ok())
        .any(|e| {
            e.file_type().is_file()
                && e.path().extension().and_then(|s| s.to_str()) == Some("jsonl")
        })
}
