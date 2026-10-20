/**
 * 设置中心左侧导航：偏好设置 / 开发环境（站内分类）+ 管理与资源（跨页入口）
 */
import { Link } from 'react-router';
import { DEV_SECTIONS, PREFERENCE_SECTIONS, RESOURCE_LINKS } from '../sections';

interface Props {
  active: string;
}

export default function SettingsSidebar({ active }: Props) {
  return (
    <nav className="settings-nav">
      <div className="nav-group">
        <div className="nav-group-label">偏好设置</div>
        {PREFERENCE_SECTIONS.map((s) => (
          <Link
            key={s.id}
            to={`/settings/${s.id}`}
            className={`nav-item${active === s.id ? ' active' : ''}`}
          >
            <span className="nav-item-icon">{s.icon}</span>
            <span className="nav-item-label">{s.label}</span>
          </Link>
        ))}
      </div>

      <div className="nav-group">
        <div className="nav-group-label">开发环境</div>
        {DEV_SECTIONS.map((s) => (
          <Link
            key={s.id}
            to={`/settings/${s.id}`}
            className={`nav-item${active === s.id ? ' active' : ''}`}
          >
            <span className="nav-item-icon">{s.icon}</span>
            <span className="nav-item-label">{s.label}</span>
          </Link>
        ))}
      </div>

      <div className="nav-group">
        <div className="nav-group-label">管理与资源</div>
        {RESOURCE_LINKS.map((l) => (
          <Link key={l.to} to={l.to} className="nav-item">
            <span className="nav-item-icon">{l.icon}</span>
            <span className="nav-item-label">{l.label}</span>
            <span className="nav-item-arrow">›</span>
          </Link>
        ))}
      </div>
    </nav>
  );
}
