use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct TestEnv {
    _temp: tempfile::TempDir,
    root: PathBuf,
    config: PathBuf,
    state: PathBuf,
    claude_a: PathBuf,
    claude_b: PathBuf,
    codex: PathBuf,
    store: PathBuf,
}

impl TestEnv {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        Self {
            _temp: temp,
            config: root.join("config.toml"),
            state: root.join("state.json"),
            claude_a: root.join("claude-a"),
            claude_b: root.join("claude-b"),
            codex: root.join("codex-home"),
            store: root.join("store"),
            root,
        }
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_cairn"));
        cmd.env("CAIRN_CONFIG", &self.config)
            .env("CAIRN_STATE", &self.state)
            .env("CAIRN_CLAUDE_HOME", &self.claude_a)
            .env("CAIRN_CODEX_HOME", &self.codex);
        cmd
    }

    fn command_with_claude(&self, home: &Path) -> Command {
        let mut cmd = self.command();
        cmd.env("CAIRN_CLAUDE_HOME", home);
        cmd
    }
}

fn run(cmd: &mut Command) -> Output {
    let out = cmd.output().expect("failed to spawn cairn");
    assert!(
        out.status.success(),
        "command failed: {cmd:?}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn run_args(env: &TestEnv, args: &[&str]) -> Output {
    let mut cmd = env.command();
    cmd.args(args);
    run(&mut cmd)
}

fn run_args_with_claude(env: &TestEnv, home: &Path, args: &[&str]) -> Output {
    let mut cmd = env.command_with_claude(home);
    cmd.args(args);
    run(&mut cmd)
}

fn encode(cwd: &Path) -> String {
    let raw = cwd.to_string_lossy();
    raw.trim_end_matches(['/', '\\'])
        .replace(['/', '\\', ':'], "-")
}

fn write_claude_session(home: &Path, cwd: &Path, id: &str, crlf: bool) -> PathBuf {
    let dir = home.join("projects").join(encode(cwd));
    fs::create_dir_all(&dir).unwrap();
    let cwd = cwd.to_string_lossy().to_string();

    let lines = [
        json!({"type": "mode", "sessionId": id}),
        json!({
            "type": "user",
            "sessionId": id,
            "cwd": cwd,
            "uuid": "u1",
            "timestamp": "2026-01-01T00:00:00Z",
            "message": {"role": "user", "content": "Fix the build"}
        }),
        json!({
            "type": "assistant",
            "sessionId": id,
            "cwd": cwd,
            "uuid": "u2",
            "parentUuid": "u1",
            "timestamp": "2026-01-01T00:00:01Z",
            "message": {"role": "assistant", "content": [{"type": "text", "text": "ok"}]}
        }),
        json!({
            "type": "attachment",
            "sessionId": id,
            "cwd": cwd,
            "uuid": "a1",
            "parentUuid": "u2",
            "timestamp": "2026-01-01T00:00:02Z",
            "attachment": {"type": "environment", "snapshot": {"workingDirectory": cwd}},
            "rendered": "environment"
        }),
    ];

    let mut text = String::new();
    for line in lines {
        text.push_str(&serde_json::to_string(&line).unwrap());
        if crlf {
            text.push_str("\r\n");
        } else {
            text.push('\n');
        }
    }

    let path = dir.join(format!("{id}.jsonl"));
    fs::write(&path, text).unwrap();
    path
}

fn first_session(env: &TestEnv) -> Value {
    let out = run_args(env, &["scan", "--json"]);
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    value.as_array().unwrap()[0].clone()
}

fn assert_status_clean(env: &TestEnv, home: &Path) {
    let out = run_args_with_claude(env, home, &["status", "--json"]);
    let status: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(status["pending_push"].as_array().unwrap().len(), 0);
    assert_eq!(status["pending_pull"].as_array().unwrap().len(), 0);
}

#[test]
fn claude_push_pull_restore_round_trip() {
    let env = TestEnv::new();
    let project_a = env.root.join("project-a");
    fs::create_dir_all(&project_a).unwrap();
    let id = "11111111-1111-4111-8111-111111111111";
    write_claude_session(&env.claude_a, &project_a, id, false);

    run_args(
        &env,
        &[
            "init",
            "--store",
            env.store.to_str().unwrap(),
            "--name",
            "test",
        ],
    );

    let session = first_session(&env);
    assert_eq!(session["id"].as_str().unwrap(), id);
    assert_eq!(session["title"].as_str().unwrap(), "Fix the build");

    run_args(&env, &["push"]);
    let bundle = env.store.join("threads").join("claude").join(id);
    assert!(bundle.join("meta.json").exists());
    assert!(bundle.join("session.jsonl.gz").exists());

    run_args_with_claude(&env, &env.claude_b, &["pull"]);
    assert_status_clean(&env, &env.claude_b);

    let project_b = env.root.join("project-b");
    fs::create_dir_all(&project_b).unwrap();
    run_args_with_claude(
        &env,
        &env.claude_b,
        &[
            "restore",
            id,
            "--agent",
            "claude",
            "--cwd",
            project_b.to_str().unwrap(),
        ],
    );

    let restored = env
        .claude_b
        .join("projects")
        .join(encode(&project_b))
        .join(format!("{id}.jsonl"));
    assert!(restored.exists(), "missing {}", restored.display());
    let text = fs::read_to_string(&restored).unwrap();
    assert!(text.contains(&project_b.to_string_lossy().to_string()));
    assert!(!text.contains("__CAIRN_CWD__"));
}

#[test]
fn crlf_session_round_trip() {
    let env = TestEnv::new();
    let project = env.root.join("crlf-project");
    fs::create_dir_all(&project).unwrap();
    let id = "22222222-2222-4222-8222-222222222222";
    write_claude_session(&env.claude_a, &project, id, true);

    run_args(
        &env,
        &[
            "init",
            "--store",
            env.store.to_str().unwrap(),
            "--name",
            "test",
        ],
    );

    let session = first_session(&env);
    assert_eq!(session["id"].as_str().unwrap(), id);
    assert_eq!(session["title"].as_str().unwrap(), "Fix the build");

    run_args(&env, &["push"]);
    run_args_with_claude(&env, &env.claude_b, &["pull"]);
    assert_status_clean(&env, &env.claude_b);

    let restored = env
        .claude_b
        .join("projects")
        .join(encode(&project))
        .join(format!("{id}.jsonl"));
    let content = fs::read(&restored).unwrap();
    let lf = content.iter().filter(|b| **b == b'\n').count();
    let crlf = content.windows(2).filter(|w| w == b"\r\n").count();
    assert!(lf > 0);
    assert_eq!(lf, crlf, "restored file mixed CRLF and LF");
    assert!(!String::from_utf8_lossy(&content).contains("__CAIRN_CWD__"));
}
