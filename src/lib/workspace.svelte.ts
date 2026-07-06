import { invoke } from "@tauri-apps/api/core";

// ===== 类型 =====
export interface ProjectItem {
  name: string;
  path: string;
  has_readme?: boolean;
}

export interface WorkspaceConfig {
  name: string;
  path: string;
}

export interface EditorSetting {
  name: string;
  command: string;
}

interface AppSettings {
  editor: EditorSetting;
  workspaces: WorkspaceConfig[];
  active_workspace: string | null;
}

/**
 * 全局工作空间状态管理
 *
 * 工作空间页面和聊天页面共享同一份项目数据，
 * 任一页面切换工作空间后，另一页面也会同步。
 */
class WorkspaceStore {
  projects = $state<ProjectItem[]>([]);
  workspaces = $state<WorkspaceConfig[]>([]);
  activePath = $state("");
  activeName = $state("");
  editor = $state<EditorSetting>({ name: "", command: "" });
  loading = $state(false);

  /** 从设置加载并自动扫描活动工作空间 */
  async loadFromSettings() {
    try {
      const settings = await invoke<AppSettings>("get_settings");
      this.workspaces = settings.workspaces ?? [];
      this.editor = settings.editor ?? { name: "", command: "" };

      if (settings.active_workspace) {
        this.activePath = settings.active_workspace;
        const ws = this.workspaces.find(
          (w) => w.path === settings.active_workspace,
        );
        this.activeName = ws?.name ?? "";
        await this.scanProjects(settings.active_workspace);
      }
    } catch (e) {
      console.error("加载工作空间失败:", e);
    }
  }

  /** 扫描指定路径下的项目 */
  async scanProjects(path: string) {
    this.loading = true;
    try {
      this.projects = await invoke<ProjectItem[]>("scan_workspace", { path });
    } catch (e) {
      console.error("扫描项目失败:", e);
      this.projects = [];
    } finally {
      this.loading = false;
    }
  }

  /** 切换活动工作空间 */
  async switchWorkspace(path: string) {
    await invoke("set_active_workspace", { path });
    this.activePath = path;
    const ws = this.workspaces.find((w) => w.path === path);
    this.activeName = ws?.name ?? "";
    await this.scanProjects(path);
  }

  /** 按名称查找项目 */
  findProjectByName(name: string): ProjectItem | undefined {
    return this.projects.find(
      (p) => p.name.toLowerCase() === name.toLowerCase(),
    );
  }
}

export const workspaceStore = new WorkspaceStore();
