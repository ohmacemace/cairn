use cairn_core::paths::paths_equivalent;
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

    fn command_for(&self, config: &Path, state: &Path, claude_home: &Path) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_cairn"));
        cmd.env("CAIRN_CONFIG", config)
            .env("CAIRN_STATE", state)
            .env("CAIRN_CLAUDE_HOME", claude_home)
            .env("CAIRN_CODEX_HOME", &self.codex);
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

fn toml_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "\\\\")
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

fn append_claude_turn(home: &Path, cwd: &Path, id: &str, text: &str) {
    let path = home
        .join("projects")
        .join(encode(cwd))
        .join(format!("{id}.jsonl"));
    let mut raw = fs::read_to_string(&path).unwrap();
    let cwd = cwd.to_string_lossy().to_string();
    let user = json!({
        "type": "user",
        "sessionId": id,
        "cwd": cwd,
        "uuid": uuid::Uuid::new_v4().to_string(),
        "timestamp": "2026-01-01T00:00:03Z",
        "message": {"role": "user", "content": text}
    });
    let assistant = json!({
        "type": "assistant",
        "sessionId": id,
        "cwd": cwd,
        "uuid": uuid::Uuid::new_v4().to_string(),
        "timestamp": "2026-01-01T00:00:04Z",
        "message": {"role": "assistant", "content": [{"type": "text", "text": format!("done {text}")}]}
    });
    raw.push_str(&serde_json::to_string(&user).unwrap());
    raw.push('\n');
    raw.push_str(&serde_json::to_string(&assistant).unwrap());
    raw.push('\n');
    fs::write(&path, raw).unwrap();
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
    let restored_cwd = text
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find_map(|value| {
            value
                .get("cwd")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .expect("restored session should contain a cwd");
    assert!(paths_equivalent(
        &restored_cwd,
        project_b.to_string_lossy().as_ref()
    ));
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

#[test]
fn divergent_push_creates_fork_with_point() {
    let env = TestEnv::new();
    let project = env.root.join("fork-project");
    fs::create_dir_all(&project).unwrap();
    let id = "55555555-5555-4555-8555-555555555555";
    write_claude_session(&env.claude_a, &project, id, false);

    run_args(
        &env,
        &[
            "init",
            "--store",
            env.store.to_str().unwrap(),
            "--name",
            "machine-a",
        ],
    );
    run_args(&env, &["push"]);

    let config_b = env.root.join("config-b.toml");
    let state_b = env.root.join("state-b.json");
    let claude_b = env.root.join("claude-b");

    let mut init_b = env.command_for(&config_b, &state_b, &claude_b);
    init_b.args([
        "init",
        "--store",
        env.store.to_str().unwrap(),
        "--name",
        "machine-b",
    ]);
    run(&mut init_b);

    let mut pull_b = env.command_for(&config_b, &state_b, &claude_b);
    pull_b.arg("pull");
    run(&mut pull_b);

    append_claude_turn(&env.claude_a, &project, id, "change from A");
    append_claude_turn(&claude_b, &project, id, "change from B");

    let mut push_a = env.command_for(&env.config, &env.state, &env.claude_a);
    push_a.arg("push");
    run(&mut push_a);

    let mut push_b = env.command_for(&config_b, &state_b, &claude_b);
    push_b.arg("push");
    let out = run(&mut push_b);
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(
        text.contains("diverged") || text.contains("fork"),
        "expected a fork notice, got: {text}"
    );

    let claude_dir = env.store.join("threads").join("claude");
    let mut forks = Vec::new();
    for entry in fs::read_dir(&claude_dir).unwrap() {
        let entry = entry.unwrap();
        let meta_path = entry.path().join("meta.json");
        if !meta_path.exists() {
            continue;
        }
        let meta: Value = serde_json::from_slice(&fs::read(&meta_path).unwrap()).unwrap();
        if meta["parent_timeline"].as_str() == Some(id) {
            forks.push(meta);
        }
    }
    assert_eq!(forks.len(), 1, "expected exactly one fork bundle");
    let fork = &forks[0];
    assert_ne!(fork["timeline_id"].as_str().unwrap(), id);
    assert_eq!(fork["family_id"].as_str().unwrap(), id);
    let point = &fork["fork_point"];
    assert!(point["line"].as_u64().unwrap() > 0);
    assert!(point["byte"].as_u64().unwrap() > 0);
    assert!(point["hash"].as_str().unwrap().starts_with("sha256:"));
    assert_eq!(point["parentTimeline"].as_str().unwrap(), id);
}

#[test]
fn wsl_file_level_round_trip() {
    let env = TestEnv::new();
    let fake_root = env.root.join("fake-wsl");
    let claude_home = fake_root.join("home").join("test").join(".claude");
    let codex_home = fake_root.join("home").join("test").join(".codex");
    fs::create_dir_all(&claude_home).unwrap();
    fs::create_dir_all(
        codex_home
            .join("sessions")
            .join("2026")
            .join("01")
            .join("01"),
    )
    .unwrap();

    let claude_id = "33333333-3333-4333-8333-333333333333";
    write_claude_session(&claude_home, Path::new("/home/test/proj"), claude_id, false);

    let codex_id = "44444444-4444-4444-8444-444444444444";
    let codex_path = codex_home
        .join("sessions")
        .join("2026")
        .join("01")
        .join("01")
        .join(format!("rollout-2026-01-01T00-00-00-{codex_id}.jsonl"));
    let lines = [
        json!({
            "type": "session_meta",
            "payload": {
                "id": codex_id,
                "cwd": "/home/test/proj",
                "timestamp": "2026-01-01T00:00:00Z",
                "model_provider": "openai",
                "source": "cli"
            }
        }),
        json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "Fix the WSL build"}]
            }
        }),
    ];
    let mut text = String::new();
    for line in lines {
        text.push_str(&serde_json::to_string(&line).unwrap());
        text.push_str("\r\n");
    }
    fs::write(&codex_path, text).unwrap();

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
    let mut config = fs::read_to_string(&env.config).unwrap();
    config.push_str(&format!(
        "\n[[wsl]]\ndistro = \"Ubuntu\"\ncodex_home = \"/home/test/.codex\"\nclaude_home = \"/home/test/.claude\"\nunc_root = \"{}\"\n",
        toml_path(&fake_root)
    ));
    fs::write(&env.config, config).unwrap();

    let out = run_args(&env, &["scan", "--json"]);
    let items: Vec<Value> = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(items.len(), 2);
    assert!(items
        .iter()
        .all(|item| item["wsl_distro"].as_str() == Some("Ubuntu")));
    assert!(items.iter().any(|item| {
        item["id"].as_str() == Some(claude_id) && item["title"].as_str() == Some("Fix the build")
    }));
    assert!(items.iter().any(|item| {
        item["id"].as_str() == Some(codex_id) && item["title"].as_str() == Some("Fix the WSL build")
    }));

    run_args(&env, &["push"]);
    assert!(env
        .store
        .join("threads")
        .join("claude")
        .join(claude_id)
        .join("meta.json")
        .exists());
    assert!(env
        .store
        .join("threads")
        .join("codex")
        .join(codex_id)
        .join("meta.json")
        .exists());

    run_args_with_claude(&env, &env.claude_b, &["pull"]);
    assert_status_clean(&env, &env.claude_b);

    run_args(
        &env,
        &[
            "restore",
            claude_id,
            "--agent",
            "claude",
            "--cwd",
            "/home/test/proj2",
        ],
    );
    let restored = claude_home
        .join("projects")
        .join(encode(Path::new("/home/test/proj2")))
        .join(format!("{claude_id}.jsonl"));
    assert!(restored.exists(), "missing {}", restored.display());
    let restored_text = fs::read_to_string(&restored).unwrap();
    assert!(restored_text.contains("/home/test/proj2"));

    run_args(
        &env,
        &[
            "restore",
            codex_id,
            "--agent",
            "codex",
            "--cwd",
            "/home/test/proj2",
        ],
    );
    let codex_text = fs::read_to_string(&codex_path).unwrap();
    assert!(codex_text.contains("/home/test/proj2"));
    assert!(!codex_home.join("state_5.sqlite").exists());
}
