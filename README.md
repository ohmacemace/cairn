# Cairn

**Cloud session management for AI coding agents.**

Cairn keeps your Codex and Claude Code sessions in a private Git repository you own, so you can pick up the same conversation on another machine without recreating context.

> Status: early development (M1). The core engine and CLI work on Linux. The desktop app, macOS/Windows packaging, and timeline UI are in progress.

## The problem

AI coding sessions live in local files:

- Codex: `~/.codex`
- Claude Code: `~/.claude/projects`

Switch computers and the sessions do not follow you. Cairn makes them portable by storing a compact, resumable copy in a Git repository that you control.

## Design principles

- **Bring your own cloud**: any private Git remote works (GitHub, Gitee, GitLab, self-hosted).
- **Same session, not a copy**: restore materializes the original session id so the native tool resumes the existing conversation.
- **Never sync secrets**: `auth.json`, credentials, and machine-local config are excluded.
- **Timelines, not lost work**: when two machines change the same session, both versions are preserved as separate timelines.
- **Native handoff**: continue in Codex Desktop, Codex CLI, or Claude Code. Cairn does not replace the agent runtime.

## Install

Requires Rust 1.85+ and Git.

```bash
cargo build --release -p cairn-cli
# binary: target/release/cairn
```

## Quick start

```bash
# 1. Connect a private Git repository as the session store.
cairn init \
  --store ~/Sync/cairn \
  --remote git@github.com:you/cairn-store.git \
  --git-name "Your Name" \
  --git-email "you@example.com"

# 2. See local sessions.
cairn scan

# 3. Save them to the store.
cairn push

# 4. On another machine, restore and continue.
cairn pull
cairn restore <session-id> --cwd ~/code/project
```

`restore --open` runs the native resume command after restoring.

## Save modes

| Mode | What is stored | Resumable |
|---|---|---|
| `index` | metadata and preview only | no |
| `standard` (default) | transcript with internal caches dropped and large blocks truncated | yes |
| `full` | byte-for-byte session content and sidecars | yes |

Configure in `~/.config/cairn/config.toml`:

```toml
[save]
mode = "standard"
attachments = false
max_text_block_bytes = 8192
```

Codex sessions are currently stored as full rollouts (gzip) until trimming is validated against live resumes.

## WSL (file-level)

On Windows, Cairn can read and restore sessions inside WSL through the
`\\wsl.localhost\<distro>` UNC path. Add one or more sources to
`~/.config/cairn/config.toml`:

```toml
[[wsl]]
distro = "Ubuntu"
codex_home = "/home/you/.codex"
claude_home = "/home/you/.claude"
# unc_root = "\\\\wsl.localhost\\Ubuntu"  # optional override
```

`codex_home` and `claude_home` are Linux paths inside the distro.

This is file-level support only: Cairn never opens or writes WSL SQLite
databases. Codex may rebuild its index on the next start. Cairn does not
auto-launch agents inside WSL; open Codex or Claude Code there yourself.

## Security

Session transcripts can contain source code, command output, and secrets. **Always use a private repository.** Cairn refuses to sync authentication files, but it cannot inspect every line of a transcript. Review your own risk before pushing.

## Roadmap

- [x] M0: validate Codex / Claude Code restore paths
- [x] M1: core engine, Git store, CLI, Claude + Codex adapters
- [ ] M2: timelines, leases, conflict handling, Codex trimming validation
- [ ] M3: Tauri desktop app, three-platform packaging
- [ ] M4: polish, auto-sync, tray, first public release

## License

MIT
