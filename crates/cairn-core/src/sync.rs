use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::adapters::claude::ClaudeAdapter;
use crate::adapters::codex::CodexAdapter;
use crate::adapters::codex_file::CodexFileAdapter;
use crate::adapters::AgentAdapter;
use crate::config::{Config, SyncState};
use crate::error::{Error, Result};
use crate::git;
use crate::model::{Agent, ImportOutcome, MachineInfo, SessionMeta, SessionRecord};
use crate::paths::now_ts;
use crate::store::Store;

#[derive(Debug, Default)]
pub struct ScanReport {
    pub records: Vec<SessionRecord>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Default)]
pub struct PushReport {
    pub added: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub conflicts: Vec<String>,
    pub warnings: Vec<String>,
    pub committed: bool,
}

#[derive(Debug, Default)]
pub struct PullReport {
    pub imported: usize,
    pub skipped: usize,
    pub conflicts: Vec<String>,
    pub warnings: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Default)]
pub struct StatusReport {
    pub local: usize,
    pub remote: usize,
    pub pending_push: Vec<String>,
    pub pending_pull: Vec<String>,
    pub conflicts: Vec<String>,
    pub warnings: Vec<String>,
}

pub struct SyncEngine {
    pub config: Config,
    pub store: Store,
}

impl SyncEngine {
    pub fn new(config: Config) -> Result<Self> {
        let store = Store::open(&config.store_path())?;
        Ok(Self { config, store })
    }

    pub fn adapters(&self) -> Vec<Box<dyn AgentAdapter>> {
        let machine = self.config.machine();
        let mut out: Vec<Box<dyn AgentAdapter>> = Vec::new();

        if self.config.agents.codex {
            let bin = std::env::var("CAIRN_CODEX_BIN").unwrap_or_else(|_| "codex".to_string());
            out.push(Box::new(CodexAdapter::new(
                self.config.codex_home(),
                bin,
                self.config.save.clone(),
                machine.clone(),
            )));
        }
        if self.config.agents.claude {
            out.push(Box::new(ClaudeAdapter::new(
                self.config.claude_home(),
                self.config.save.clone(),
                machine.clone(),
            )));
        }

        for source in &self.config.wsl {
            let wsl_machine = MachineInfo {
                id: self.config.machine_id.clone(),
                name: format!("{} (WSL: {})", self.config.machine_name, source.distro),
            };
            if self.config.agents.codex {
                if let Some(home) = source.codex_home() {
                    out.push(Box::new(CodexFileAdapter::new(
                        home,
                        self.config.save.clone(),
                        wsl_machine.clone(),
                        source.distro.clone(),
                    )));
                }
            }
            if self.config.agents.claude {
                if let Some(home) = source.claude_home() {
                    out.push(Box::new(ClaudeAdapter::new_wsl(
                        home,
                        self.config.save.clone(),
                        wsl_machine,
                        source.distro.clone(),
                    )));
                }
            }
        }

        out
    }

    pub fn scan(&self) -> Result<ScanReport> {
        let mut report = ScanReport::default();
        for adapter in self.adapters() {
            match adapter.discover() {
                Ok(records) => report.records.extend(records),
                Err(err) => report
                    .warnings
                    .push(format!("{} adapter unavailable: {err}", adapter.agent())),
            }
        }
        report
            .records
            .sort_by_key(|a| std::cmp::Reverse(a.meta.updated_at));
        Ok(report)
    }

