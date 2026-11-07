import { useEffect, useMemo, useState } from "react";
import {
  AlertTriangle,
  ArrowDown,
  ArrowUp,
  Check,
  Copy,
  FolderGit2,
  GitMerge,
  HardDrive,
  Laptop,
  Loader2,
  Mountain,
  RefreshCw,
  Settings,
  Terminal,
  Wifi,
  WifiOff,
  X,
} from "lucide-react";
import {
  createMergedSession,
  generateMergeBrief,
  getConfig,
  initStore,
  isDesktop,
  listSessions,
  pullStore,
  pushStore,
  restoreSession,
  runDoctor,
  saveSettings,
  syncStatus,
  updateStore,
} from "./api";
import { open } from "@tauri-apps/plugin-dialog";
import { t } from "./i18n";
import type {
  Agent,
  ConfigView,
  MergeMaterialView,
  SessionView,
  StatusView,
  WslSourceView,
} from "./types";

type View = "sessions" | "settings" | "doctor";
type AgentFilter = "all" | Agent;

export default function App() {
  const [config, setConfig] = useState<ConfigView | null | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    getConfig()
      .then(setConfig)
      .catch((err) => {
        setError(String(err));
        setConfig(null);
      });
  }, []);

  if (config === undefined) return <Splash />;
  if (config === null) return <Welcome initialError={error} onDone={setConfig} />;
  return <Workspace config={config} onConfigChange={setConfig} />;
}

function Splash() {
  return (
    <div className="splash">
      <div className="brand-mark">
        <Mountain size={28} />
      </div>
      <div className="splash-title">Cairn</div>
      <Loader2 className="spin" size={18} />
    </div>
  );
}

function Welcome({
  initialError,
  onDone,
}: {
  initialError: string | null;
  onDone: (config: ConfigView) => void;
}) {
  const [storePath, setStorePath] = useState("~/Cairn");
  const [remote, setRemote] = useState("");
  const [machineName, setMachineName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(initialError);

  async function submit(e: { preventDefault: () => void }) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const config = await initStore(
        storePath.trim(),
        remote.trim() || null,
        machineName.trim() || null,
      );
      onDone(config);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="welcome">
      <div className="welcome-card">
        <div className="welcome-brand">
          <div className="brand-mark large">
            <Mountain size={30} />
          </div>
          <div>
            <h1>Cairn</h1>
            <p className="muted">你的 AI 编程会话，跟着你走。</p>
          </div>
        </div>

        <p className="welcome-copy">
          连接一个你自己的私有 Git 仓库。Cairn 只负责保存、同步和恢复会话，
          不托管数据，也不读取任何登录凭证。
        </p>

        <form onSubmit={submit} className="stack">
          <label>
            <span>仓库目录</span>
            <input
              value={storePath}
              onChange={(e) => setStorePath(e.target.value)}
              placeholder="~/Cairn"
              autoFocus
            />
            <small>默认 ~/Cairn，之后可在设置里更改</small>
          </label>

          <label>
            <span>Git 远程地址（可选）</span>
            <input
              value={remote}
              onChange={(e) => setRemote(e.target.value)}
              placeholder="git@github.com:you/cairn-store.git"
            />
            <small>建议使用私有仓库。留空则只保存在本机，不会同步到其他设备。</small>
          </label>

          <label>
            <span>机器名称（可选）</span>
            <input
              value={machineName}
              onChange={(e) => setMachineName(e.target.value)}
              placeholder="macbook / windows-box"
            />
          </label>

          {error && (
            <div className="alert error">
              <AlertTriangle size={15} />
              <span>{error}</span>
            </div>
          )}

          <button className="button primary wide" type="submit" disabled={busy}>
            {busy ? <Loader2 className="spin" size={16} /> : <Check size={16} />}
            {busy ? "正在连接…" : "开始使用"}
          </button>
        </form>

        {!isDesktop() && (
          <div className="hint">
            当前是浏览器预览模式，界面可用，但不会读写真实会话。
          </div>
        )}
      </div>
    </div>
  );
}

