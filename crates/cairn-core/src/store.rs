use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::config::write_atomic;
use crate::error::{Error, Result};
use crate::model::{Agent, SessionMeta, SessionRecord, SidecarFile, BUNDLE_SCHEMA};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreManifest {
    pub schema: u32,
    pub created_at: i64,
    pub generator: String,
}

#[derive(Debug, Clone)]
pub struct Store {
    pub root: PathBuf,
}

impl Store {
    pub fn init(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root)?;
        let manifest_path = root.join("cairn.json");
        if !manifest_path.exists() {
            let manifest = StoreManifest {
                schema: 1,
                created_at: crate::paths::now_ts(),
                generator: format!("cairn {}", env!("CARGO_PKG_VERSION")),
            };
            write_atomic(
                &manifest_path,
                serde_json::to_vec_pretty(&manifest)?.as_slice(),
            )?;
        }
        Ok(Self {
            root: root.to_path_buf(),
        })
    }

    pub fn open(root: &Path) -> Result<Self> {
        let manifest_path = root.join("cairn.json");
        if !manifest_path.exists() {
            return Err(Error::Store(format!(
                "{} is not a cairn store (missing cairn.json)",
                root.display()
            )));
        }
        let raw = std::fs::read_to_string(&manifest_path)?;
        let manifest: StoreManifest = serde_json::from_str(&raw)?;
        if manifest.schema != 1 {
            return Err(Error::Store(format!(
                "unsupported store schema {}",
                manifest.schema
            )));
        }
        Ok(Self {
            root: root.to_path_buf(),
        })
    }

    pub fn bundle_dir(&self, agent: Agent, timeline_id: &str) -> PathBuf {
        self.root
            .join("threads")
            .join(agent.as_str())
            .join(sanitize_component(timeline_id))
    }

    pub fn has_bundle(&self, agent: Agent, timeline_id: &str) -> bool {
        self.bundle_dir(agent, timeline_id)
            .join("meta.json")
            .exists()
    }

    pub fn write_bundle(&self, record: &SessionRecord) -> Result<()> {
        if record.meta.schema != BUNDLE_SCHEMA {
            return Err(Error::Store(format!(
                "unsupported bundle schema {}",
                record.meta.schema
            )));
        }
        let dir = self.bundle_dir(record.meta.agent, &record.meta.timeline_id);
        std::fs::create_dir_all(&dir)?;

        let meta_raw = serde_json::to_vec_pretty(&record.meta)?;
        write_atomic(&dir.join("meta.json"), &meta_raw)?;

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&record.content)?;
        let compressed = encoder.finish()?;
        write_atomic(&dir.join("session.jsonl.gz"), &compressed)?;

        for sidecar in &record.sidecars {
            let rel = sanitize_relative(&sidecar.relative_path)?;
            let path = dir.join("sidecars").join(rel);
            write_atomic(&path, &sidecar.content)?;
        }

        Ok(())
    }

    pub fn read_bundle(&self, agent: Agent, timeline_id: &str) -> Result<SessionRecord> {
        let dir = self.bundle_dir(agent, timeline_id);
        let meta_path = dir.join("meta.json");
        if !meta_path.exists() {
            return Err(Error::NotFound(format!("bundle {}/{}", agent, timeline_id)));
        }
        let meta: SessionMeta = serde_json::from_slice(&std::fs::read(&meta_path)?)?;
        let compressed = std::fs::read(dir.join("session.jsonl.gz"))?;
        let mut decoder = GzDecoder::new(compressed.as_slice());
        let mut content = Vec::new();
        decoder.read_to_end(&mut content)?;

        let mut sidecars = Vec::new();
        let sidecar_root = dir.join("sidecars");
        if sidecar_root.exists() {
            for entry in WalkDir::new(&sidecar_root)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if !entry.file_type().is_file() {
                    continue;
                }
                let rel = entry
                    .path()
                    .strip_prefix(&sidecar_root)
                    .map_err(|_| Error::Store("invalid sidecar path".into()))?;
                sidecars.push(SidecarFile {
                    relative_path: rel.to_string_lossy().replace('\\', "/"),
                    content: std::fs::read(entry.path())?,
                });
            }
        }

        Ok(SessionRecord {
            meta,
            content,
            sidecars,
        })
    }

    pub fn list_bundles(&self) -> Result<Vec<SessionMeta>> {
        let threads = self.root.join("threads");
        if !threads.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in WalkDir::new(&threads)
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_name() != "meta.json" {
                continue;
            }
            let raw = std::fs::read(entry.path())?;
            let meta: SessionMeta = serde_json::from_slice(&raw)?;
            out.push(meta);
        }
        out.sort_by_key(|a| std::cmp::Reverse(a.updated_at));
        Ok(out)
    }

    pub fn find_by_native_id(&self, agent: Agent, native_id: &str) -> Result<Option<SessionMeta>> {
        Ok(self
            .list_bundles()?
            .into_iter()
            .find(|m| m.agent == agent && m.native_id == native_id))
    }

    pub fn find(&self, agent: Agent, id: &str) -> Result<Option<SessionMeta>> {
        let bundles = self.list_bundles()?;
        Ok(bundles.into_iter().find(|m| {
            m.agent == agent && (m.native_id == id || m.timeline_id == id || m.family_id == id)
        }))
    }
}

fn sanitize_component(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

fn sanitize_relative(value: &str) -> Result<PathBuf> {
    let path = PathBuf::from(value.replace('\\', "/"));
    if path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(Error::Store(format!("unsafe sidecar path: {value}")));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Agent, SessionMeta, SessionRecord, BUNDLE_SCHEMA};

    fn sample_meta() -> SessionMeta {
        SessionMeta {
            schema: BUNDLE_SCHEMA,
            agent: Agent::Claude,
            family_id: "fam".into(),
            timeline_id: "tl".into(),
            native_id: "native".into(),
            title: "hello".into(),
            created_at: 1,
            updated_at: 2,
            cwd: "/home/a/proj".into(),
            project_key: "local:proj-abc".into(),
            git_remote: None,
            git_branch: None,
            source: None,
            model: None,
            archived: false,
            pinned: false,
            machine_id: "m1".into(),
            machine_name: "machine".into(),
            content_hash: "hash".into(),
            size_bytes: 3,
            source_path: None,
            parent_timeline: None,
            merged_from: Vec::new(),
            fork_point: None,
            wsl_distro: None,
        }
    }

    #[test]
    fn round_trips_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::init(tmp.path()).unwrap();
        let record = SessionRecord::new(sample_meta(), b"abc".to_vec());
        store.write_bundle(&record).unwrap();
        let loaded = store.read_bundle(Agent::Claude, "tl").unwrap();
        assert_eq!(loaded.meta.native_id, "native");
        assert_eq!(loaded.content, b"abc");
        assert_eq!(store.list_bundles().unwrap().len(), 1);
    }
}
