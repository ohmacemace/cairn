use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use cairn_core::config::{Config, WslSource};
use cairn_core::git;
use cairn_core::model::{Agent, SaveMode, SessionMeta};
use cairn_core::paths::expand_tilde;
use cairn_core::store::Store;
use cairn_core::sync::SyncEngine;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentSettings {
    codex: bool,
    claude: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WslSourceView {
    distro: String,
    codex_home: Option<String>,
    claude_home: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfigView {
    machine_name: String,
    store_path: String,
    remote: Option<String>,
    save_mode: String,
    agents: AgentSettings,
    wsl: Vec<WslSourceView>,
    config_path: String,
    store_manifest: String,
}

impl ConfigView {
    fn from_config(config: &Config) -> Self {
        Self {
            machine_name: config.machine_name.clone(),
            store_path: config.store.path.clone(),
            remote: config.store.remote.clone(),
            save_mode: match config.save.mode {
                SaveMode::Index => "index".into(),
                SaveMode::Standard => "standard".into(),
                SaveMode::Full => "full".into(),
            },
            agents: AgentSettings {
                codex: config.agents.codex,
                claude: config.agents.claude,
            },
            wsl: config
                .wsl
                .iter()
                .map(|source| WslSourceView {
                    distro: source.distro.clone(),
                    codex_home: source.codex_home.clone(),
                    claude_home: source.claude_home.clone(),
                })
                .collect(),
            config_path: Config::config_path().display().to_string(),
            store_manifest: config.store_path().join("cairn.json").display().to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionView {
    id: String,
    timeline_id: String,
    family_id: String,
    parent_timeline: Option<String>,
    agent: String,
    title: String,
    project: String,
    cwd: String,
    updated_at: i64,
    size_bytes: u64,
    source: Option<String>,
    wsl_distro: Option<String>,
    machine_name: String,
    local: bool,
    remote: bool,
    cwd_exists: bool,
    mapped_cwd: Option<String>,
    archived: bool,
    merged_from: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanView {
    sessions: Vec<SessionView>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusView {
    local: usize,
    remote: usize,
    pending_push: Vec<String>,
    pending_pull: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PushView {
    added: usize,
    updated: usize,
    unchanged: usize,
    committed: bool,
    conflicts: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PullView {
    remote: usize,
    warnings: Vec<String>,
    notes: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RestoreView {
    imported: bool,
    native_id: String,
    path: String,
    note: Option<String>,
    resume_hint: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DoctorView {
    lines: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MergeMaterialView {
    agent: String,
    title: String,
    fork_line: u64,
    message_count_a: usize,
    message_count_b: usize,
    warnings: Vec<String>,
    material: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MergeBriefView {
    brief: String,
    command: String,
    material: MergeMaterialView,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MergeSessionView {
    native_id: String,
    timeline_id: String,
    title: String,
    path: String,
    note: Option<String>,
    resume_hint: String,
}

fn merge_material_view(material: cairn_core::merge::MergeMaterial) -> MergeMaterialView {
    MergeMaterialView {
        agent: material.agent.to_string(),
        title: material.title,
        fork_line: material.fork_line,
        message_count_a: material.message_count_a,
        message_count_b: material.message_count_b,
        warnings: material.warnings,
        material: material.markdown,
    }
}

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn load_config() -> Result<Config, String> {
    Config::load().map_err(err)
}

fn session_view(meta: &SessionMeta, local: bool, remote: bool, config: &Config) -> SessionView {
    let original = PathBuf::from(&meta.cwd);
    let resolved = config.resolve_cwd(meta);
    let cwd_exists = if meta.wsl_distro.is_some() {
        true
    } else {
        original.exists() || resolved.exists()
    };
    let mapped_cwd = if resolved != original {
        Some(resolved.to_string_lossy().to_string())
    } else {
        None
    };
    SessionView {
        id: meta.native_id.clone(),
        timeline_id: meta.timeline_id.clone(),
        family_id: meta.family_id.clone(),
        parent_timeline: meta.parent_timeline.clone(),
        agent: meta.agent.to_string(),
        title: meta.title.clone(),
        project: meta.project_key.clone(),
        cwd: meta.cwd.clone(),
        updated_at: meta.updated_at,
        size_bytes: meta.size_bytes,
        source: meta.source.clone(),
        wsl_distro: meta.wsl_distro.clone(),
        machine_name: meta.machine_name.clone(),
        local,
        remote,
        cwd_exists,
        mapped_cwd,
        archived: meta.archived,
        merged_from: meta.merged_from.clone(),
    }
}

fn resume_hint(meta: &SessionMeta) -> String {
    let id = &meta.native_id;
    match (&meta.agent, &meta.wsl_distro) {
        (Agent::Codex, Some(distro)) => format!("wsl.exe -d {distro} -- codex resume {id}"),
        (Agent::Claude, Some(distro)) => {
            format!("wsl.exe -d {distro} -- claude --resume {id}")
        }
        (Agent::Codex, None) => format!("codex resume {id}"),
        (Agent::Claude, None) => format!("claude --resume {id}"),
    }
}

fn fallback_hint(agent: Option<Agent>, id: &str) -> String {
    match agent {
        Some(Agent::Codex) => format!("codex resume {id}"),
        Some(Agent::Claude) => format!("claude --resume {id}"),
        None => format!("codex resume {id}  /  claude --resume {id}"),
    }
}

fn parse_save_mode(value: &str) -> Result<SaveMode, String> {
    match value {
        "index" => Ok(SaveMode::Index),
        "standard" => Ok(SaveMode::Standard),
        "full" => Ok(SaveMode::Full),
        other => Err(format!("unknown save mode: {other}")),
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn tool_version(name: &str, args: &[&str]) -> String {
    match Command::new(name).args(args).output() {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
        _ => "not found".to_string(),
    }
}

async fn blocking<T, F>(task: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|error| format!("task failed: {error}"))?
}

#[tauri::command]
async fn get_config() -> Result<Option<ConfigView>, String> {
    blocking(|| {
        if !Config::config_path().exists() {
            return Ok(None);
        }
        let config = load_config()?;
        Ok(Some(ConfigView::from_config(&config)))
    })
    .await
}

fn ensure_store(store: &Path, remote: Option<&str>) -> Result<(), String> {
    if !store.join(".git").exists() {
        let mut cloned = false;
        if let Some(url) = remote {
            if !store.exists() {
                let output = Command::new("git")
                    .arg("clone")
                    .arg(url)
                    .arg(store)
                    .output();
                cloned = matches!(output, Ok(output) if output.status.success());
            }
        }
        if !cloned {
            Store::init(store).map_err(err)?;
            git::init(store, remote, "main").map_err(err)?;
        }
    }
    Store::init(store).map_err(err)?;
    Ok(())
}

#[tauri::command]
async fn init_store(
    store_path: String,
    remote: Option<String>,
    machine_name: Option<String>,
) -> Result<ConfigView, String> {
    blocking(move || {
        let store = expand_tilde(&store_path);
        ensure_store(&store, remote.as_deref())?;
        let config = Config::default_for(
            store.to_string_lossy().to_string(),
            non_empty(remote),
            machine_name.and_then(|value| {
                let value = value.trim().to_string();
                if value.is_empty() {
                    None
                } else {
                    Some(value)
                }
            }),
        );
        config.save().map_err(err)?;
        Ok(ConfigView::from_config(&config))
    })
    .await
}

#[tauri::command]
async fn update_store(
    store_path: String,
    remote: Option<String>,
    machine_name: Option<String>,
) -> Result<ConfigView, String> {
    blocking(move || {
        let mut config = load_config()?;
        let store = expand_tilde(&store_path);
        ensure_store(&store, remote.as_deref())?;
        config.store.path = store.to_string_lossy().to_string();
        config.store.remote = non_empty(remote);
        if let Some(name) = machine_name {
            let name = name.trim();
            if !name.is_empty() {
                config.machine_name = name.to_string();
            }
        }
        config.save().map_err(err)?;
        Ok(ConfigView::from_config(&config))
    })
    .await
}

#[tauri::command]
async fn list_sessions() -> Result<ScanView, String> {
    blocking(|| {
        let config = load_config()?;
        let engine = SyncEngine::new(config).map_err(err)?;
        let (entries, warnings) = engine.list_all().map_err(err)?;
        Ok(ScanView {
            sessions: entries
                .iter()
                .map(|entry| session_view(&entry.meta, entry.local, entry.remote, &engine.config))
                .collect(),
            warnings,
        })
    })
    .await
}

#[tauri::command]
async fn sync_status() -> Result<StatusView, String> {
    blocking(|| {
        let config = load_config()?;
        let engine = SyncEngine::new(config).map_err(err)?;
        let status = engine.status().map_err(err)?;
        Ok(StatusView {
            local: status.local,
            remote: status.remote,
            pending_push: status.pending_push,
            pending_pull: status.pending_pull,
            warnings: status.warnings,
        })
    })
    .await
}

#[tauri::command]
async fn push_store() -> Result<PushView, String> {
    blocking(|| {
        let config = load_config()?;
        let engine = SyncEngine::new(config).map_err(err)?;
        let report = engine.push(false).map_err(err)?;
        Ok(PushView {
            added: report.added,
            updated: report.updated,
            unchanged: report.unchanged,
            committed: report.committed,
            conflicts: report.conflicts,
            warnings: report.warnings,
        })
    })
    .await
}

#[tauri::command]
async fn pull_store() -> Result<PullView, String> {
    blocking(|| {
        let config = load_config()?;
        let engine = SyncEngine::new(config).map_err(err)?;
        let report = engine.pull(false).map_err(err)?;
        Ok(PullView {
            remote: report.remote,
            warnings: report.warnings,
            notes: report.notes,
        })
    })
    .await
}

#[tauri::command]
async fn restore_session(
    id: String,
    timeline_id: Option<String>,
    agent: Option<String>,
    cwd: Option<String>,
    remember: bool,
) -> Result<RestoreView, String> {
    blocking(move || {
        let config = load_config()?;
        let mut engine = SyncEngine::new(config).map_err(err)?;
        let parsed_agent = agent
            .as_deref()
            .map(Agent::from_str)
            .transpose()
            .map_err(err)?;
        let timeline_id = non_empty(timeline_id);
        let cwd_path = non_empty(cwd).map(PathBuf::from);
        let outcome = if let (Some(agent), Some(timeline)) = (parsed_agent, timeline_id.as_deref())
        {
            engine.restore_timeline(agent, timeline, cwd_path.as_deref(), false)
        } else {
            engine.restore(&id, parsed_agent, cwd_path.as_deref(), false)
        }
        .map_err(err)?;

        let meta = if let (Some(agent), Some(timeline)) = (parsed_agent, timeline_id.as_deref()) {
            let requested = engine.store.find(agent, timeline).map_err(err)?;
            if requested.as_ref().map(|meta| meta.native_id.as_str())
                != Some(outcome.native_id.as_str())
            {
                engine.store.find(agent, &outcome.native_id).map_err(err)?
            } else {
                requested
            }
        } else {
            match parsed_agent {
                Some(agent) => engine.store.find(agent, &outcome.native_id).map_err(err)?,
                None => engine
                    .store
                    .find(Agent::Codex, &outcome.native_id)
                    .map_err(err)?
                    .or(engine
                        .store
                        .find(Agent::Claude, &outcome.native_id)
                        .map_err(err)?),
            }
        };
        let hint = meta
            .as_ref()
            .map(resume_hint)
            .unwrap_or_else(|| fallback_hint(parsed_agent, &outcome.native_id));

        if remember {
            if let Some(meta) = &meta {
                let target = cwd_path
                    .clone()
                    .unwrap_or_else(|| engine.config.resolve_cwd(meta));
                if target.to_string_lossy() != meta.cwd {
                    engine.config.paths.insert(
                        meta.project_key.clone(),
                        target.to_string_lossy().to_string(),
                    );
                    engine.config.save().map_err(err)?;
                }
            }
        }

        Ok(RestoreView {
            imported: outcome.imported,
            native_id: outcome.native_id,
            path: outcome.path.to_string_lossy().to_string(),
            note: outcome.note,
            resume_hint: hint,
        })
    })
    .await
}

#[tauri::command]
async fn save_settings(
    save_mode: String,
    agents: AgentSettings,
    wsl: Vec<WslSourceView>,
) -> Result<ConfigView, String> {
    blocking(move || {
        let mut config = load_config()?;
        config.save.mode = parse_save_mode(&save_mode)?;
        config.agents.codex = agents.codex;
        config.agents.claude = agents.claude;
        config.wsl = wsl
            .into_iter()
            .filter(|source| !source.distro.trim().is_empty())
            .map(|source| WslSource {
                distro: source.distro.trim().to_string(),
                codex_home: non_empty(source.codex_home),
                claude_home: non_empty(source.claude_home),
                unc_root: None,
            })
            .collect();
        config.save().map_err(err)?;
        Ok(ConfigView::from_config(&config))
    })
    .await
}

#[tauri::command]
async fn run_doctor() -> Result<DoctorView, String> {    blocking(|| {
        let mut lines = Vec::new();
        lines.push(format!("config: {}", Config::config_path().display()));
        match load_config() {
            Ok(config) => {
                lines.push(format!(
                    "machine: {} ({})",
                    config.machine_name, config.machine_id
                ));
                lines.push(format!("store: {}", config.store_path().display()));
                match Store::open(&config.store_path()) {
                    Ok(_) => lines.push("store manifest: ok".into()),
                    Err(error) => lines.push(format!("store manifest: {error}")),
                }
                lines.push(format!(
                    "codex home: {} (exists: {})",
                    config.codex_home().display(),
                    config.codex_home().exists()
                ));
                lines.push(format!(
                    "claude home: {} (exists: {})",
                    config.claude_home().display(),
                    config.claude_home().exists()
                ));
                if config.wsl.is_empty() {
                    lines.push("wsl sources: none".into());
                } else {
                    for source in &config.wsl {
                        lines.push(format!(
                            "wsl [{}] root: {}",
                            source.distro,
                            source.root().display()
                        ));
                        if let Some(path) = source.codex_home() {
                            lines.push(format!(
                                "  codex:  {} (exists: {})",
                                path.display(),
                                path.exists()
                            ));
                        }
                        if let Some(path) = source.claude_home() {
                            lines.push(format!(
                                "  claude: {} (exists: {})",
                                path.display(),
                                path.exists()
                            ));
                        }
                    }
                }
            }
            Err(error) => lines.push(format!("config error: {error}")),
        }
        lines.push(format!("git: {}", tool_version("git", &["--version"])));
        lines.push(format!("codex: {}", tool_version("codex", &["--version"])));
        lines.push(format!(
            "claude: {}",
            tool_version("claude", &["--version"])
        ));
        Ok(DoctorView { lines })
    })
    .await
}

#[tauri::command]
async fn merge_material(
    agent: String,
    timeline_a: String,
    timeline_b: String,
) -> Result<MergeMaterialView, String> {
    blocking(move || {
        let config = load_config()?;
        let engine = SyncEngine::new(config).map_err(err)?;
        let parsed = Agent::from_str(&agent).map_err(err)?;
        let material = cairn_core::merge::build_material(
            &engine.store,
            parsed,
            &timeline_a,
            &timeline_b,
        )
        .map_err(err)?;
        Ok(merge_material_view(material))
    })
    .await
}

#[tauri::command]
async fn generate_merge_brief(
    agent: String,
    timeline_a: String,
    timeline_b: String,
) -> Result<MergeBriefView, String> {
    blocking(move || {
        let config = load_config()?;
        let engine = SyncEngine::new(config).map_err(err)?;
        let parsed = Agent::from_str(&agent).map_err(err)?;
        let brief = cairn_core::merge::generate_brief(
            &engine,
            parsed,
            &timeline_a,
            &timeline_b,
        )
        .map_err(err)?;
        Ok(MergeBriefView {
            brief: brief.brief,
            command: brief.command,
            material: merge_material_view(brief.material),
        })
    })
    .await
}

#[tauri::command]
async fn create_merged_session(
    agent: String,
    timeline_a: String,
    timeline_b: String,
    brief: String,
    cwd: String,
) -> Result<MergeSessionView, String> {
    blocking(move || {
        let config = load_config()?;
        let engine = SyncEngine::new(config).map_err(err)?;
        let parsed = Agent::from_str(&agent).map_err(err)?;
        let target = expand_tilde(cwd.trim());
        if target.to_string_lossy().trim().is_empty() {
            return Err("请选择合并会话的工作目录".to_string());
        }
        let merged = cairn_core::merge::create_merged_session(
            &engine,
            parsed,
            &timeline_a,
            &timeline_b,
            &brief,
            &target,
        )
        .map_err(err)?;
        Ok(MergeSessionView {
            native_id: merged.meta.native_id.clone(),
            timeline_id: merged.meta.timeline_id.clone(),
            title: merged.meta.title.clone(),
            path: merged.path.to_string_lossy().to_string(),
            note: merged.note,
            resume_hint: resume_hint(&merged.meta),
        })
    })
    .await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            get_config,
            init_store,
            update_store,
            list_sessions,
            sync_status,
            push_store,
            pull_store,
            restore_session,
            save_settings,
            run_doctor,
            merge_material,
            generate_merge_brief,
            create_merged_session
        ])
        .run(tauri::generate_context!())
        .expect("error while running Cairn");
}
