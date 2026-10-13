/** 工作空间首页（项目列表/详情）共享类型 */
import type { ProjectItem } from '@/stores/workspace';

export interface SubProject {
  name: string;
  path: string;
}

/** scan_workspace 返回的项目卡片（store 中的 ProjectItem 为其窄化子集） */
export interface ProjectCard extends ProjectItem {
  has_readme: boolean;
  sub_projects: SubProject[];
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