function Workspace({
  config,
  onConfigChange,
}: {
  config: ConfigView;
  onConfigChange: (config: ConfigView) => void;
}) {
  const [sessions, setSessions] = useState<SessionView[]>([]);
  const [warnings, setWarnings] = useState<string[]>([]);
  const [status, setStatus] = useState<StatusView | null>(null);
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [view, setView] = useState<View>("sessions");
  const [agentFilter, setAgentFilter] = useState<AgentFilter>("all");
  const [projectFilter, setProjectFilter] = useState("all");
  const [wslOnly, setWslOnly] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [toast, setToast] = useState<{ kind: "ok" | "error"; text: string } | null>(
    null,
  );
  const hasRemote = Boolean(config.remote);

  const refresh = async (showSpinner = true) => {
    if (showSpinner) setBusy("refresh");
    try {
      const [scan, nextStatus] = await Promise.all([listSessions(), syncStatus()]);
      setSessions(scan.sessions);
      setWarnings([...scan.warnings, ...nextStatus.warnings]);
      setStatus(nextStatus);
      setSelectedKey(
        (current) =>
          current ??
          (scan.sessions[0]
            ? `${scan.sessions[0].agent}-${scan.sessions[0].timelineId}`
            : null),
      );
    } catch (err) {
      setToast({ kind: "error", text: String(err) });
    } finally {
      if (showSpinner) setBusy(null);
    }
  };

  useEffect(() => {
    void refresh();
  }, []);

  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(() => setToast(null), 4200);
    return () => window.clearTimeout(timer);
  }, [toast]);

  const projects = useMemo(() => {
    const counts = new Map<string, number>();
    for (const session of sessions) {
      counts.set(session.project, (counts.get(session.project) ?? 0) + 1);
    }
    return Array.from(counts.entries()).sort((a, b) => b[1] - a[1]);
  }, [sessions]);

  const filtered = useMemo(() => {
    const query = search.trim().toLowerCase();
    return sessions.filter((session) => {
      if (agentFilter !== "all" && session.agent !== agentFilter) return false;
      if (projectFilter !== "all" && session.project !== projectFilter) return false;
      if (wslOnly && !session.wslDistro) return false;
      if (query) {
        const haystack =
          `${session.title} ${session.project} ${session.cwd} ${session.id}`.toLowerCase();
        if (!haystack.includes(query)) return false;
      }
      return true;
    });
  }, [sessions, agentFilter, projectFilter, wslOnly, search]);

  const selected =
    sessions.find(
      (session) => `${session.agent}-${session.timelineId}` === selectedKey,
    ) ?? null;

  async function sync(kind: "push" | "pull") {
    setBusy(kind);
    try {
      if (kind === "push") {
        const result = await pushStore();
        const conflictText = result.conflicts.length
          ? `，冲突 ${result.conflicts.length}`
          : "";
        setToast({
          kind: "ok",
          text: hasRemote
            ? `已推送：新增 ${result.added}，更新 ${result.updated}，未变 ${result.unchanged}${conflictText}`
            : t("workspace.pushedLocal"),
        });
      } else {
        const result = await pullStore();
        setToast({
          kind: "ok",
          text: `会话仓库已刷新：${result.remote} 条会话。恢复请按会话单独操作。`,
        });
      }
      await refresh(false);
    } catch (err) {
      setToast({ kind: "error", text: String(err) });
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          <div className="brand-mark">
            <Mountain size={18} />
          </div>
          <div className="brand-text">
            <strong>Cairn</strong>
            <span>云会话管理</span>
          </div>
        </div>

        <div className="topbar-meta">
          <span className="pill">
            <Laptop size={13} />
            {config.machineName}
          </span>
          <span className="pill muted-pill" title={config.storePath}>
            <HardDrive size={13} />
            {shortPath(config.storePath)}
          </span>
          {status && (
            <>
              <span className="pill">
                <Wifi size={13} />
                本地 {status.local}
              </span>
              <span className="pill">
                <WifiOff size={13} />
                云端 {status.remote}
              </span>
              {status.pendingPush.length > 0 && (
                <span className="pill warn">
                  <ArrowUp size={13} />
                  {status.pendingPush.length} 待推送
                </span>
              )}
              {status.pendingPull.length > 0 && (
                <span className="pill warn">
                  <ArrowDown size={13} />
                  {status.pendingPull.length} 待拉取
                </span>
              )}
            </>
          )}
        </div>

        <div className="topbar-actions">
          <button
            className="button ghost"
            onClick={() => refresh()}
            disabled={busy !== null}
            title="刷新"
          >
            <RefreshCw className={busy === "refresh" ? "spin" : ""} size={15} />
            刷新
          </button>
          <button
            className="button ghost"
            onClick={() => void sync("push")}
            disabled={busy !== null}
          >
            {busy === "push" ? <Loader2 className="spin" size={15} /> : <ArrowUp size={15} />}
            {hasRemote ? t("workspace.push") : t("workspace.pushLocal")}
          </button>
          <button
            className="button primary"
            onClick={() => void sync("pull")}
            disabled={busy !== null || !hasRemote}
            title={!hasRemote ? t("workspace.localOnly") : undefined}
          >
            {busy === "pull" ? <Loader2 className="spin" size={15} /> : <ArrowDown size={15} />}
            {t("workspace.pull")}
          </button>
          <button
            className={`button ghost ${view === "settings" ? "active" : ""}`}
            onClick={() => setView(view === "settings" ? "sessions" : "settings")}
            title="设置"
          >
            <Settings size={15} />
          </button>
        </div>
      </header>

      <div className="workspace">
        <aside className="sidebar">
          <button
            className={`nav-item ${agentFilter === "all" && !wslOnly ? "active" : ""}`}
            onClick={() => {
              setAgentFilter("all");
              setWslOnly(false);
              setProjectFilter("all");
            }}
          >
            <Mountain size={15} />
            <span>所有会话</span>
            <b>{sessions.length}</b>
          </button>

          <div className="nav-section">工具</div>
          <button
            className={`nav-item ${agentFilter === "codex" ? "active" : ""}`}
            onClick={() => setAgentFilter("codex")}
          >
            <Terminal size={15} />
            <span>Codex</span>
            <b>{sessions.filter((s) => s.agent === "codex").length}</b>
          </button>
          <button
            className={`nav-item ${agentFilter === "claude" ? "active" : ""}`}
            onClick={() => setAgentFilter("claude")}
          >
            <Terminal size={15} />
            <span>Claude Code</span>
            <b>{sessions.filter((s) => s.agent === "claude").length}</b>
          </button>

          <div className="nav-section">来源</div>
          <button
            className={`nav-item ${wslOnly ? "active" : ""}`}
            onClick={() => setWslOnly((value) => !value)}
          >
            <Laptop size={15} />
            <span>仅 WSL</span>
            <b>{sessions.filter((s) => s.wslDistro).length}</b>
          </button>

          <div className="nav-section">项目</div>
          <button
            className={`nav-item ${projectFilter === "all" ? "active" : ""}`}
            onClick={() => setProjectFilter("all")}
          >
            <FolderGit2 size={15} />
            <span>全部项目</span>
            <b>{projects.length}</b>
          </button>
          {projects.slice(0, 12).map(([project, count]) => (
            <button
              key={project}
              className={`nav-item ${projectFilter === project ? "active" : ""}`}
              onClick={() => setProjectFilter(project)}
              title={project}
            >
              <FolderGit2 size={15} />
              <span>{prettyProject(project)}</span>
              <b>{count}</b>
            </button>
          ))}

          <div className="sidebar-footer">
            <button className="nav-item" onClick={() => setView("doctor")}>
              <Wifi size={15} />
              <span>诊断</span>
            </button>
            {!isDesktop() && <div className="hint small">浏览器预览模式</div>}
          </div>
        </aside>

        {view === "settings" ? (
          <main className="settings-area">
            <SettingsPanel
              config={config}
              onConfigChange={onConfigChange}
              onToast={setToast}
            />
          </main>
        ) : view === "doctor" ? (
          <main className="settings-area">
            <DoctorPanel />
          </main>
        ) : (
          <>
            <main className="list-pane">
              <div className="pane-header">
                <div>
                  <h2>
                    {agentFilter === "all"
                      ? "所有会话"
                      : agentFilter === "codex"
                        ? "Codex 会话"
                        : "Claude Code 会话"}
                  </h2>
                  <p className="muted">
                    {filtered.length} 条
                    {projectFilter !== "all" && ` · ${prettyProject(projectFilter)}`}
                    {wslOnly && " · WSL"}
                  </p>
                </div>
                <input
                  className="search"
                  value={search}
                  onChange={(e) => setSearch(e.target.value)}
                  placeholder="搜索标题、项目、路径…"
                />
              </div>
              {!hasRemote && (
                <div className="alert warn">
                  <AlertTriangle size={15} />
                  <span>{t("workspace.localOnly")}</span>
                </div>
              )}
              {warnings.length > 0 && (
                <div className="alert warn">
                  <AlertTriangle size={15} />
                  <span>{warnings[0]}</span>
                </div>
              )}

              <div className="session-list">
                {filtered.map((session) => {
                  const key = `${session.agent}-${session.timelineId}`;
                  return (
                    <button
                      key={key}
                      className={`session-card ${selectedKey === key ? "selected" : ""}`}
                      onClick={() => setSelectedKey(key)}
                    >
                      <div className="session-row">
                        <span className={`agent-dot ${session.agent}`} />
                        <strong>{session.title}</strong>
                        {session.parentTimeline && (
                          <span className="badge fork">分叉</span>
                        )}
                        {session.mergedFrom?.length > 0 && (
                          <span className="badge merge">合并</span>
                        )}
                        {session.archived && (
                          <span className="badge archived">已归档</span>
                        )}
                        {!session.local && <span className="badge remote">云端</span>}
                        {session.wslDistro && (
                          <span className="badge wsl">WSL</span>
                        )}
                      </div>
                      <div className="session-sub">
                        <span>{prettyProject(session.project)}</span>
                        <span>{formatTime(session.updatedAt)}</span>
                        <span>{formatBytes(session.sizeBytes)}</span>
                      </div>
                    </button>
                  );
                })}
                {filtered.length === 0 && (
                  <div className="empty">
                    <Mountain size={28} />
                    <h3>没有会话</h3>
                    <p>换一个筛选条件，或先在本机运行 Codex / Claude Code。</p>
                  </div>
                )}
              </div>
            </main>

            <aside className="detail-pane">
              {selected ? (
                <SessionDetail
                  key={`${selected.agent}-${selected.timelineId}`}
                  session={selected}
                  timelines={sessions.filter(
                    (session) => session.familyId === selected.familyId,
                  )}
                  onSelect={setSelectedKey}
                  onRestored={() => void refresh(false)}
                  onToast={setToast}
                />
              ) : (
                <div className="empty detail-empty">
                  <Mountain size={30} />
                  <h3>选择一个会话</h3>
                  <p>查看详情、恢复会话，或复制继续命令。</p>
                </div>
              )}
            </aside>
          </>
        )}
      </div>

      {toast && (
        <div className={`toast ${toast.kind}`}>
          {toast.kind === "ok" ? <Check size={15} /> : <AlertTriangle size={15} />}
          <span>{toast.text}</span>
        </div>
      )}
    </div>
  );
}

