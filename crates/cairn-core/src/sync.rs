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

#[derive(Debug, Clone)]
pub struct SessionListing {
    pub meta: SessionMeta,
    pub local: bool,
    pub remote: bool,
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
    /// Number of bundles visible in the store after refreshing metadata.
    pub remote: usize,
    /// Only set by `restore_all`, never by `pull`.
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

        // Apply the local timeline mapping before anything compares keys.
        // Without it, a forked session keeps looking like its parent.
        let state = match SyncState::load(&Config::state_path()) {
            Ok(state) => state,
            Err(err) => {
                report
                    .warnings
                    .push(format!("local sync state unavailable: {err}"));
                SyncState::default()
            }
        };
        for record in &mut report.records {
            let key = SyncState::native_key(record.meta.agent, &record.meta.native_id);
            if let Some(timeline_id) = state.local_timelines.get(&key) {
                if self.store.has_bundle(record.meta.agent, timeline_id) {
                    record.meta.timeline_id = timeline_id.clone();
                }
            }
        }

        report
            .records
            .sort_by_key(|a| std::cmp::Reverse(a.meta.updated_at));
        Ok(report)
    }

    /// Merge locally discovered sessions with store bundles.
    /// Local metadata wins when a timeline exists in both places.
    pub fn list_all(&self) -> Result<(Vec<SessionListing>, Vec<String>)> {
        let scan = self.scan()?;
        let warnings = scan.warnings;
        let store = self.store.list_bundles()?;

        let mut by_key: BTreeMap<(Agent, String), SessionListing> = BTreeMap::new();
        for record in scan.records {
            by_key.insert(
                (record.meta.agent, record.meta.timeline_id.clone()),
                SessionListing {
                    meta: record.meta,
                    local: true,
                    remote: false,
                },
            );
        }
        for meta in store {
            let key = (meta.agent, meta.timeline_id.clone());
            match by_key.get_mut(&key) {
                Some(entry) => {
                    entry.remote = true;
                    // Store metadata is authoritative for archive and merge data;
                    // local adapters cannot know about it.
                    entry.meta.archived |= meta.archived;
                    if entry.meta.parent_timeline.is_none() {
                        entry.meta.parent_timeline = meta.parent_timeline.clone();
                    }
                    if entry.meta.fork_point.is_none() {
                        entry.meta.fork_point = meta.fork_point.clone();
                    }
                    if entry.meta.merged_from.is_empty() {
                        entry.meta.merged_from = meta.merged_from.clone();
                    }
                    if !meta.merged_from.is_empty() {
                        // Merge titles are authored once; do not replace them
                        // with the first line of the generated brief.
                        entry.meta.title = meta.title.clone();
                    }
                }
                None => {
                    by_key.insert(
                        key,
                        SessionListing {
                            meta,
                            local: false,
                            remote: true,
                        },
                    );
                }
            }
        }

        let mut entries: Vec<_> = by_key.into_values().collect();
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.meta.updated_at));
        Ok((entries, warnings))
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

        for mut record in scan.records {
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
                    let (remote_changed, local_changed) = match known.as_ref() {
                        Some(k) => (
                            k.store_hash != remote_meta.content_hash,
                            k.local_hash != record.meta.content_hash,
                        ),
                        // Without a local baseline we cannot prove the remote
                        // copy is an ancestor of ours. Preserve both.
                        None => (true, true),
                    };

                    if remote_changed && !local_changed {
                        report.warnings.push(format!(
                            "{} {}: local copy is behind the store; restore before pushing",
                            record.meta.agent, record.meta.native_id
                        ));
                        report.unchanged += 1;
                        continue;
                    }

                    if remote_changed && local_changed {
                        let mut fork = record.clone();
                        let new_timeline = uuid::Uuid::new_v4().to_string();
                        preserve_store_meta(&mut fork, remote_meta);
                        fork.meta.timeline_id = new_timeline.clone();
                        fork.meta.parent_timeline = Some(remote_meta.timeline_id.clone());
                        fork.meta.family_id = remote_meta.family_id.clone();
                        if let Ok(remote_record) = self
                            .store
                            .read_bundle(record.meta.agent, &remote_meta.timeline_id)
                        {
                            let (line, byte, hash, message_id) = crate::merge::common_prefix(
                                &record.content,
                                &remote_record.content,
                                record.meta.agent,
                            );
                            fork.meta.fork_point = Some(crate::model::ForkPoint {
                                parent_timeline: remote_meta.timeline_id.clone(),
                                line,
                                byte,
                                hash,
                                message_id,
                                forked_at: now_ts(),
                                forked_by: self.config.machine_id.clone(),
                            });
                        }
                        if !dry_run {
                            self.store.write_bundle(&fork)?;
                        }
                        state.local_timelines.insert(
                            SyncState::native_key(record.meta.agent, &record.meta.native_id),
                            new_timeline.clone(),
                        );
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
                        preserve_store_meta(&mut record, remote_meta);
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

    /// Refresh store metadata only. Sessions are materialized one at a time
    /// with `restore` / `restore_timeline`.
    pub fn pull(&self, dry_run: bool) -> Result<PullReport> {
        if !dry_run {
            git::pull(&self.store.root, &self.config.store.branch)?;
        }
        let mut report = PullReport {
            remote: self.store.list_bundles()?.len(),
            ..Default::default()
        };
        report.notes.push(format!(
            "会话仓库已刷新：{} 条会话。恢复请按会话单独操作。",
            report.remote
        ));
        Ok(report)
    }

    /// Materialize every stored session. Kept for explicit maintenance use;
    /// the normal sync path never calls this.
    pub fn restore_all(&self, dry_run: bool) -> Result<PullReport> {
        git::pull(&self.store.root, &self.config.store.branch)?;

        let mut report = PullReport::default();
        let scan = self.scan()?;
        report.warnings.extend(scan.warnings);

        let mut local_by_native: BTreeMap<(Agent, String), SessionRecord> = scan
            .records
            .into_iter()
            .map(|record| ((record.meta.agent, record.meta.native_id.clone()), record))
            .collect();

        let state_path = Config::state_path();
        let mut state = SyncState::load(&state_path)?;
        let remote_bundles = self.store.list_bundles()?;
        report.remote = remote_bundles.len();

        for meta in &remote_bundles {
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

                // The local native session may be another timeline of the same
                // family (forks share the native id in the store). Never
                // overwrite it during a bulk pull.
                let local_is_sibling = remote_bundles.iter().any(|other| {
                    other.agent == meta.agent
                        && other.timeline_id != meta.timeline_id
                        && other.native_id == meta.native_id
                        && other.content_hash == local.meta.content_hash
                });
                if local_is_sibling {
                    report.conflicts.push(format!(
                        "{} {}: another timeline already uses this native id locally; skipped",
                        meta.agent, meta.native_id
                    ));
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
            local_by_native.insert(
                (record.meta.agent, record.meta.native_id.clone()),
                record.clone(),
            );
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
        let state = SyncState::load(&Config::state_path()).unwrap_or_default();

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
                    if let Some(known) = state.timelines.get(&key) {
                        let remote_changed = known.store_hash != remote.content_hash;
                        let local_changed = known.local_hash != local.meta.content_hash;
                        match (remote_changed, local_changed) {
                            (true, false) => report.pending_pull.push(key.clone()),
                            (false, true) => report.pending_push.push(key.clone()),
                            (true, true) => report.pending_push.push(key.clone()),
                            (false, false) => {}
                        }
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

        self.restore_timeline(meta.agent, &meta.timeline_id, cwd, dry_run)
    }

    pub fn restore_timeline(
        &self,
        agent: Agent,
        timeline_id: &str,
        cwd: Option<&Path>,
        dry_run: bool,
    ) -> Result<ImportOutcome> {
        let record = self.store.read_bundle(agent, timeline_id)?;
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

        if record.meta.wsl_distro.is_none() && !target_cwd.exists() {
            std::fs::create_dir_all(&target_cwd)?;
        }

        // A fork keeps the parent's native id in the store. It cannot be
        // materialized next to another timeline that already owns that native
        // id on this machine without overwriting it.
        if !record.content.is_empty() {
            let sibling_in_store = self.store.list_bundles()?.iter().any(|meta| {
                meta.agent == agent
                    && meta.native_id == record.meta.native_id
                    && meta.timeline_id != record.meta.timeline_id
            });
            if sibling_in_store {
                let scan = self.scan()?;
                if let Some(local) = scan.records.iter().find(|local| {
                    local.meta.agent == agent && local.meta.native_id == record.meta.native_id
                }) {
                    if local.meta.content_hash != record.meta.content_hash
                        && local.meta.timeline_id != record.meta.timeline_id
                    {
                        return Err(Error::Config(format!(
                            "原生会话 {} 已被本机另一条时间线占用。请先备份或移除本机会话，再恢复这条时间线。",
                            record.meta.native_id
                        )));
                    }
                }
            }
        }

        let adapter = self.adapter_for_meta(&record.meta)?;
        let outcome = adapter.import(&record, &target_cwd)?;

        // Record the restore so a later local append can be recognized as
        // divergence instead of overwriting the store copy. Forks also need a
        // local native-id mapping so scans keep them separate from the parent.
        let state_path = Config::state_path();
        let mut state = SyncState::load(&state_path)?;
        if record.meta.timeline_id != record.meta.native_id {
            state.local_timelines.insert(
                SyncState::native_key(agent, &record.meta.native_id),
                record.meta.timeline_id.clone(),
            );
        }
        state.timelines.insert(
            SyncState::key(agent, &record.meta.timeline_id),
            crate::config::TimelineState {
                store_hash: record.meta.content_hash.clone(),
                local_hash: record.meta.content_hash.clone(),
                synced_at: now_ts(),
            },
        );
        state.save(&state_path)?;

        Ok(outcome)
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

/// Local adapters do not know about store-only metadata. Preserve it when a
/// locally changed session is written back.
fn preserve_store_meta(record: &mut SessionRecord, remote: &SessionMeta) {
    if record.meta.parent_timeline.is_none() {
        record.meta.parent_timeline = remote.parent_timeline.clone();
    }
    if record.meta.fork_point.is_none() {
        record.meta.fork_point = remote.fork_point.clone();
    }
    if record.meta.merged_from.is_empty() {
        record.meta.merged_from = remote.merged_from.clone();
    }
    record.meta.archived |= remote.archived;
    if !remote.merged_from.is_empty() {
        record.meta.title = remote.title.clone();
    }
}
