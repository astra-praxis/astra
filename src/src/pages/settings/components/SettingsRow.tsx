/**
 * 设置项统一行布局：左侧名称与说明，右侧控件。
 * stacked 模式把控件放到说明下方（适合主题网格等宽控件）。
 */
import type { ReactNode } from 'react';

interface Props {
  title: string;
  desc?: string;
  children: ReactNode;
  stacked?: boolean;
}

export default function SettingsRow({ title, desc, children, stacked }: Props) {
  return (
    <div className={`settings-row${stacked ? ' stacked' : ''}`}>
      <div className="row-text">
        <span className="row-title">{title}</span>
        {desc && <span className="row-desc">{desc}</span>}
      </div>
      <div className="row-control">{children}</div>
    </div>
  );
}
