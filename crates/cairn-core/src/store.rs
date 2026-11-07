use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::config::write_atomic;
use crate::error::{Error, Result};
use crate::hash::sha256_hex;
use crate::model::{Agent, SessionMeta, SessionRecord, SidecarFile, BUNDLE_SCHEMA};

const CHUNK_MAX_LINES: u64 = 512;
const CHUNK_MAX_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkRef {
    /// `sha256:<hex>` of the uncompressed chunk bytes.
    pub id: String,
    /// 1-based inclusive line range.
    pub start_line: u64,
    pub end_line: u64,
    /// 0-based byte offsets, `end_byte` exclusive.
    pub start_byte: u64,
    pub end_byte: u64,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkIndex {
    pub schema: u32,
    pub last_line: u64,
    pub last_byte: u64,
    #[serde(default)]
    pub last_ordinal: Option<u64>,
    #[serde(default)]
    pub last_message_id: Option<String>,
    pub head_hash: String,
    pub chunks: Vec<ChunkRef>,
}

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

        self.write_chunks(&dir, &record.content)?;

        let meta_raw = serde_json::to_vec_pretty(&record.meta)?;
        write_atomic(&dir.join("meta.json"), &meta_raw)?;

        for sidecar in &record.sidecars {
            let rel = sanitize_relative(&sidecar.relative_path)?;
            let path = dir.join("sidecars").join(rel);
            write_atomic(&path, &sidecar.content)?;
        }

        Ok(())
    }

    fn write_chunks(&self, dir: &Path, content: &[u8]) -> Result<()> {
        let index_path = dir.join("chunks.json");
        let chunks_dir = dir.join("chunks");
        let full_path = dir.join("session.jsonl.gz");

        if content.is_empty() {
            let _ = std::fs::remove_file(&index_path);
            let _ = std::fs::remove_file(&full_path);
            let _ = std::fs::remove_dir_all(&chunks_dir);
            return Ok(());
        }

        std::fs::create_dir_all(&chunks_dir)?;
        let existing = if index_path.exists() {
            serde_json::from_slice::<ChunkIndex>(&std::fs::read(&index_path)?).ok()
        } else {
            None
        };

        let append = existing.as_ref().is_some_and(|index| {
            let last = index.last_byte as usize;
            last <= content.len()
                && format!("sha256:{}", sha256_hex(&content[..last])) == index.head_hash
        });

        let (mut index, start_byte, start_line, rebuild) = match (append, existing) {
            (true, Some(index)) => {
                let start_byte = index.last_byte as usize;
                let start_line = index.last_line;
                (index, start_byte, start_line, false)
            }
            _ => (
                ChunkIndex {
                    schema: 1,
                    last_line: 0,
                    last_byte: 0,
                    last_ordinal: None,
                    last_message_id: None,
                    head_hash: String::new(),
                    chunks: Vec::new(),
                },
                0,
                0,
                true,
            ),
        };

        let new_chunks = build_chunks(&content[start_byte..], start_line, start_byte);
        for chunk in &new_chunks {
            let file = chunks_dir.join(chunk_file_name(&chunk.id));
            if !file.exists() {
                let raw = chunk_bytes(content, chunk)?;
                let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
                encoder.write_all(raw)?;
                let compressed = encoder.finish()?;
                write_atomic(&file, &compressed)?;
            }
        }
        index.chunks.extend(new_chunks);
        index.last_line += line_count(&content[start_byte..]);
        index.last_byte = content.len() as u64;
        index.head_hash = format!("sha256:{}", sha256_hex(content));
        let (ordinal, message_id) = tail_info(content);
        if ordinal.is_some() {
            index.last_ordinal = ordinal;
        }
        if message_id.is_some() {
            index.last_message_id = message_id;
        }

        write_atomic(&index_path, &serde_json::to_vec_pretty(&index)?)?;
        if rebuild {
            // Chunk files are content-addressed, so pruning only happens after
            // the new index is durable.
            let keep: std::collections::BTreeSet<String> = index
                .chunks
                .iter()
                .map(|chunk| chunk_file_name(&chunk.id))
                .collect();
            for entry in std::fs::read_dir(&chunks_dir)?.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if entry.file_type().map(|t| t.is_file()).unwrap_or(false) && !keep.contains(&name)
                {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        // The full snapshot is only a compatibility fallback for old stores.
        let _ = std::fs::remove_file(&full_path);
        Ok(())
    }

    pub fn read_bundle(&self, agent: Agent, timeline_id: &str) -> Result<SessionRecord> {
        let dir = self.bundle_dir(agent, timeline_id);
        let meta_path = dir.join("meta.json");
        if !meta_path.exists() {
            return Err(Error::NotFound(format!("bundle {}/{}", agent, timeline_id)));
        }
        let meta: SessionMeta = serde_json::from_slice(&std::fs::read(&meta_path)?)?;

        let content = if dir.join("chunks.json").exists() {
            read_chunks(&dir, &meta)?
        } else if dir.join("session.jsonl.gz").exists() {
            // Bundles written before chunk storage.
            let compressed = std::fs::read(dir.join("session.jsonl.gz"))?;
            let mut decoder = GzDecoder::new(compressed.as_slice());
            let mut content = Vec::new();
            decoder.read_to_end(&mut content)?;
            content
        } else {
            Vec::new()
        };

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

fn build_chunks(bytes: &[u8], start_line: u64, base_byte: usize) -> Vec<ChunkRef> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut offset = 0usize;
    let mut chunk_start = 0usize;
    let mut chunk_start_line = start_line + 1;
    let mut lines = 0u64;

    for chunk in bytes.split_inclusive(|b| *b == b'\n') {
        offset += chunk.len();
        lines += 1;
        let chunk_len = offset - chunk_start;
        if lines >= CHUNK_MAX_LINES || chunk_len >= CHUNK_MAX_BYTES || offset == bytes.len() {
            let raw = &bytes[chunk_start..offset];
            let hash = format!("sha256:{}", sha256_hex(raw));
            out.push(ChunkRef {
                id: hash.clone(),
                start_line: chunk_start_line,
                end_line: chunk_start_line + lines - 1,
                start_byte: (base_byte + chunk_start) as u64,
                end_byte: (base_byte + offset) as u64,
                hash,
            });
            chunk_start = offset;
            chunk_start_line += lines;
            lines = 0;
        }
    }
    out
}

fn chunk_file_name(id: &str) -> String {
    format!("{}.jsonl.gz", id.strip_prefix("sha256:").unwrap_or(id))
}

fn chunk_bytes<'a>(content: &'a [u8], chunk: &ChunkRef) -> Result<&'a [u8]> {
    let start = chunk.start_byte as usize;
    let end = chunk.end_byte as usize;
    content
        .get(start..end)
        .ok_or_else(|| Error::Store(format!("chunk {} is outside the session", chunk.id)))
}

