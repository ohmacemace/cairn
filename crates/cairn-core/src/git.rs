use std::path::Path;
use std::process::{Command, Output};

use crate::error::{Error, Result};

pub fn ensure_available() -> Result<()> {
    let output = Command::new("git").arg("--version").output();
    match output {
        Ok(o) if o.status.success() => Ok(()),
        _ => Err(Error::Git(
            "git executable not found. Please install Git and make sure it is on PATH.".into(),
        )),
    }
}

pub fn init(root: &Path, remote: Option<&str>, branch: &str) -> Result<()> {
    ensure_available()?;
    std::fs::create_dir_all(root)?;

    if !root.join(".git").exists() {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["init", "-b", branch])
            .output()?;
        if !output.status.success() {
            // Older Git versions do not support `git init -b`.
            run_ok(Command::new("git").arg("-C").arg(root).arg("init"))?;
            let _ = Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["checkout", "-b", branch])
                .output();
        }
    }

    if let Some(remote) = remote {
        if has_remote(root, "origin") {
            let _ = Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["remote", "set-url", "origin", remote])
                .output();
        } else {
            run_ok(
                Command::new("git")
                    .arg("-C")
                    .arg(root)
                    .args(["remote", "add", "origin", remote]),
            )?;
        }
    }

    Ok(())
}

pub fn has_remote(root: &Path, name: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["remote", "get-url", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn has_commits(root: &Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--verify", "HEAD"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Pull with rebase. A freshly initialized store without remote or commits is a no-op.
pub fn pull(root: &Path, branch: &str) -> Result<()> {
    if !has_remote(root, "origin") {
        return Ok(());
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["pull", "--rebase", "--autostash", "origin", branch])
        .output()?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    // An empty remote branch is not an error for a brand new store.
    if stderr.contains("couldn't find remote ref")
        || stderr.contains("no such ref was fetched")
        || stderr.contains("couldn't find remote ref")
    {
        return Ok(());
    }
    Err(Error::Git(format!("git pull failed: {}", stderr.trim())))
}

pub fn commit_all(root: &Path, message: &str) -> Result<bool> {
    if !root.join(".git").exists() {
        return Ok(false);
    }
    run_ok(Command::new("git").arg("-C").arg(root).args(["add", "-A"]))?;

    let diff = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["diff", "--cached", "--quiet"])
        .output()?;
    if diff.status.success() {
        return Ok(false);
    }

    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "user.name=cairn",
            "-c",
            "user.email=cairn@localhost",
            "commit",
            "-m",
            message,
        ])
        .output()?;
    if !output.status.success() {
        return Err(Error::Git(format!(
            "git commit failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(true)
}

pub fn push(root: &Path, branch: &str) -> Result<()> {
    if !has_remote(root, "origin") {
        return Ok(());
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["push", "-u", "origin", branch])
        .output()?;
    if !output.status.success() {
        return Err(Error::Git(format!(
            "git push failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

pub fn status_porcelain(root: &Path) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain"])
        .output()?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn run_ok(command: &mut Command) -> Result<Output> {
    let output = command.output()?;
    if !output.status.success() {
        return Err(Error::Git(format!(
            "git command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output)
}
