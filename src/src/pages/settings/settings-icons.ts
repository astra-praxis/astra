/**
 * 设置中心图标映射（lucide）：侧边栏导航与分区标题共用，
 * 独立成文件避免 sections/index.ts 与分区组件循环依赖。
 */
import {
  Bot,
  Brain,
  Cpu,
  FolderOpen,
  GraduationCap,
  ImagePlus,
  Plug,
  Search,
  Settings,
  Sparkles,
  SquarePen,
  Wrench,
  type LucideIcon,
} from 'lucide-react';

/** 设置分类图标，key 对应 sections/index.ts 的 section id */
export const SECTION_ICONS: Record<string, LucideIcon> = {
  general: Settings,
  agent: Bot,
  memory: Brain,
  search: Search,
  'image-gen': ImagePlus,
  editor: SquarePen,
  workspace: FolderOpen,
};

/** 管理与资源入口图标，key 对应 RESOURCE_LINKS 的 to */
export const RESOURCE_ICONS: Record<string, LucideIcon> = {
  '/models': Cpu,
  '/providers': Plug,
  '/skills': GraduationCap,
  '/my-skills': Sparkles,
  '/tools': Wrench,
};
