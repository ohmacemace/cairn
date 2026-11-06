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
    if let Some(rest) = raw.strip_prefix("~\\") {
        return home_dir().join(rest);
    }
    PathBuf::from(raw)
}

/// Normalize a path for cross-platform comparison.
/// This is intentionally not used for display or file access: it only powers
/// project identity and equality checks.
pub fn normalize_path_key(raw: &str) -> String {
    normalize_path_key_impl(raw, cfg!(any(windows, target_os = "macos")))
}

fn normalize_path_key_impl(raw: &str, casefold: bool) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut path = trimmed.replace('\\', "/");
    if path.starts_with("//") {
        path = format!("//{}", path[2..].replace("//", "/"));
    } else {
        while path.contains("//") {
            path = path.replace("//", "/");
        }
    }

    if path.len() >= 2 && path.as_bytes()[1] == b':' && path.as_bytes()[0].is_ascii_alphabetic() {
        let upper = path[..1].to_ascii_uppercase();
        path.replace_range(..1, &upper);
    }

    let is_root = path == "/" || path == "//";
    let is_drive_root = path.len() == 3 && path.as_bytes()[1] == b':' && path.ends_with('/');
    if !is_root && !is_drive_root && path.ends_with('/') {
        path.pop();
    }

    if casefold {
        path.make_ascii_lowercase();
    }
    path
}

pub fn paths_equivalent(a: &str, b: &str) -> bool {
    normalize_path_key(a) == normalize_path_key(b)
}

fn path_variants(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    push_unique(&mut out, raw.to_string());
    push_unique(&mut out, raw.replace('\\', "/"));
    push_unique(&mut out, raw.replace('/', "\\"));

    if raw.len() >= 2 && raw.as_bytes()[1] == b':' && raw.as_bytes()[0].is_ascii_alphabetic() {
        let mut lower = raw.to_string();
        lower.replace_range(..1, &raw[..1].to_ascii_lowercase());
        push_unique(&mut out, lower);
        let mut upper = raw.to_string();
        upper.replace_range(..1, &raw[..1].to_ascii_uppercase());
        push_unique(&mut out, upper);
    }

    out
}

fn push_unique(out: &mut Vec<String>, value: String) {
    if !value.is_empty() && !out.contains(&value) {
        out.push(value);
    }
}

/// Split one `split_inclusive(b'\n')` chunk into (line without EOL, has_cr, has_lf).
pub fn split_line_ending(chunk: &[u8]) -> (&[u8], bool, bool) {
    let has_newline = chunk.last() == Some(&b'\n');
    let mut line = if has_newline {
        &chunk[..chunk.len() - 1]
    } else {
        chunk
    };
    let has_cr = line.last() == Some(&b'\r');
    if has_cr {
        line = &line[..line.len() - 1];
    }
    (line, has_cr, has_newline)
}