    pub fn push(&self, dry_run: bool) -> Result<PushReport> {
        git::pull(&self.store.root, &self.config.store.branch)?;

        let mut report = PushReport::default();
        let scan = self.scan()?;
        report.warnings.extend(scan.warnings);

        let existing = self.store.list_bundles()?;
        let existing_by_key: BTreeMap<String, SessionMeta> = existing
            .into_iter()
            .map(|meta| (SyncState::key(meta.agent, &meta.timeline_id), meta))
            .collect();

        let state_path = Config::state_path();
        let mut state = SyncState::load(&state_path)?;

        for record in scan.records {
            let key = SyncState::key(record.meta.agent, &record.meta.timeline_id);
            let known = state.timelines.get(&key).cloned();

            match existing_by_key.get(&key) {
                None => {
                    if !dry_run {
                        self.store.write_bundle(&record)?;
                    }
                    report.added += 1;
                    state.timelines.insert(
                        key,
                        crate::config::TimelineState {
                            store_hash: record.meta.content_hash.clone(),
                            local_hash: record.meta.content_hash.clone(),
                            synced_at: now_ts(),
                        },
                    );
                }
                Some(remote_meta) if remote_meta.content_hash == record.meta.content_hash => {
                    report.unchanged += 1;
                    state.timelines.insert(
                        key,
                        crate::config::TimelineState {
                            store_hash: record.meta.content_hash.clone(),
                            local_hash: record.meta.content_hash.clone(),
                            synced_at: now_ts(),
                        },
                    );
                }
                Some(remote_meta) => {
                    let both_changed = known
                        .as_ref()
                        .map(|k| {
                            k.store_hash != remote_meta.content_hash
                                && k.local_hash != record.meta.content_hash
                        })
                        .unwrap_or(false);

                    if both_changed {
                        let mut fork = record.clone();
                        let new_timeline = uuid::Uuid::new_v4().to_string();
                        fork.meta.timeline_id = new_timeline.clone();
                        fork.meta.parent_timeline = Some(remote_meta.timeline_id.clone());
                        fork.meta.family_id = remote_meta.family_id.clone();
                        if !dry_run {
                            self.store.write_bundle(&fork)?;
                        }
                        report.conflicts.push(format!(
                            "{} {} diverged; saved local timeline {}",
                            record.meta.agent, record.meta.native_id, new_timeline
                        ));
                        state.timelines.insert(
                            SyncState::key(record.meta.agent, &new_timeline),
                            crate::config::TimelineState {
                                store_hash: fork.meta.content_hash.clone(),
                                local_hash: fork.meta.content_hash.clone(),
                                synced_at: now_ts(),
                            },
                        );
                    } else {
                        if !dry_run {
                            self.store.write_bundle(&record)?;
                        }
                        report.updated += 1;
                        state.timelines.insert(
                            key,
                            crate::config::TimelineState {
                                store_hash: record.meta.content_hash.clone(),
                                local_hash: record.meta.content_hash.clone(),
                                synced_at: now_ts(),
                            },
                        );
                    }
                }
            }
        }

        if !dry_run {
            let message = format!(
                "cairn: push from {} at {}",
                self.config.machine_name,
                chrono::Utc::now().to_rfc3339()
            );
            report.committed = git::commit_all(&self.store.root, &message)?;
            git::push(&self.store.root, &self.config.store.branch)?;
            state.save(&state_path)?;
        }

        Ok(report)
    }

    pub fn pull(&self, dry_run: bool) -> Result<PullReport> {
        git::pull(&self.store.root, &self.config.store.branch)?;

        let mut report = PullReport::default();
        let scan = self.scan()?;
        report.warnings.extend(scan.warnings);

        let local_by_native: BTreeMap<(Agent, String), SessionRecord> = scan
            .records
            .into_iter()
            .map(|record| ((record.meta.agent, record.meta.native_id.clone()), record))
            .collect();

        let state_path = Config::state_path();
        let mut state = SyncState::load(&state_path)?;

        for meta in self.store.list_bundles()? {
            let key = SyncState::key(meta.agent, &meta.timeline_id);
            if let Some(local) = local_by_native.get(&(meta.agent, meta.native_id.clone())) {
                if local.meta.content_hash == meta.content_hash {
                    report.skipped += 1;
                    state.timelines.insert(
                        key,
                        crate::config::TimelineState {
                            store_hash: meta.content_hash.clone(),
                            local_hash: local.meta.content_hash.clone(),
                            synced_at: now_ts(),
                        },
                    );
                    continue;
                }

                let both_changed = state
                    .timelines
                    .get(&key)
                    .map(|k| {
                        k.store_hash != meta.content_hash && k.local_hash != local.meta.content_hash
                    })
                    .unwrap_or(false);
                if both_changed {
                    report.conflicts.push(format!(
                        "{} {} diverged; local copy kept. Run `cairn push` to preserve it as a timeline.",
                        meta.agent, meta.native_id
                    ));
                    continue;
                }
                if local.meta.updated_at > meta.updated_at {
                    report.skipped += 1;
                    continue;
                }
            }

            let record = self.store.read_bundle(meta.agent, &meta.timeline_id)?;
            let target_cwd = self.config.resolve_cwd(&record.meta);
            let is_wsl = record.meta.wsl_distro.is_some();
            if !is_wsl && !target_cwd.exists() {
                if dry_run {
                    report.notes.push(format!(
                        "would restore {} {} into {} (directory will be created)",
                        record.meta.agent,
                        record.meta.native_id,
                        target_cwd.display()
                    ));
                    report.imported += 1;
                    continue;
                }
                std::fs::create_dir_all(&target_cwd)?;
            }

            let adapter = match self.adapter_for_meta(&record.meta) {
                Ok(adapter) => adapter,
                Err(err) if is_wsl => {
                    report.warnings.push(format!(
                        "skipped WSL session {} ({}): {err}",
                        record.meta.native_id, record.meta.agent
                    ));
                    continue;
                }
                Err(err) => return Err(err),
            };
            if dry_run {
                report.notes.push(format!(
                    "would restore {} {} into {}",
                    record.meta.agent,
                    record.meta.native_id,
                    target_cwd.display()
                ));
                report.imported += 1;
                continue;
            }

            let outcome = adapter.import(&record, &target_cwd)?;
            report.imported += usize::from(outcome.imported);
            if let Some(note) = outcome.note {
                report.notes.push(format!(
                    "{} {}: {}",
                    record.meta.agent, record.meta.native_id, note
                ));
            }
            state.timelines.insert(
                key,
                crate::config::TimelineState {
                    store_hash: record.meta.content_hash.clone(),
                    local_hash: record.meta.content_hash.clone(),
                    synced_at: now_ts(),
                },
            );
        }

        if !dry_run {
            state.save(&state_path)?;
        }
        Ok(report)
    }

