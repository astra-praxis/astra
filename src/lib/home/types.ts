/** 工作空间首页（项目列表/详情）共享类型 */

export interface SubProject {
    name: string;
    path: string;
}

export interface ProjectCard {
    name: string;
    path: string;
    has_readme: boolean;
    sub_projects: SubProject[];
}

export interface EditorSetting {
    name: string;
    command: string;
}

export interface WorkspaceConfig {
    name: string;
    path: string;
}

export interface AppSettings {
    editor: EditorSetting;
    workspaces: WorkspaceConfig[];
    active_workspace: string | null;
}

export interface GitRepo {
    name: string;
    path: string;
    remote_url: string | null;
}

export interface SubDetail {
    name: string;
    path: string;
    git_repo: GitRepo | null;
    children: SubDetail[];
    readme_preview: string;
    depth: number;
}

export interface ProjectDetail {
    name: string;
    path: string;
    has_readme: boolean;
    readme_preview: string;
    sub_items: SubDetail[];
}
