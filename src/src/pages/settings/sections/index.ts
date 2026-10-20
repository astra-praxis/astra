/**
 * 分类注册表：导航与路由都从这里取定义
 */
import type { ComponentType } from 'react';
import GeneralSettings from './GeneralSettings';
import AgentSettings from './AgentSettings';
import MemorySettings from './MemorySettings';
import SearchSettings from './SearchSettings';
import ImageGenerationSettings from './ImageGenerationSettings';
import DevelopmentSettings from './DevelopmentSettings';
import WorkspaceSettings from './WorkspaceSettings';

export interface SettingsSectionDef {
  id: string;
  label: string;
  icon: string;
  /** 分类内容区顶部的描述文案 */
  desc: string;
  component: ComponentType;
}

export const PREFERENCE_SECTIONS: SettingsSectionDef[] = [
  {
    id: 'general',
    label: '常规',
    icon: '⚙️',
    desc: '调整外观和项目扫描的默认行为',
    component: GeneralSettings,
  },
  {
    id: 'agent',
    label: 'Agent 行为',
    icon: '🤖',
    desc: '控制 Agent 执行任务时的确认策略与能力边界',
    component: AgentSettings,
  },
  {
    id: 'memory',
    label: '记忆',
    icon: '🧠',
    desc: '会话记忆的总结方式与模型来源',
    component: MemorySettings,
  },
  {
    id: 'search',
    label: '搜索',
    icon: '🔍',
    desc: 'Agent 联网搜索（search_web）使用的后端',
    component: SearchSettings,
  },
  {
    id: 'image-gen',
    label: '图片生成',
    icon: '🖼️',
    desc: 'generate_image 工具的网关与模型配置',
    component: ImageGenerationSettings,
  },
];

export const DEV_SECTIONS: SettingsSectionDef[] = [
  {
    id: 'editor',
    label: '默认编辑器',
    icon: '⚡',
    desc: '点击工作空间的项目卡片时，使用此编辑器打开项目目录',
    component: DevelopmentSettings,
  },
  {
    id: 'workspace',
    label: '工作空间',
    icon: '📁',
    desc: '添加多个工作空间目录，在主页可一键切换',
    component: WorkspaceSettings,
  },
];

export const ALL_SECTIONS: SettingsSectionDef[] = [...PREFERENCE_SECTIONS, ...DEV_SECTIONS];

/** 管理与资源：独立管理页面的跨页入口，不是设置项 */
export const RESOURCE_LINKS: { icon: string; label: string; to: string }[] = [
  { icon: '🤖', label: '模型管理', to: '/models' },
  { icon: '🔌', label: 'API 接口', to: '/providers' },
  { icon: '🧩', label: 'Skills 管理', to: '/skills' },
  { icon: '⚡', label: '我的 Skills', to: '/my-skills' },
  { icon: '🧰', label: '工具箱', to: '/tools' },
  { icon: '📦', label: '产物库', to: '/artifacts' },
  { icon: '📜', label: '日志', to: '/logs' },
];