    pub fn status(&self) -> Result<StatusReport> {
        let mut report = StatusReport::default();
        let scan = self.scan()?;
        report.warnings.extend(scan.warnings);
        report.local = scan.records.len();

        let remote = self.store.list_bundles()?;
        report.remote = remote.len();

        let local_by_key: BTreeMap<String, &SessionRecord> = scan
            .records
            .iter()
            .map(|r| (SyncState::key(r.meta.agent, &r.meta.timeline_id), r))
            .collect();
        let remote_by_key: BTreeMap<String, &SessionMeta> = remote
            .iter()
            .map(|m| (SyncState::key(m.agent, &m.timeline_id), m))
            .collect();

        let mut keys: BTreeSet<String> = BTreeSet::new();
        keys.extend(local_by_key.keys().cloned());
        keys.extend(remote_by_key.keys().cloned());

        for key in keys {
            match (local_by_key.get(&key), remote_by_key.get(&key)) {
                (Some(local), Some(remote)) => {
                    if local.meta.content_hash == remote.content_hash {
                        continue;
                    }
                    if local.meta.updated_at > remote.updated_at {
                        report.pending_push.push(key);
                    } else {
                        report.pending_pull.push(key);
                    }
                }
                (Some(_), None) => report.pending_push.push(key),
                (None, Some(_)) => report.pending_pull.push(key),
                (None, None) => {}
            }
        }

        Ok(report)
    }

    pub fn restore(
        &self,
        id: &str,
        agent: Option<Agent>,
        cwd: Option<&Path>,
        dry_run: bool,
    ) -> Result<ImportOutcome> {
        let meta = match agent {
            Some(agent) => self.store.find(agent, id)?,
            None => self
                .store
                .find(Agent::Codex, id)?
                .or(self.store.find(Agent::Claude, id)?),
        }
        .ok_or_else(|| Error::NotFound(format!("session {id}")))?;

        let record = self.store.read_bundle(meta.agent, &meta.timeline_id)?;
        let target_cwd = match cwd {
            Some(path) => path.to_path_buf(),
            None => self.config.resolve_cwd(&record.meta),
        };

        if dry_run {
            return Ok(ImportOutcome {
                imported: false,
                native_id: record.meta.native_id.clone(),
                path: target_cwd,
                note: Some("dry run".into()),
            });
        }

        if !record.meta.wsl_distro.is_some() && !target_cwd.exists() {
            std::fs::create_dir_all(&target_cwd)?;
        }

        let adapter = self.adapter_for_meta(&record.meta)?;
        adapter.import(&record, &target_cwd)
    }

    pub fn adapter_for(&self, agent: Agent) -> Result<Box<dyn AgentAdapter>> {
        self.adapters()
            .into_iter()
            .find(|a| a.agent() == agent && a.wsl_distro().is_none())
            .ok_or_else(|| Error::Config(format!("{agent} adapter is disabled")))
    }

    pub fn adapter_for_meta(&self, meta: &SessionMeta) -> Result<Box<dyn AgentAdapter>> {
        let wanted = meta.wsl_distro.as_deref();
        self.adapters()
            .into_iter()
            .find(|a| a.agent() == meta.agent && a.wsl_distro() == wanted)
            .ok_or_else(|| match wanted {
                Some(distro) => Error::Config(format!(
                    "no {} adapter configured for WSL distro {distro}",
                    meta.agent
                )),
                None => Error::Config(format!("{} adapter is disabled", meta.agent)),
            })
    }

    pub fn resume_command(&self, agent: Agent, native_id: &str) -> Result<Vec<String>> {
        Ok(self.adapter_for(agent)?.resume_command(native_id))
    }
}
