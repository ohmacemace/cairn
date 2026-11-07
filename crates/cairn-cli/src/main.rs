use cairn_core::config::Config;
use cairn_core::git;
use cairn_core::model::{Agent, SessionRecord};
use cairn_core::store::Store;
use cairn_core::sync::SyncEngine;
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Parser, Debug)]
#[command(
    name = "cairn",
    version,
    about = "Cloud session management for AI coding agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Connect a private git repository as the session store.
    Init {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        remote: Option<String>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        force: bool,
    },
    /// List sessions discovered on this machine.
    Scan {
        #[arg(long)]
        agent: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Compare local sessions with the store.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Save local sessions into the store.
    Push {
        #[arg(long)]
        dry_run: bool,
    },
    /// Refresh session store metadata without materializing sessions.
    Pull {
        #[arg(long)]
        dry_run: bool,
    },
    /// Restore one session by id.
    Restore {
        id: String,
        #[arg(long)]
        agent: Option<String>,
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(long)]
        open: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Check the local installation and store health.
    Doctor,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("cairn: {err}");
        std::process::exit(1);
    }
}

fn run() -> cairn_core::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init {
            store,
            remote,
            name,
            force,
        } => cmd_init(store, remote, name, force),
        Commands::Scan { agent, json } => cmd_scan(agent, json),
        Commands::Status { json } => cmd_status(json),
        Commands::Push { dry_run } => cmd_push(dry_run),
        Commands::Pull { dry_run } => cmd_pull(dry_run),
        Commands::Restore {
            id,
            agent,
            cwd,
            open,
            dry_run,
        } => cmd_restore(id, agent, cwd, open, dry_run),
        Commands::Doctor => cmd_doctor(),
    }
}

fn cmd_init(
    store: PathBuf,
    remote: Option<String>,
    name: Option<String>,
    force: bool,
) -> cairn_core::Result<()> {
    let config_path = Config::config_path();
    if config_path.exists() && !force {
        return Err(cairn_core::Error::Config(format!(
            "{} already exists; use --force to overwrite",
            config_path.display()
        )));
    }

    let store_exists = store.join(".git").exists();
    if !store_exists {
        if let Some(remote_url) = &remote {
            // Try a real clone first so an existing store is preserved.
            let clone = Command::new("git")
                .args(["clone", remote_url, &store.to_string_lossy()])
                .output();
            match clone {
                Ok(out) if out.status.success() => {}
                _ => {
                    Store::init(&store)?;
                    git::init(&store, remote.as_deref(), "main")?;
                }
            }
        } else {
            Store::init(&store)?;
            git::init(&store, None, "main")?;
        }
    }

    // A cloned store may not have the manifest yet.
    Store::init(&store)?;

    let config = Config::default_for(store.to_string_lossy().to_string(), remote, name);
    config.save()?;

    println!("cairn initialized");
    println!("  config:  {}", config_path.display());
    println!("  store:   {}", store.display());
    println!("  machine: {} ({})", config.machine_name, config.machine_id);
    Ok(())
}

fn cmd_scan(agent: Option<String>, json: bool) -> cairn_core::Result<()> {
    let config = Config::load()?;
    let engine = SyncEngine::new(config)?;
    let mut report = engine.scan()?;

    if let Some(agent) = agent {
        let agent: Agent = agent.parse()?;
        report.records.retain(|r| r.meta.agent == agent);
    }

    for warning in &report.warnings {
        eprintln!("warning: {warning}");
    }

    if json {
        let items: Vec<_> = report
            .records
            .iter()
            .map(|r| {
                serde_json::json!({
                    "agent": r.meta.agent.to_string(),
                    "id": r.meta.native_id,
                    "title": r.meta.title,
                    "project": r.meta.project_key,
                    "cwd": r.meta.cwd,
                    "updated_at": r.meta.updated_at,
                    "size_bytes": r.meta.size_bytes,
                    "source": r.meta.source,
                    "wsl_distro": r.meta.wsl_distro,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(());
    }

    println!(
        "{:<8} {:<38} {:<24} {:<18} {:>8}  TITLE",
        "AGENT", "ID", "PROJECT", "UPDATED", "SIZE"
    );
    for record in &report.records {
        let project = match &record.meta.wsl_distro {
            Some(distro) => format!("wsl:{distro}:{}", record.meta.project_key),
            None => record.meta.project_key.clone(),
        };
        println!(
            "{:<8} {:<38} {:<24} {:<18} {:>8}  {}",
            record.meta.agent.to_string(),
            record.meta.native_id,
            short(&project, 24),
            human_time(record.meta.updated_at),
            human_bytes(record.meta.size_bytes),
            record.meta.title
        );
    }
    println!("\n{} session(s)", report.records.len());
    Ok(())
}

fn cmd_status(json: bool) -> cairn_core::Result<()> {
    let config = Config::load()?;
    let engine = SyncEngine::new(config)?;
    let status = engine.status()?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "local": status.local,
                "remote": status.remote,
                "pending_push": status.pending_push,
                "pending_pull": status.pending_pull,
                "conflicts": status.conflicts,
                "warnings": status.warnings,
            }))?
        );
        return Ok(());
    }

    for warning in &status.warnings {
        eprintln!("warning: {warning}");
    }
    println!("local sessions:  {}", status.local);
    println!("store sessions:  {}", status.remote);
    println!("pending push:    {}", status.pending_push.len());
    println!("pending pull:    {}", status.pending_pull.len());
    for key in &status.pending_push {
        println!("  push  {key}");
    }
    for key in &status.pending_pull {
        println!("  pull  {key}");
    }
    Ok(())
}

