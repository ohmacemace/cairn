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
  configPath: "~/.config/cairn/config.toml",
  storeManifest: "~/Cairn/cairn.json",
};

let mockSessions: SessionView[] = [
  {
    id: "01a1b1a0-2239-7012-829e-fc4ff70993b7",
    timelineId: "01a1b1a0-2239-7012-829e-fc4ff70993b7",
    familyId: "01a1b1a0-2239-7012-829e-fc4ff70993b7",
    parentTimeline: null,
    agent: "codex",
    title: "重构支付模块",
    project: "git:github.com/acme/payments",
    cwd: "/home/you/code/payments",
    updatedAt: now - 3600,
    sizeBytes: 1_840_000,
    source: "cli",
    wslDistro: null,
    machineName: "macbook",
    local: true,
    remote: true,
    cwdExists: true,
    mappedCwd: null,
  },
  {
    id: "01a1b1a0-2239-7012-829e-fc4ff70993b7",
    timelineId: "7c2f6e4a-1b2c-4d5e-8f90-123456789abc",
    familyId: "01a1b1a0-2239-7012-829e-fc4ff70993b7",
    parentTimeline: "01a1b1a0-2239-7012-829e-fc4ff70993b7",
    agent: "codex",
    title: "重构支付模块（分叉）",
    project: "git:github.com/acme/payments",
    cwd: "/home/you/code/payments",
    updatedAt: now - 1800,
    sizeBytes: 1_960_000,
    source: "cli",
    wslDistro: null,
    machineName: "windows-box",
    local: false,
    remote: true,
    cwdExists: false,
    mappedCwd: null,
  },
  {
    id: "cf4f3229-7204-4bd9-ae29-a9d9d89ee421",
    timelineId: "cf4f3229-7204-4bd9-ae29-a9d9d89ee421",
    familyId: "cf4f3229-7204-4bd9-ae29-a9d9d89ee421",
    parentTimeline: null,
    agent: "claude",
    title: "我想做一个开源项目",
    project: "git:github.com/you/cairn",
    cwd: "/home/you/code/cairn",
    updatedAt: now - 7200,
    sizeBytes: 704_000,
    source: "cli",
    wslDistro: null,
    machineName: "linux-desktop",
    local: true,
    remote: true,
    cwdExists: true,
    mappedCwd: null,
  },
  {
    id: "44444444-4444-4444-8444-444444444444",
    timelineId: "44444444-4444-4444-8444-444444444444",
    familyId: "44444444-4444-4444-8444-444444444444",
    parentTimeline: null,
    agent: "codex",
    title: "Fix the WSL build",
    project: "local:wsl-proj",
    cwd: "/home/you/proj",
    updatedAt: now - 86_400,
    sizeBytes: 220_000,
    source: "cli",
    wslDistro: "Ubuntu",
    machineName: "windows-box (WSL: Ubuntu)",
    local: true,
    remote: false,
    cwdExists: true,
    mappedCwd: null,
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
      configPath: "~/.config/cairn/config.toml",
      storeManifest: `${storePath.replace(/\/$/, "")}/cairn.json`,
    };
    return mockConfig;
  }
  return invoke<ConfigView>("init_store", { storePath, remote, machineName });
}

export async function listSessions(): Promise<ScanResult> {
  if (!isTauri) return { sessions: mockSessions, warnings: [] };
  return invoke<ScanResult>("list_sessions");
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
  timelineId: string,
  agent: Agent,
  cwd: string | null,
  remember: boolean,
): Promise<RestoreResult> {
  if (!isTauri) {
    const session = mockSessions.find((item) => item.timelineId === timelineId);
    const target = cwd || session?.mappedCwd || session?.cwd || "~";
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
  return invoke<RestoreResult>("restore_session", {
    id,
    timelineId,
    agent,
    cwd,
    remember,
  });
}

export async function updateStore(
  storePath: string,
  remote: string | null,
  machineName: string | null,
): Promise<ConfigView> {
  if (!isTauri && mockConfig) {
    mockConfig = {
      ...mockConfig,
      storePath,
      storeManifest: `${storePath.replace(/\/$/, "")}/cairn.json`,
      remote,
      machineName: machineName?.trim() || mockConfig.machineName,
    };
    return mockConfig;
  }
  return invoke<ConfigView>("update_store", { storePath, remote, machineName });
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
