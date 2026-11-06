export type Locale = "zh-CN" | "en-US";

const messages: Record<Locale, Record<string, string>> = {
  "zh-CN": {
    "settings.title": "设置",
    "settings.subtitle": "保存模式、工具来源和 WSL 配置",
    "settings.save": "保存",
    "settings.saved": "设置已保存",
    "settings.repo": "云端仓库",
    "settings.repoHint": "存放所有会话的 Git 仓库；未配置远程地址时只保存在本机 ~/Cairn。",
    "settings.localDir": "本地目录",
    "settings.remote": "远程地址",
    "settings.machine": "机器",
    "settings.unconfigured": "未配置",
    "workspace.localOnly": "未配置云端仓库，推送只会保存到本机。",
    "workspace.push": "推送",
    "workspace.pushLocal": "保存到本地",
    "workspace.pull": "拉取",
    "workspace.pushedLocal": "已保存到本地（未配置云端仓库）",
    "settings.localOnly": "未配置（仅本地）",
    "settings.configFile": "本机配置",
    "settings.storeManifest": "仓库清单",
    "settings.changeRepo": "更换仓库",
    "settings.storeUpdated": "仓库配置已更新",
    "settings.saveChanges": "保存更改",
    "settings.cancel": "取消",
    "settings.saveMode": "保存模式",
    "settings.modeIndex": "索引",
    "settings.modeStandard": "标准",
    "settings.modeFull": "完整",
    "settings.modeIndexHint": "只存索引，不能恢复",
    "settings.modeStandardHint": "可恢复，裁剪大块内容",
    "settings.modeFullHint": "完整原样保存",
    "settings.tools": "工具来源",
    "settings.codex": "Codex",
    "settings.claude": "Claude Code",
    "settings.wsl": "WSL 来源",
    "settings.wslHint": "文件级支持：通过 Windows 网络路径读写，不修改 WSL 内的数据库文件。",
    "settings.wslEmpty": "还没有配置 WSL 发行版。",
    "settings.distro": "发行版",
    "settings.codexHome": "Codex 目录",
    "settings.claudeHome": "Claude 目录",
    "settings.add": "添加",
    "settings.remove": "删除",
    "settings.wslCodexPlaceholder": "/home/you/.codex",
    "settings.wslClaudePlaceholder": "/home/you/.claude",
  },
  "en-US": {
    "settings.title": "Settings",
    "settings.subtitle": "Save mode, agent sources, and WSL configuration",
    "settings.save": "Save",
    "settings.saved": "Settings saved",
    "settings.repo": "Cloud repository",
    "settings.repoHint": "Git repository that stores all sessions. Without a remote, data stays on this machine under ~/Cairn.",
    "settings.localDir": "Local directory",
    "settings.remote": "Remote",
    "settings.machine": "Machine",
    "settings.unconfigured": "Not configured",
    "settings.localOnly": "Not configured (local only)",
    "workspace.localOnly": "No cloud repository configured. Push only saves locally.",
    "workspace.push": "Push",
    "workspace.pushLocal": "Save locally",
    "workspace.pull": "Pull",
    "workspace.pushedLocal": "Saved locally (no cloud repository configured)",
    "settings.configFile": "Machine config",
    "settings.storeManifest": "Store manifest",
    "settings.changeRepo": "Change repository",
    "settings.storeUpdated": "Repository configuration updated",
    "settings.saveChanges": "Save changes",
    "settings.cancel": "Cancel",
    "settings.saveMode": "Save mode",
    "settings.modeIndex": "Index",
    "settings.modeStandard": "Standard",
    "settings.modeFull": "Full",
    "settings.modeIndexHint": "Metadata only; not resumable",
    "settings.modeStandardHint": "Resumable; trims large blocks",
    "settings.modeFullHint": "Byte-for-byte copy",
    "settings.tools": "Agent sources",
    "settings.codex": "Codex",
    "settings.claude": "Claude Code",
    "settings.wsl": "WSL sources",
    "settings.wslHint": "File-level access through Windows network paths. WSL databases are never modified.",
    "settings.wslEmpty": "No WSL distribution configured yet.",
    "settings.distro": "Distribution",
    "settings.codexHome": "Codex home",
    "settings.claudeHome": "Claude home",
    "settings.add": "Add",
    "settings.remove": "Remove",
    "settings.wslCodexPlaceholder": "/home/you/.codex",
    "settings.wslClaudePlaceholder": "/home/you/.claude",
  },
};

let locale: Locale = "zh-CN";

export function setLocale(next: Locale) {
  locale = next;
  if (typeof localStorage !== "undefined") {
    localStorage.setItem("cairn.locale", next);
  }
}

export function getLocale(): Locale {
  return locale;
}

export function t(key: string, vars?: Record<string, string | number>): string {
  let value = messages[locale][key] ?? messages["zh-CN"][key] ?? key;
  if (vars) {
    for (const [name, replacement] of Object.entries(vars)) {
      value = value.replaceAll(`{${name}}`, String(replacement));
    }
  }
  return value;
}
