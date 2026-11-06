use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::home_dir;
use crate::hash::short_hash;

#[derive(Debug, Clone, Default)]
pub struct GitInfo {
    pub remote: Option<String>,
    pub branch: Option<String>,
}

pub fn expand_tilde(raw: &str) -> PathBuf {
    if raw == "~" {
        return home_dir();
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return home_dir().join(rest);
    }
    PathBuf::from(raw)
}

/// Normalize common git remote URL forms into `host/owner/repo`.
pub fn normalize_git_remote(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    let without_scheme = if let Some(rest) = raw.strip_prefix("git@") {
        // git@github.com:owner/repo.git
        let (host, path) = rest.split_once(':')?;
        format!("{host}/{path}")
    } else if let Some(rest) = raw.strip_prefix("ssh://") {
        // ssh://git@github.com/owner/repo.git
        let rest = rest.rsplit_once('@').map(|(_, r)| r).unwrap_or(rest);
        rest.to_string()
    } else if let Some(rest) = raw.strip_prefix("https://") {
        rest.to_string()
    } else if let Some(rest) = raw.strip_prefix("http://") {
        rest.to_string()
    } else if raw.contains('/') {
        raw.to_string()
    } else {
        return None;
    };

    let without_user = if let Some((user, rest)) = without_scheme.split_once('@') {
        if user.contains('/') {
            without_scheme.as_str()
        } else {
            rest
        }
    } else {
        without_scheme.as_str()
    };

    let mut parts = without_user.splitn(2, '/');
    let host = parts.next()?.trim();
    let path = parts.next()?.trim();
    if host.is_empty() || path.is_empty() {
        return None;
    }

    let path = path.trim_end_matches('/').trim_end_matches(".git");
    let path = path.trim_start_matches('/');
    if path.is_empty() {
        return None;
    }

    Some(format!("{}/{}", host.to_ascii_lowercase(), path).to_ascii_lowercase())
}

pub fn git_info(cwd: &Path) -> GitInfo {
    let remote = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let branch = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "HEAD");

    GitInfo { remote, branch }
}

pub fn project_key(cwd: &Path, remote: Option<&str>) -> String {
    if let Some(normalized) = remote.and_then(normalize_git_remote) {
        return format!("git:{normalized}");
    }
    let name = cwd
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".to_string());
    format!(
        "local:{name}-{}",
        short_hash(cwd.to_string_lossy().as_bytes(), 8)
    )
}

/// Claude Code stores sessions under ~/.claude/projects/<encoded-cwd>/.
/// Observed encoding: replace path separators with '-'.
pub fn encode_claude_project_dir(cwd: &Path) -> String {
    let raw = cwd.to_string_lossy();
    let raw = raw.trim_end_matches(['/', '\\']);
    raw.replace(['/', '\\', ':'], "-")
}

/// Reuse an existing Claude project directory if one already maps to this cwd.
pub fn claude_project_dir_for(home: &Path, cwd: &Path) -> PathBuf {
    let projects = home.join("projects");
    if let Ok(entries) = std::fs::read_dir(&projects) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if dir_contains_cwd(&path, cwd) {
                return path;
            }
        }
    }
    projects.join(encode_claude_project_dir(cwd))
}

fn dir_contains_cwd(dir: &Path, cwd: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        if let Ok(file) = std::fs::File::open(&path) {
            let mut reader = BufReader::new(file);
            let mut line = String::new();
            if reader.read_line(&mut line).is_ok() {
                if let Ok(value) = serde_json::from_str::<Value>(&line) {
                    if value.get("cwd").and_then(Value::as_str) == Some(&cwd.to_string_lossy()) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Rewrite the old absolute cwd inside every JSON string of a JSONL session.
/// Structured fields and embedded environment context both live in strings,
/// so a prefix-safe replacement is enough and keeps line structure intact.
pub fn rewrite_cwd(content: &[u8], old: &str, new: &str) -> Vec<u8> {
    if old.is_empty() || old == new {
        return content.to_vec();
    }

    let mut out = Vec::with_capacity(content.len());
    for line in content.split(|b| *b == b'\n') {
        if line.is_empty() {
            out.push(b'\n');
            continue;
        }
        match serde_json::from_slice::<Value>(line) {
            Ok(mut value) => {
                rewrite_value(&mut value, old, new);
                if serde_json::to_writer(&mut out, &value).is_err() {
                    out.extend_from_slice(line);
                }
                out.push(b'\n');
            }
            Err(_) => {
                let text = String::from_utf8_lossy(line);
                out.extend_from_slice(text.replace(old, new).as_bytes());
                out.push(b'\n');
            }
        }
    }
    out
}

fn rewrite_value(value: &mut Value, old: &str, new: &str) {
    match value {
        Value::String(s) => {
            if s.contains(old) {
                *s = s.replace(old, new);
            }
        }
        Value::Array(items) => {
            for item in items {
                rewrite_value(item, old, new);
            }
        }
        Value::Object(map) => {
            for (_, item) in map.iter_mut() {
                rewrite_value(item, old, new);
            }
        }
        _ => {}
    }
}

pub fn clean_title(text: &str) -> String {
    for line in text.lines() {
        let stripped = strip_tags(line);
        let trimmed = stripped.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("Caveat:")
            || trimmed.starts_with("This session is being continued")
            || trimmed.starts_with("Set effort level")
        {
            continue;
        }
        return truncate_chars(trimmed, 80);
    }
    "Untitled session".to_string()
}

fn strip_tags(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

fn truncate_chars(input: &str, max: usize) -> String {
    let mut out: String = input.chars().take(max).collect();
    if input.chars().count() > max {
        out.push('…');
    }
    out
}

pub fn parse_timestamp(raw: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|d| d.timestamp())
}

pub fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn file_mtime(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or_else(now_ts)
}

pub fn is_git_repo(cwd: &Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn read_git_remote(cwd: &Path) -> Option<String> {
    git_info(cwd).remote
}

pub fn project_identity(cwd: &Path) -> (String, Option<String>, Option<String>) {
    let info = git_info(cwd);
    let remote = info.remote.as_deref().and_then(normalize_git_remote);
    let key = project_key(cwd, remote.as_deref());
    (key, remote, info.branch)
}
