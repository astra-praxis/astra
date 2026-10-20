/**
 * 设置分类容器：标题 + 描述 + 内容块
 */
import type { ReactNode } from 'react';
import { SECTION_ICONS } from '../settings-icons';

interface Props {
  /** 对应 settings-icons.ts SECTION_ICONS 的 key */
  iconId: string;
  title: string;
  desc: string;
  children: ReactNode;
}

export default function SettingsSection({ iconId, title, desc, children }: Props) {
  const Icon = SECTION_ICONS[iconId];
  return (
    <section className="settings-section">
      <header className="section-head">
        <span className="section-glyph">{Icon && <Icon size={18} strokeWidth={1.8} />}</span>
        <div className="section-heading">
          <h2>{title}</h2>
          <p>{desc}</p>
        </div>
      </header>
      <div className="section-body">{children}</div>
    </section>
  );
}