pub fn push_line_ending(out: &mut Vec<u8>, has_cr: bool, has_newline: bool) {
    if has_cr {
        out.push(b'\r');
    }
    if has_newline {
        out.push(b'\n');
    }
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

/// Git info for a path inside a WSL distro. Only runs on Windows; on other
/// platforms this returns an empty result and callers fall back to a local key.
pub fn wsl_git_info(distro: &str, cwd: &str) -> GitInfo {
    if !cfg!(windows) {
        return GitInfo::default();
    }

    let run = |args: &[&str]| -> Option<String> {
        Command::new("wsl.exe")
            .args(["-d", distro, "--"])
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };

    let remote = run(&["git", "-C", cwd, "remote", "get-url", "origin"]);
    let branch = run(&["git", "-C", cwd, "rev-parse", "--abbrev-ref", "HEAD"])
        .filter(|value| value != "HEAD");
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
        short_hash(normalize_path_key(&cwd.to_string_lossy()).as_bytes(), 8)
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
    let target = cwd.to_string_lossy();
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
                let line = line.trim_end_matches(['\r', '\n']);
                if let Ok(value) = serde_json::from_str::<Value>(line) {
                    if let Some(session_cwd) = value.get("cwd").and_then(Value::as_str) {
                        if paths_equivalent(session_cwd, &target) {
                            return true;
                        }
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

    let variants = path_variants(old);
    let mut out = Vec::with_capacity(content.len());
    for chunk in content.split_inclusive(|b| *b == b'\n') {
        let (line, has_cr, has_newline) = split_line_ending(chunk);
        if line.is_empty() {
            push_line_ending(&mut out, has_cr, has_newline);
            continue;
        }
        match serde_json::from_slice::<Value>(line) {
            Ok(mut value) => {
                rewrite_value(&mut value, &variants, new);
                if serde_json::to_writer(&mut out, &value).is_err() {
                    out.extend_from_slice(line);
                }
            }
            Err(_) => {
                let mut text = String::from_utf8_lossy(line).to_string();
                for variant in &variants {
                    text = text.replace(variant, new);
                }
                out.extend_from_slice(text.as_bytes());
            }
        }
        push_line_ending(&mut out, has_cr, has_newline);
    }
    out
}

fn rewrite_value(value: &mut Value, old_variants: &[String], new: &str) {
    match value {
        Value::String(s) => {
            for old in old_variants {
                if s.contains(old) {
                    *s = s.replace(old, new);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                rewrite_value(item, old_variants, new);
            }
        }
        Value::Object(map) => {
            for (_, item) in map.iter_mut() {
                rewrite_value(item, old_variants, new);
            }
        }
        _ => {}
    }
}

pub fn clean_title(text: &str) -> String {
    for line in text.lines() {
        if is_command_text(line) {
            continue;
        }
        let stripped = strip_tags(line);
        let trimmed = stripped.trim();
        if trimmed.is_empty() || is_command_text(trimmed) {
            continue;
        }
        return truncate_chars(trimmed, 80);
    }
    "Untitled session".to_string()
}

pub fn is_command_text(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with("<local-command")
        || trimmed.starts_with("<command-")
        || trimmed.starts_with("The command below was run directly in Claude Code")
        || trimmed.starts_with("Set effort level")
        || trimmed.starts_with("This session is being continued")
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

pub const CWD_PLACEHOLDER: &str = "__CAIRN_CWD__";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cwd_placeholder_round_trips() {
        let raw = b"{\"cwd\":\"/home/a/proj\",\"text\":\"/home/a/proj foo\"}\n";
        let canonical = rewrite_cwd(raw, "/home/a/proj", CWD_PLACEHOLDER);
        let restored = rewrite_cwd(&canonical, CWD_PLACEHOLDER, "/home/b/proj");
        assert_eq!(
            String::from_utf8(restored).unwrap(),
            "{\"cwd\":\"/home/b/proj\",\"text\":\"/home/b/proj foo\"}\n"
        );
    }

    #[test]
    fn normalizes_cross_platform_paths() {
        assert_eq!(
            normalize_path_key_impl("c:\\Users\\mace\\proj\\", false),
            "C:/Users/mace/proj"
        );
        assert_eq!(
            normalize_path_key_impl("C:/Users/Mace/Proj", true),
            "c:/users/mace/proj"
        );
        assert_eq!(normalize_path_key_impl("/", false), "/");
        assert_eq!(normalize_path_key_impl("C:/", false), "C:/");
        assert!(paths_equivalent("/home/a/proj/", "/home/a/proj"));
        assert!(paths_equivalent(
            "C:\\Users\\mace\\proj",
            "c:/Users/mace/proj"
        ));
    }

    #[test]
    fn rewrite_cwd_handles_windows_separators() {
        let raw = b"{\"cwd\":\"C:\\\\Users\\\\a\\\\proj\",\"text\":\"C:/Users/a/proj\"}\n";
        let out = rewrite_cwd(raw, "C:\\Users\\a\\proj", "D:\\work\\proj");
        let text = String::from_utf8(out).unwrap();
        let value: Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(value["cwd"], "D:\\work\\proj");
        assert_eq!(value["text"], "D:\\work\\proj");
    }

    #[test]
    fn rewrite_cwd_preserves_crlf() {
        let raw = b"{\"cwd\":\"/home/a/proj\"}\r\n{\"cwd\":\"/home/a/proj\"}\r\n";
        let out = rewrite_cwd(raw, "/home/a/proj", "/home/b/proj");
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "{\"cwd\":\"/home/b/proj\"}\r\n{\"cwd\":\"/home/b/proj\"}\r\n"
        );
    }
}
