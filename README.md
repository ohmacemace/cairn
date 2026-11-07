# Cairn

**Cloud session management for AI coding agents.**

Cairn keeps your Codex and Claude Code sessions in a private Git repository you own, so you can pick up the same conversation on another machine without recreating context.

> Status: v0.1 development. Core engine, CLI, automatic timeline forks, merge sessions, and the Tauri desktop app are implemented. Packaging and real-device validation are in progress.

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

## Desktop app

```bash
npm install
npm run tauri dev      # Tauri window
npm run dev            # browser preview with mock data
```

Linux requires the usual Tauri system packages:

```bash
sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev patchelf pkg-config
```

CI builds the desktop shell on Ubuntu, macOS, and Windows.

## Quick start

```bash
# 1. Connect a private Git repository as the session store.
cairn init \
  --store ~/Sync/cairn \
  --remote git@github.com:you/cairn-store.git

# 2. See local sessions.
cairn scan

# 3. Save them to the store.
cairn push

# 4. On another machine, restore and continue.
cairn pull
cairn restore <session-id> --cwd ~/code/project
```

`restore --open` runs the native resume command after restoring.

`cairn pull` only refreshes the session store metadata. Session files are
materialized one at a time by `cairn restore` or from the desktop app, so a
large store never rewrites every agent session on every sync.

## Timelines and merge

Session files are append-only. When two machines continue the same session, the first push keeps the original timeline and the later push is stored as a fork. Cairn records the fork point (line, byte offset, prefix hash) at the moment divergence is detected, so later operations do not have to rescan the whole history. Session JSONL is never text-merged by Git.

To combine two timelines, open the session in the desktop app and choose **Merge timelines**:

1. Cairn extracts a structured material pack from the fork point onward: user requests, assistant results, changed files, commands, errors, and existing compaction summaries.
2. It invokes the session's own agent CLI in read-only mode to turn the pack into a Markdown brief: `codex exec --sandbox read-only --ephemeral` or `claude --print --tools "" --no-session-persistence`. Cairn does not call a model API itself.
3. Review or edit the brief, then create a merged session. The new session gets a fresh native id, the brief as its first user message, and `mergedFrom` metadata. The source timelines are archived, not deleted.

Code conflicts are not merged by Cairn; resolve them in the project's own Git repository.

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
- [x] M2: timelines, automatic forks, fork points, merge sessions
- [x] M3: Tauri desktop app
- [ ] M4: polish, auto-sync, tray, packaging validation, first public release

## License

MIT
