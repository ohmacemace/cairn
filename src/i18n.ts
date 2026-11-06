export type Locale = "zh-CN" | "en-US";

const messages: Record<Locale, Record<string, string>> = {
  "zh-CN": {
    "settings.title": "设置",
    "settings.subtitle": "保存模式、工具来源和 WSL 配置",
    "settings.save": "保存",
    "settings.saved": "设置已保存",
    "settings.repo": "仓库",
    "settings.localDir": "本地目录",
    "settings.remote": "远程地址",
    "settings.machine": "机器",
    "settings.unconfigured": "未配置",
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
    "settings.paths": "路径映射",
    "settings.pathsHint": "添加本地项目目录，Cairn 会自动识别项目。恢复时也会自动记住你选择的目录。",
    "settings.pathsEmpty": "还没有路径映射。",
    "settings.addProject": "添加本地项目",
    "settings.pathsAdded": "路径映射已添加",
    "settings.mappingKey": "项目标识",
    "settings.mappingPath": "本机路径",
    "settings.repositories": "本地仓库",
    "settings.repositoriesHint": "添加已经 clone 到本机的项目目录，Cairn 会自动识别 Git 远程地址。",
    "settings.repositoriesEmpty": "还没有添加本地仓库。",
    "settings.addRepository": "添加仓库",
    "settings.repositoryAdded": "本地仓库已添加",
    "settings.repositoryRemoved": "本地仓库已移除",
    "settings.repositoryRemote": "远程",
    "settings.repositoryPath": "本地目录",
    "settings.add": "添加",
    "settings.remove": "删除",
    "settings.wslCodexPlaceholder": "/home/you/.codex",
    "settings.wslClaudePlaceholder": "/home/you/.claude",
    "settings.mappingKeyPlaceholder": "git:github.com/you/repo",
    "settings.mappingPathPlaceholder": "~/code/repo",
  },
  "en-US": {
    "settings.title": "Settings",
    "settings.subtitle": "Save mode, agent sources, and WSL configuration",
    "settings.save": "Save",
    "settings.saved": "Settings saved",
    "settings.repo": "Repository",
    "settings.localDir": "Local directory",
    "settings.remote": "Remote",
    "settings.machine": "Machine",
    "settings.unconfigured": "Not configured",
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
    "settings.paths": "Path mappings",
    "settings.pathsHint": "Add a local project folder. Cairn detects the project automatically and remembers folders chosen during restore.",
    "settings.pathsEmpty": "No path mappings yet.",
    "settings.addProject": "Add local project",
    "settings.pathsAdded": "Path mapping added",
    "settings.mappingKey": "Project key",
    "settings.mappingPath": "Local path",
    "settings.repositories": "Local repositories",
    "settings.repositoriesHint": "Add a project folder already cloned on this machine. Cairn detects the Git remote automatically.",
    "settings.repositoriesEmpty": "No local repositories added yet.",
    "settings.addRepository": "Add repository",
    "settings.repositoryAdded": "Local repository added",
    "settings.repositoryRemoved": "Local repository removed",
    "settings.repositoryRemote": "Remote",
    "settings.repositoryPath": "Local path",
    "settings.add": "Add",
    "settings.remove": "Remove",
    "settings.wslCodexPlaceholder": "/home/you/.codex",
    "settings.wslClaudePlaceholder": "/home/you/.claude",
    "settings.mappingKeyPlaceholder": "git:github.com/you/repo",
    "settings.mappingPathPlaceholder": "~/code/repo",
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