function SessionDetail({
  session,
  timelines,
  onSelect,
  onRestored,
  onToast,
}: {
  session: SessionView;
  timelines: SessionView[];
  onSelect: (key: string) => void;
  onRestored: () => void;
  onToast: (toast: { kind: "ok" | "error"; text: string }) => void;
}) {
  const [restoreCwd, setRestoreCwd] = useState(session.mappedCwd || session.cwd);
  const [busy, setBusy] = useState(false);
  const [mergeOpen, setMergeOpen] = useState(false);
  const [result, setResult] = useState<{
    path: string;
    note?: string | null;
    hint: string;
  } | null>(null);
  const remoteTimelines = timelines.filter(
    (timeline) => timeline.remote && !timeline.archived,
  );
  const canMerge = remoteTimelines.length >= 2;

  async function restore() {
    setBusy(true);
    try {
      let target = restoreCwd.trim() || null;
      if (!session.cwdExists && !session.mappedCwd && !target) {
        const picked = await pickDirectory(session.cwd);
        if (!picked) {
          setBusy(false);
          return;
        }
        target = picked;
        setRestoreCwd(picked);
      }
      const outcome = await restoreSession(
        session.id,
        session.timelineId,
        session.agent,
        target,
        true,
      );
      setResult({
        path: outcome.path,
        note: outcome.note,
        hint: outcome.resumeHint,
      });
      onToast({
        kind: "ok",
        text: outcome.imported
          ? target
            ? "会话已恢复，路径映射已记住"
            : "会话已恢复"
          : "会话已是最新",
      });
      onRestored();
    } catch (err) {
      onToast({ kind: "error", text: String(err) });
    } finally {
      setBusy(false);
    }
  }

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      onToast({ kind: "ok", text: "已复制" });
    } catch {
      onToast({ kind: "error", text: "复制失败" });
    }
  }

  return (
    <div className="detail">
      <div className="detail-head">
        <span className={`agent-badge ${session.agent}`}>
          {session.agent === "codex" ? "Codex" : "Claude Code"}
        </span>
        {session.wslDistro && <span className="badge wsl">WSL · {session.wslDistro}</span>}
        {session.parentTimeline && <span className="badge fork">分叉</span>}
        {session.local ? (
          <span className="badge local">本机</span>
        ) : (
          <span className="badge remote">仅在云端</span>
        )}
        <h2>{session.title}</h2>
        <p className="muted mono">{session.id}</p>
      </div>

      {timelines.length > 1 && (
        <div className="timeline-box">
          <div className="restore-title">
            <h3>时间线</h3>
            <span className="muted">{timelines.length} 条</span>
          </div>
          {timelines
            .slice()
            .sort((a, b) => b.updatedAt - a.updatedAt)
            .map((timeline) => {
              const key = `${timeline.agent}-${timeline.timelineId}`;
              const active = timeline.timelineId === session.timelineId;
              return (
                <button
                  key={key}
                  className={`timeline-item ${active ? "active" : ""}`}
                  onClick={() => onSelect(key)}
                >
                  <span className="timeline-name">
                    {timeline.timelineId === timeline.familyId ? "主线" : "分叉"}
                  </span>
                  <span className="timeline-meta">
                    {timeline.machineName} · {formatTime(timeline.updatedAt)}
                  </span>
                  <span className="timeline-flags">
                    {timeline.local ? "本机" : "云端"}
                    {timeline.remote ? " · 已同步" : ""}
                    {timeline.archived ? " · 已归档" : ""}
                  </span>
                </button>
              );
            })}
          <button
            className="button primary wide merge-entry"
            onClick={() => setMergeOpen(true)}
            disabled={!canMerge}
            title={
              canMerge
                ? "选择两条时间线，生成合并简报并创建新会话"
                : "至少需要两条已推送到会话仓库的时间线"
            }
          >
            <GitMerge size={15} />
            合并时间线
          </button>
          {!canMerge && (
            <p className="muted restore-hint">
              合并前需要先推送两条时间线，并确保它们都没有归档。
            </p>
          )}
        </div>
      )}

      {mergeOpen && (
        <MergeModal
          session={session}
          timelines={timelines}
          onClose={() => setMergeOpen(false)}
          onCreated={onRestored}
          onToast={onToast}
        />
      )}

      <div className="meta-grid">
        <div>
          <span>项目</span>
          <strong title={session.project}>{prettyProject(session.project)}</strong>
        </div>
        <div>
          <span>机器</span>
          <strong>{session.machineName}</strong>
        </div>
        <div>
          <span>更新时间</span>
          <strong>{formatTime(session.updatedAt)}</strong>
        </div>
        <div>
          <span>大小</span>
          <strong>{formatBytes(session.sizeBytes)}</strong>
        </div>
      </div>

      <div className="field">
        <label>工作目录</label>
        <code>{session.cwd}</code>
      </div>

      <div className="restore-box">
        <div className="restore-title">
          <h3>恢复会话</h3>
          <span className="muted">写回本地，交给原生工具继续</span>
        </div>
        <label className="stack-tight">
          <span>恢复到目录</span>
          <input
            value={restoreCwd}
            onChange={(e) => setRestoreCwd(e.target.value)}
            placeholder={session.cwd}
          />
        </label>
        {!session.cwdExists && !session.mappedCwd && (
          <p className="muted restore-hint">
            原路径在本机不存在。点击恢复会让你选择本地项目目录，选完会自动记住映射。
          </p>
        )}
        <button className="button primary wide" onClick={() => void restore()} disabled={busy}>
          {busy ? <Loader2 className="spin" size={15} /> : <FolderGit2 size={15} />}
          {busy
            ? "正在恢复…"
            : !session.cwdExists && !session.mappedCwd
              ? "选择目录并恢复"
              : "恢复会话"}
        </button>

        {result && (
          <div className="restore-result">
            <div className="result-line">
              <Check size={14} />
              <span className="mono">{result.path}</span>
            </div>
            {result.note && <p className="muted">{result.note}</p>}
            <div className="command">
              <code>{result.hint}</code>
              <button className="icon-button" onClick={() => void copy(result.hint)} title="复制命令">
                <Copy size={14} />
              </button>
            </div>
          </div>
        )}
      </div>

      <div className="note">
        Cairn 不会自动启动 agent。恢复后打开 Codex / Claude Code，在对应入口继续这条会话。
      </div>

      {session.parentTimeline && (
        <div className="note warn-note">
          这是从主线分叉出的时间线。恢复时会先备份同名本地会话，再写入当前时间线。
        </div>
      )}

      <div className="detail-footer">
        <span className="muted">来源</span>
        <span>{session.source || "unknown"}</span>
      </div>
    </div>
  );
}