fn line_count(bytes: &[u8]) -> u64 {
    if bytes.is_empty() {
        return 0;
    }
    let newlines = bytes.iter().filter(|byte| **byte == b'\n').count() as u64;
    if bytes.last() == Some(&b'\n') {
        newlines
    } else {
        newlines + 1
    }
}

fn tail_info(content: &[u8]) -> (Option<u64>, Option<String>) {
    let mut ordinal = None;
    let mut message_id = None;
    for chunk in content.rsplit(|byte| *byte == b'\n') {
        if chunk.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<Value>(chunk) else {
            continue;
        };
        if ordinal.is_none() {
            ordinal = value.get("ordinal").and_then(Value::as_u64);
        }
        if message_id.is_none() {
            match value.get("type").and_then(Value::as_str) {
                Some("user") | Some("assistant") => {
                    message_id = value
                        .get("uuid")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                }
                _ => {
                    let payload = value.get("payload").unwrap_or(&Value::Null);
                    if payload.get("type").and_then(Value::as_str) == Some("message") {
                        message_id = payload
                            .get("id")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                    }
                }
            }
        }
        if ordinal.is_some() && message_id.is_some() {
            break;
        }
    }
    (ordinal, message_id)
}

fn read_chunks(dir: &Path, meta: &SessionMeta) -> Result<Vec<u8>> {
    let index: ChunkIndex = serde_json::from_slice(&std::fs::read(dir.join("chunks.json"))?)?;
    let mut content = Vec::with_capacity(index.last_byte as usize);
    for chunk in &index.chunks {
        let path = dir.join("chunks").join(chunk_file_name(&chunk.id));
        let compressed = std::fs::read(&path)?;
        let mut decoder = GzDecoder::new(compressed.as_slice());
        let mut raw = Vec::new();
        decoder.read_to_end(&mut raw)?;
        if raw.len() as u64 != chunk.end_byte - chunk.start_byte {
            return Err(Error::Store(format!(
                "chunk {} length mismatch in {}",
                chunk.id, meta.timeline_id
            )));
        }
        if format!("sha256:{}", sha256_hex(&raw)) != chunk.hash {
            return Err(Error::Store(format!(
                "chunk {} hash mismatch in {}",
                chunk.id, meta.timeline_id
            )));
        }
        content.extend_from_slice(&raw);
    }
    if content.len() as u64 != index.last_byte {
        return Err(Error::Store(format!(
            "chunk index length mismatch in {}",
            meta.timeline_id
        )));
    }
    if !content.is_empty() {
        if format!("sha256:{}", sha256_hex(&content)) != index.head_hash {
            return Err(Error::Store(format!(
                "chunk index head mismatch in {}",
                meta.timeline_id
            )));
        }
        if sha256_hex(&content) != meta.content_hash {
            return Err(Error::Store(format!(
                "bundle content hash mismatch in {}",
                meta.timeline_id
            )));
        }
    }
    Ok(content)
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
            content_hash: sha256_hex(b"abc"),
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

    fn record(content: &[u8]) -> SessionRecord {
        let mut meta = sample_meta();
        meta.content_hash = sha256_hex(content);
        meta.size_bytes = content.len() as u64;
        SessionRecord::new(meta, content.to_vec())
    }

    fn user_line(i: usize, pad: usize) -> String {
        format!(
            "{{\"type\":\"user\",\"uuid\":\"u{i}\",\"text\":\"{}\"}}\n",
            "x".repeat(pad)
        )
    }

    #[test]
    fn chunks_append_only_new_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::init(tmp.path()).unwrap();
        let mut first = String::new();
        for i in 0..700 {
            first.push_str(&user_line(i, 120));
        }
        store.write_bundle(&record(first.as_bytes())).unwrap();

        let dir = store.bundle_dir(Agent::Claude, "tl");
        let before: ChunkIndex =
            serde_json::from_slice(&std::fs::read(dir.join("chunks.json")).unwrap()).unwrap();
        assert_eq!(before.chunks.len(), 2);
        assert_eq!(before.last_line, 700);
        assert!(!dir.join("session.jsonl.gz").exists());

        let mut second = first.clone();
        for i in 700..720 {
            second.push_str(&user_line(i, 120));
        }
        store.write_bundle(&record(second.as_bytes())).unwrap();

        let after: ChunkIndex =
            serde_json::from_slice(&std::fs::read(dir.join("chunks.json")).unwrap()).unwrap();
        assert_eq!(after.chunks.len(), 3, "append should add exactly one chunk");
        assert_eq!(&after.chunks[..2], &before.chunks[..]);
        assert_eq!(after.last_line, 720);
        assert_eq!(after.last_message_id.as_deref(), Some("u719"));
        assert!(after.head_hash.starts_with("sha256:"));
        assert_eq!(
            store.read_bundle(Agent::Claude, "tl").unwrap().content,
            second.as_bytes()
        );
    }

    #[test]
    fn chunks_rebuild_on_divergence() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::init(tmp.path()).unwrap();
        let mut first = String::new();
        for i in 0..600 {
            first.push_str(&user_line(i, 20));
        }
        store.write_bundle(&record(first.as_bytes())).unwrap();

        let mut replacement = String::new();
        for i in 0..10 {
            replacement.push_str(&user_line(10_000 + i, 20));
        }
        store.write_bundle(&record(replacement.as_bytes())).unwrap();

        let dir = store.bundle_dir(Agent::Claude, "tl");
        let index: ChunkIndex =
            serde_json::from_slice(&std::fs::read(dir.join("chunks.json")).unwrap()).unwrap();
        let files: std::collections::BTreeSet<String> = std::fs::read_dir(dir.join("chunks"))
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect();
        let expected: std::collections::BTreeSet<String> = index
            .chunks
            .iter()
            .map(|chunk| chunk_file_name(&chunk.id))
            .collect();
        assert_eq!(files, expected, "old chunk files should be pruned");
        assert_eq!(
            store.read_bundle(Agent::Claude, "tl").unwrap().content,
            replacement.as_bytes()
        );
    }

    #[test]
    fn reads_legacy_full_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::init(tmp.path()).unwrap();
        let content = b"{\"type\":\"user\",\"uuid\":\"u1\"}\n";
        let record = record(content);
        let dir = store.bundle_dir(Agent::Claude, "tl");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("meta.json"),
            serde_json::to_vec(&record.meta).unwrap(),
        )
        .unwrap();
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(content).unwrap();
        std::fs::write(dir.join("session.jsonl.gz"), encoder.finish().unwrap()).unwrap();

        let loaded = store.read_bundle(Agent::Claude, "tl").unwrap();
        assert_eq!(loaded.content, content);
    }
}
