/**
 * 设置分类容器：标题 + 描述 + 内容块
 */
import type { ReactNode } from 'react';

interface Props {
  icon: string;
  title: string;
  desc: string;
  children: ReactNode;
}

export default function SettingsSection({ icon, title, desc, children }: Props) {
  return (
    <section className="settings-section">
      <header className="section-head">
        <span className="section-glyph">{icon}</span>
        <div className="section-heading">
          <h2>{title}</h2>
          <p>{desc}</p>
        </div>
      </header>
      <div className="section-body">{children}</div>
    </section>
  );
}