function MergeModal({
  session,
  timelines,
  onClose,
  onCreated,
  onToast,
}: {
  session: SessionView;
  timelines: SessionView[];
  onClose: () => void;
  onCreated: () => void;
  onToast: (toast: { kind: "ok" | "error"; text: string }) => void;
}) {
  const candidates = timelines.filter(
    (timeline) => timeline.remote && !timeline.archived,
  );
  const first = candidates.find(
    (timeline) => timeline.timelineId === session.timelineId,
  ) ?? candidates[0];
  const second = candidates.find(
    (timeline) => timeline.timelineId !== first?.timelineId,
  );
  const [a, setA] = useState(first?.timelineId ?? "");
  const [b, setB] = useState(second?.timelineId ?? "");
  const [phase, setPhase] = useState<"idle" | "generating" | "review" | "creating">(
    "idle",
  );
  const [brief, setBrief] = useState("");
  const [material, setMaterial] = useState<MergeMaterialView | null>(null);
  const [command, setCommand] = useState("");
  const [target, setTarget] = useState(session.mappedCwd || session.cwd);
  const [error, setError] = useState<string | null>(null);
  const busy = phase === "generating" || phase === "creating";

  async function generate() {
    if (!a || !b || a === b) {
      setError("请选择两条不同的时间线");
      return;
    }
    setPhase("generating");
    setError(null);
    try {
      const result = await generateMergeBrief(session.agent, a, b);
      setBrief(result.brief);
      setMaterial(result.material);
      setCommand(result.command);
      setPhase("review");
    } catch (err) {
      setError(String(err));
      setPhase("idle");
    }
  }

  async function pickTarget() {
    const picked = await pickDirectory(target || session.cwd);
    if (picked) setTarget(picked);
  }

  async function create() {
    if (!brief.trim()) {
      setError("合并简报为空");
      return;
    }
    if (!target.trim()) {
      setError("请选择合并会话的工作目录");
      return;
    }
    setPhase("creating");
    setError(null);
    try {
      const created = await createMergedSession(
        session.agent,
        a,
        b,
        brief,
        target.trim(),
      );
      onToast({
        kind: "ok",
        text: `已创建合并会话：${created.title}。旧时间线已归档。`,
      });
      onCreated();
      onClose();
    } catch (err) {
      setError(String(err));
      setPhase("review");
    }
  }

  return (
    <div
      className="modal-overlay"
      onClick={() => {
        if (!busy) onClose();
      }}
    >
      <div className="modal merge-modal" onClick={(event) => event.stopPropagation()}>
        <div className="modal-head">
          <div>
            <h2>合并时间线</h2>
            <p className="muted">
              使用这条会话自己的 agent（
              {session.agent === "codex" ? "Codex" : "Claude Code"}）生成合并简报。
            </p>
          </div>
          <button
            className="icon-button"
            onClick={onClose}
            disabled={busy}
            title="关闭"
          >
            <X size={16} />
          </button>
        </div>

        <div className="merge-pickers">
          <label className="stack-tight">
            <span>时间线 A</span>
            <select
              value={a}
              onChange={(event) => {
                setA(event.target.value);
                setPhase("idle");
                setBrief("");
                setMaterial(null);
                setCommand("");
                setError(null);
              }}
              disabled={busy}
            >
              {candidates.map((timeline) => (
                <option key={timeline.timelineId} value={timeline.timelineId}>
                  {timeline.timelineId === timeline.familyId ? "主线" : "分叉"} ·{" "}
                  {timeline.machineName} · {formatTime(timeline.updatedAt)}
                </option>
              ))}
            </select>
          </label>
          <label className="stack-tight">
            <span>时间线 B</span>
            <select
              value={b}
              onChange={(event) => {
                setB(event.target.value);
                setPhase("idle");
                setBrief("");
                setMaterial(null);
                setCommand("");
                setError(null);
              }}
              disabled={busy}
            >
              <option value="">选择另一条时间线</option>
              {candidates
                .filter((timeline) => timeline.timelineId !== a)
                .map((timeline) => (
                  <option key={timeline.timelineId} value={timeline.timelineId}>
                    {timeline.timelineId === timeline.familyId ? "主线" : "分叉"} ·{" "}
                    {timeline.machineName} · {formatTime(timeline.updatedAt)}
                  </option>
                ))}
            </select>
          </label>
        </div>

        {phase === "idle" && (
          <div className="merge-explain">
            <p>
              第一步：Cairn 提取两条时间线分叉后的消息、文件改动、命令和错误，生成材料包。
              agent 只读取材料包并输出 Markdown，不会修改你的项目或会话。
            </p>
            <p className="muted">
              调用 {session.agent === "codex" ? "codex exec" : "claude --print"} 会消耗你
              自己的 agent 额度。材料包可能包含代码片段，请确认后再继续。
            </p>
          </div>
        )}

        {phase === "generating" && (
          <div className="merge-progress">
            <Loader2 className="spin" size={18} />
            <span>
              正在调用 {session.agent === "codex" ? "Codex" : "Claude Code"} 生成简报…
            </span>
          </div>
        )}

        {(phase === "review" || phase === "creating") && (
          <>
            <div className="merge-progress-line">
              <Check size={14} />
              <span>
                已生成简报（{material?.messageCountA ?? 0} +{" "}
                {material?.messageCountB ?? 0} 条分叉后对话）
              </span>
              <button
                className="button ghost"
                onClick={() => void generate()}
                disabled={busy}
              >
                <RefreshCw size={14} />
                重新生成
              </button>
            </div>

            {material && material.warnings.length > 0 && (
              <div className="alert warn">
                <AlertTriangle size={15} />
                <span>{material.warnings.join("；")}</span>
              </div>
            )}

            <label className="stack-tight">
              <span>合并简报（可编辑）</span>
              <textarea
                className="merge-brief"
                value={brief}
                onChange={(event) => setBrief(event.target.value)}
                disabled={phase === "creating"}
              />
            </label>

            {material && (
              <details className="merge-material">
                <summary>查看原始材料包</summary>
                <pre>{material.material}</pre>
              </details>
            )}

            <div className="merge-target">
              <label className="stack-tight">
                <span>新会话工作目录</span>
                <input
                  value={target}
                  onChange={(event) => setTarget(event.target.value)}
                  disabled={phase === "creating"}
                />
              </label>
              <button
                className="button ghost"
                onClick={() => void pickTarget()}
                disabled={busy}
              >
                <FolderGit2 size={15} />
                选择目录
              </button>
            </div>

            <p className="muted merge-note">
              创建后会生成一个新的原生会话，简报作为第一条消息。原来的两条时间线会归档，
              但不会删除；代码冲突仍需在你自己的项目里用 Git 处理。
            </p>

            <button
              className="button primary wide"
              onClick={() => void create()}
              disabled={busy}
            >
              {phase === "creating" ? (
                <Loader2 className="spin" size={15} />
              ) : (
                <GitMerge size={15} />
              )}
              {phase === "creating" ? "正在创建合并会话…" : "创建合并会话"}
            </button>
          </>
        )}

        {command && <p className="muted mono merge-command">{command}</p>}

        {error && (
          <div className="alert error">
            <AlertTriangle size={15} />
            <span>{error}</span>
          </div>
        )}

        {phase === "idle" && (
          <div className="modal-actions">
            <button className="button ghost" onClick={onClose}>
              取消
            </button>
            <button
              className="button primary"
              onClick={() => void generate()}
              disabled={!a || !b || a === b}
            >
              <GitMerge size={15} />
              生成合并简报
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

function SettingsPanel({
  config,
  onConfigChange,
  onToast,
}: {
  config: ConfigView;
  onConfigChange: (config: ConfigView) => void;
  onToast: (toast: { kind: "ok" | "error"; text: string }) => void;
}) {
  const [saveMode, setSaveMode] = useState(config.saveMode);
  const [agents, setAgents] = useState(config.agents);
  const [wsl, setWsl] = useState<WslSourceView[]>(config.wsl);
  const [busy, setBusy] = useState(false);
  const [editingStore, setEditingStore] = useState(false);
  const [storePath, setStorePath] = useState(config.storePath);
  const [remote, setRemote] = useState(config.remote || "");
  const [machineName, setMachineName] = useState(config.machineName);

  function updateWsl(index: number, patch: Partial<WslSourceView>) {
    setWsl((current) =>
      current.map((item, i) => (i === index ? { ...item, ...patch } : item)),
    );
  }

  async function save() {
    setBusy(true);
    try {
      const next = await saveSettings(saveMode, agents, wsl);
      onConfigChange(next);
      onToast({ kind: "ok", text: t("settings.saved") });
    } catch (err) {
      onToast({ kind: "error", text: String(err) });
    } finally {
      setBusy(false);
    }
  }

  function startEditStore() {
    setStorePath(config.storePath);
    setRemote(config.remote || "");
    setMachineName(config.machineName);
    setEditingStore(true);
  }

  async function saveStore() {
    setBusy(true);
    try {
      const next = await updateStore(
        storePath.trim(),
        remote.trim() || null,
        machineName.trim() || null,
      );
      onConfigChange(next);
      setEditingStore(false);
      onToast({ kind: "ok", text: t("settings.storeUpdated") });
    } catch (err) {
      onToast({ kind: "error", text: String(err) });
    } finally {
      setBusy(false);
    }
  }

  const modeCopy = {
    index: {
      label: t("settings.modeIndex"),
      hint: t("settings.modeIndexHint"),
    },
    standard: {
      label: t("settings.modeStandard"),
      hint: t("settings.modeStandardHint"),
    },
    full: {
      label: t("settings.modeFull"),
      hint: t("settings.modeFullHint"),
    },
  };

  return (
    <div className="settings">
      <div className="pane-header">
        <div>
          <h2>{t("settings.title")}</h2>
          <p className="muted">{t("settings.subtitle")}</p>
        </div>
        <button className="button primary" onClick={() => void save()} disabled={busy}>
          {busy ? <Loader2 className="spin" size={15} /> : <Check size={15} />}
          {t("settings.save")}
        </button>
      </div>

      <section className="settings-card">
        <div className="settings-head">
          <div>
            <h3>{t("settings.repo")}</h3>
            <p className="muted">{t("settings.repoHint")}</p>
          </div>
          {!editingStore && (
            <button className="button ghost" onClick={startEditStore}>
              {t("settings.changeRepo")}
            </button>
          )}
        </div>

        {editingStore ? (
          <div className="store-editor">
            <label className="stack-tight">
              <span>{t("settings.localDir")}</span>
              <input
                value={storePath}
                onChange={(e) => setStorePath(e.target.value)}
                placeholder="~/Cairn"
              />
            </label>
            <label className="stack-tight">
              <span>{t("settings.remote")}</span>
              <input
                value={remote}
                onChange={(e) => setRemote(e.target.value)}
                placeholder="git@github.com:you/cairn-store.git"
              />
            </label>
            <label className="stack-tight">
              <span>{t("settings.machine")}</span>
              <input
                value={machineName}
                onChange={(e) => setMachineName(e.target.value)}
              />
            </label>
            <div className="editor-actions">
              <button className="button primary" onClick={() => void saveStore()} disabled={busy}>
                {busy ? <Loader2 className="spin" size={15} /> : <Check size={15} />}
                {t("settings.saveChanges")}
              </button>
              <button
                className="button ghost"
                onClick={() => setEditingStore(false)}
                disabled={busy}
              >
                {t("settings.cancel")}
              </button>
            </div>
          </div>
        ) : (
          <>
            <div className="kv">
              <span>{t("settings.localDir")}</span>
              <code>{config.storePath}</code>
            </div>
            <div className="kv">
              <span>{t("settings.remote")}</span>
              <code>{config.remote || t("settings.localOnly")}</code>
            </div>
            <div className="kv">
              <span>{t("settings.machine")}</span>
              <code>{config.machineName}</code>
            </div>
            <div className="kv">
              <span>{t("settings.configFile")}</span>
              <code>{config.configPath}</code>
            </div>
            <div className="kv">
              <span>{t("settings.storeManifest")}</span>
              <code>{config.storeManifest}</code>
            </div>
          </>
        )}
      </section>

      <section className="settings-card">
        <h3>{t("settings.saveMode")}</h3>
        <div className="segmented">
          {(["index", "standard", "full"] as const).map((mode) => (
            <button
              key={mode}
              className={saveMode === mode ? "active" : ""}
              onClick={() => setSaveMode(mode)}
            >
              <strong>{modeCopy[mode].label}</strong>
              <span>{modeCopy[mode].hint}</span>
            </button>
          ))}
        </div>
      </section>

      <section className="settings-card">
        <h3>{t("settings.tools")}</h3>
        <label className="toggle">
          <input
            type="checkbox"
            checked={agents.codex}
            onChange={(e) => setAgents({ ...agents, codex: e.target.checked })}
          />
          <span>{t("settings.codex")}</span>
        </label>
        <label className="toggle">
          <input
            type="checkbox"
            checked={agents.claude}
            onChange={(e) => setAgents({ ...agents, claude: e.target.checked })}
          />
          <span>{t("settings.claude")}</span>
        </label>
      </section>

      <section className="settings-card">
        <div className="settings-head">
          <div>
            <h3>{t("settings.wsl")}</h3>
            <p className="muted">{t("settings.wslHint")}</p>
          </div>
          <button
            className="button ghost"
            onClick={() => setWsl([...wsl, { distro: "", codexHome: "", claudeHome: "" }])}
          >
            + {t("settings.add")}
          </button>
        </div>

        {wsl.length === 0 && <p className="muted">{t("settings.wslEmpty")}</p>}
        {wsl.map((source, index) => (
          <div className="wsl-row" key={index}>
            <input
              value={source.distro}
              onChange={(e) => updateWsl(index, { distro: e.target.value })}
              placeholder={t("settings.distro")}
            />
            <input
              value={source.codexHome || ""}
              onChange={(e) => updateWsl(index, { codexHome: e.target.value })}
              placeholder={t("settings.wslCodexPlaceholder")}
            />
            <input
              value={source.claudeHome || ""}
              onChange={(e) => updateWsl(index, { claudeHome: e.target.value })}
              placeholder={t("settings.wslClaudePlaceholder")}
            />
            <button
              className="icon-button danger"
              onClick={() => setWsl(wsl.filter((_, i) => i !== index))}
              title={t("settings.remove")}
            >
              ×
            </button>
          </div>
        ))}
      </section>
    </div>
  );
}

function DoctorPanel() {
  const [lines, setLines] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);

  async function load() {
    setBusy(true);
    try {
      const result = await runDoctor();
      setLines(result.lines);
    } catch (err) {
      setLines([String(err)]);
    } finally {
      setBusy(false);
    }
  }

  useEffect(() => {
    void load();
  }, []);

  return (
    <div className="settings">
      <div className="pane-header">
        <div>
          <h2>诊断</h2>
          <p className="muted">本地配置、仓库和工具状态</p>
        </div>
        <button className="button ghost" onClick={() => void load()} disabled={busy}>
          <RefreshCw className={busy ? "spin" : ""} size={15} />
          刷新
        </button>
      </div>
      <section className="settings-card">
        <pre className="doctor">
          {lines.length ? lines.join("\n") : "正在检查…"}
        </pre>
      </section>
    </div>
  );
}

async function pickDirectory(defaultPath?: string | null): Promise<string | null> {
  if (isDesktop()) {
    const selected = await open({
      directory: true,
      multiple: false,
      defaultPath: defaultPath || undefined,
    });
    if (Array.isArray(selected)) return selected[0] ?? null;
    return selected ?? null;
  }
  const picked = window.prompt("选择本地项目目录", defaultPath || "");
  return picked && picked.trim() ? picked.trim() : null;
}

function formatTime(ts: number): string {
  const seconds = Math.floor(Date.now() / 1000) - ts;
  if (seconds < 60) return "刚刚";
  if (seconds < 3600) return `${Math.floor(seconds / 60)} 分钟前`;
  if (seconds < 86_400) return `${Math.floor(seconds / 3600)} 小时前`;
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(ts * 1000));
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let index = 0;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index += 1;
  }
  return `${value.toFixed(1)} ${units[index]}`;
}

function shortPath(path: string): string {
  if (path.length <= 34) return path;
  return `…${path.slice(-33)}`;
}

function prettyProject(project: string): string {
  if (project.startsWith("git:")) {
    const parts = project.slice(4).split("/");
    return parts.slice(-2).join("/");
  }
  if (project.startsWith("local:")) {
    return project.slice(6).replace(/-[0-9a-f]{8}$/, "");
  }
  return project;
}
