export type Agent = "codex" | "claude";
export type SaveMode = "index" | "standard" | "full";

export interface AgentSettings {
  codex: boolean;
  claude: boolean;
}

export interface WslSourceView {
  distro: string;
  codexHome?: string | null;
  claudeHome?: string | null;
}

export interface ConfigView {
  machineName: string;
  storePath: string;
  remote?: string | null;
  saveMode: SaveMode;
  agents: AgentSettings;
  wsl: WslSourceView[];
  configPath: string;
  storeManifest: string;
}

export interface SessionView {
  id: string;
  timelineId: string;
  familyId: string;
  parentTimeline?: string | null;
  agent: Agent;
  title: string;
  project: string;
  cwd: string;
  updatedAt: number;
  sizeBytes: number;
  source?: string | null;
  wslDistro?: string | null;
  machineName: string;
  local: boolean;
  remote: boolean;
  cwdExists: boolean;
  mappedCwd?: string | null;
}

export interface ScanResult {
  sessions: SessionView[];
  warnings: string[];
}

export interface StatusView {
  local: number;
  remote: number;
  pendingPush: string[];
  pendingPull: string[];
  warnings: string[];
}

export interface PushResult {
  added: number;
  updated: number;
  unchanged: number;
  committed: boolean;
  conflicts: string[];
  warnings: string[];
}

export interface PullResult {
  imported: number;
  skipped: number;
  conflicts: string[];
  warnings: string[];
  notes: string[];
}

export interface RestoreResult {
  imported: boolean;
  nativeId: string;
  path: string;
  note?: string | null;
  resumeHint: string;
}

export interface DoctorResult {
  lines: string[];
}
