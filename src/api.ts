import { invoke } from "@tauri-apps/api/core";
import type {
  Agent,
  ConfigView,
  DoctorResult,
  PullResult,
  PushResult,
  RestoreResult,
  ScanResult,
  SessionView,
  StatusView,
  WslSourceView,
} from "./types";

const isTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

const now = Math.floor(Date.now() / 1000);

let mockConfig: ConfigView | null = {
  machineName: "preview-machine",
  storePath: "~/Cairn",
  remote: "git@github.com:you/cairn-store.git",
  saveMode: "standard",
  agents: { codex: true, claude: true },
  wsl: [{ distro: "Ubuntu", codexHome: "/home/you/.codex", claudeHome: "/home/you/.claude" }],
};

let mockSessions: SessionView[] = [
  {
    id: "01a1b1a0-2239-7012-829e-fc4ff70993b7",
    agent: "codex",
    title: "重构支付模块",
    project: "git:github.com/acme/payments",
    cwd: "/home/you/code/payments",
    updatedAt: now - 3600,
    sizeBytes: 1_840_000,
    source: "cli",
    wslDistro: null,
    machineName: "macbook",
  },
  {
    id: "cf4f3229-7204-4bd9-ae29-a9d9d89ee421",
    agent: "claude",
    title: "我想做一个开源项目",
    project: "git:github.com/you/cairn",
    cwd: "/home/you/code/cairn",
    updatedAt: now - 7200,
    sizeBytes: 704_000,
    source: "cli",
    wslDistro: null,
    machineName: "linux-desktop",
  },
  {
    id: "44444444-4444-4444-8444-444444444444",
    agent: "codex",
    title: "Fix the WSL build",
    project: "local:wsl-proj",
    cwd: "/home/you/proj",
    updatedAt: now - 86_400,
    sizeBytes: 220_000,
    source: "cli",
    wslDistro: "Ubuntu",
    machineName: "windows-box (WSL: Ubuntu)",
  },
];

export async function getConfig(): Promise<ConfigView | null> {
  if (!isTauri) return mockConfig;
  return invoke<ConfigView | null>("get_config");
}

export async function initStore(
  storePath: string,
  remote: string | null,
  machineName: string | null,
): Promise<ConfigView> {
  if (!isTauri) {
    mockConfig = {
      machineName: machineName || "preview-machine",
      storePath,
      remote,
      saveMode: "standard",
      agents: { codex: true, claude: true },
      wsl: [],
    };
    return mockConfig;
  }
  return invoke<ConfigView>("init_store", { storePath, remote, machineName });
}

export async function scanSessions(): Promise<ScanResult> {
  if (!isTauri) return { sessions: mockSessions, warnings: [] };
  return invoke<ScanResult>("scan_sessions");
}

export async function syncStatus(): Promise<StatusView> {
  if (!isTauri) {
    return {
      local: mockSessions.length,
      remote: mockSessions.length,
      pendingPush: [],
      pendingPull: [],
      warnings: [],
    };
  }
  return invoke<StatusView>("sync_status");
}

export async function pushStore(): Promise<PushResult> {
  if (!isTauri) {
    return {
      added: 0,
      updated: 0,
      unchanged: mockSessions.length,
      committed: false,
      conflicts: [],
      warnings: [],
    };
  }
  return invoke<PushResult>("push_store");
}

export async function pullStore(): Promise<PullResult> {
  if (!isTauri) {
    return {
      imported: 0,
      skipped: mockSessions.length,
      conflicts: [],
      warnings: [],
      notes: [],
    };
  }
  return invoke<PullResult>("pull_store");
}

export async function restoreSession(
  id: string,
  agent: Agent,
  cwd: string | null,
): Promise<RestoreResult> {
  if (!isTauri) {
    const session = mockSessions.find((item) => item.id === id);
    const target = cwd || session?.cwd || "~";
    return {
      imported: true,
      nativeId: id,
      path: target,
      note: "preview mode: no files were written",
      resumeHint:
        session?.agent === "codex"
          ? `codex resume ${id}`
          : `claude --resume ${id}`,
    };
  }
  return invoke<RestoreResult>("restore_session", { id, agent, cwd });
}

export async function saveSettings(
  saveMode: ConfigView["saveMode"],
  agents: ConfigView["agents"],
  wsl: WslSourceView[],
): Promise<ConfigView> {
  if (!isTauri && mockConfig) {
    mockConfig = { ...mockConfig, saveMode, agents, wsl };
    return mockConfig;
  }
  return invoke<ConfigView>("save_settings", { saveMode, agents, wsl });
}

export async function runDoctor(): Promise<DoctorResult> {
  if (!isTauri) {
    return {
      lines: [
        "config: ~/.config/cairn/config.toml",
        "store manifest: ok",
        "codex home: ~/.codex",
        "claude home: ~/.claude",
        "wsl: Ubuntu (preview)",
        "git: ok",
      ],
    };
  }
  return invoke<DoctorResult>("run_doctor");
}

export function isDesktop(): boolean {
  return isTauri;
}