fn cmd_push(dry_run: bool) -> cairn_core::Result<()> {
    let config = Config::load()?;
    let engine = SyncEngine::new(config)?;
    let report = engine.push(dry_run)?;

    for warning in &report.warnings {
        eprintln!("warning: {warning}");
    }
    for conflict in &report.conflicts {
        eprintln!("conflict: {conflict}");
    }
    println!(
        "{}added {}, updated {}, unchanged {}{}",
        if dry_run { "[dry-run] " } else { "" },
        report.added,
        report.updated,
        report.unchanged,
        if report.committed { ", committed" } else { "" }
    );
    Ok(())
}

fn cmd_pull(dry_run: bool) -> cairn_core::Result<()> {
    let config = Config::load()?;
    let engine = SyncEngine::new(config)?;
    let report = engine.pull(dry_run)?;

    for warning in &report.warnings {
        eprintln!("warning: {warning}");
    }
    for note in &report.notes {
        println!("{note}");
    }
    println!(
        "{}store sessions: {}",
        if dry_run { "[dry-run] " } else { "" },
        report.remote
    );
    Ok(())
}

fn cmd_restore(
    id: String,
    agent: Option<String>,
    cwd: Option<PathBuf>,
    open: bool,
    dry_run: bool,
) -> cairn_core::Result<()> {
    let agent = agent.map(|a| a.parse()).transpose()?;
    let config = Config::load()?;
    let engine = SyncEngine::new(config)?;
    let outcome = engine.restore(&id, agent, cwd.as_deref(), dry_run)?;

    println!(
        "{} session {} -> {}",
        if outcome.imported {
            "restored"
        } else {
            "not imported"
        },
        outcome.native_id,
        outcome.path.display()
    );
    if let Some(note) = &outcome.note {
        println!("  {note}");
    }

    if open && !dry_run {
        let meta = engine
            .store
            .find(agent.unwrap_or(Agent::Codex), &outcome.native_id)?
            .or(engine.store.find(Agent::Claude, &outcome.native_id)?)
            .ok_or_else(|| cairn_core::Error::NotFound(outcome.native_id.clone()))?;
        let cmd = engine.resume_command(meta.agent, &meta.native_id)?;
        let target_dir = cwd
            .clone()
            .unwrap_or_else(|| engine.config.resolve_cwd(&meta));
        println!("running: {}", cmd.join(" "));
        let status = Command::new(&cmd[0])
            .args(&cmd[1..])
            .current_dir(&target_dir)
            .status()?;
        if !status.success() {
            return Err(cairn_core::Error::Adapter(format!(
                "resume command exited with {status}"
            )));
        }
    }
    Ok(())
}

fn cmd_doctor() -> cairn_core::Result<()> {
    let config_path = Config::config_path();
    println!("config: {}", config_path.display());
    match Config::load() {
        Ok(config) => {
            println!("  schema: {}", config.schema);
            println!("  machine: {} ({})", config.machine_name, config.machine_id);
            println!("  store: {}", config.store_path().display());
            match Store::open(&config.store_path()) {
                Ok(_) => println!("  store manifest: ok"),
                Err(err) => println!("  store manifest: {err}"),
            }
            println!("  codex home: {}", config.codex_home().display());
            println!("  claude home: {}", config.claude_home().display());
            if config.wsl.is_empty() {
                println!("  wsl sources: none");
            } else {
                for source in &config.wsl {
                    println!(
                        "  wsl [{}] root: {}",
                        source.distro,
                        source.root().display()
                    );
                    if let Some(path) = source.codex_home() {
                        println!("    codex:  {} (exists: {})", path.display(), path.exists());
                    }
                    if let Some(path) = source.claude_home() {
                        println!("    claude: {} (exists: {})", path.display(), path.exists());
                    }
                }
            }
        }
        Err(err) => println!("  {err}"),
    }

    for (name, args) in [
        ("git", vec!["--version"]),
        ("codex", vec!["--version"]),
        ("claude", vec!["--version"]),
    ] {
        let output = Command::new(name).args(args).output();
        match output {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
                println!("{name}: ok ({text})");
            }
            _ => println!("{name}: not found"),
        }
    }

    if let Ok(config) = Config::load() {
        let store = config.store_path();
        if store.join(".git").exists() {
            match git::status_porcelain(&store) {
                Ok(status) if status.trim().is_empty() => println!("store git: clean"),
                Ok(_) => println!("store git: uncommitted changes"),
                Err(err) => println!("store git: {err}"),
            }
        }
    }
    Ok(())
}

fn short(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let mut out: String = value.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn human_time(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .unwrap_or_else(chrono::Utc::now)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} B", bytes)
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[allow(dead_code)]
fn _unused(_: &SessionRecord, _: &Path) {}
