/**
 * 设置中心外壳：左侧分类导航 + 右侧内容区 + 固定保存栏。
 * 分类经 /settings/:section 路由可达（支持深链）；内容区独立滚动。
 */
import { useEffect } from 'react';
import { Navigate, useParams } from 'react-router';
import SettingsSidebar from './components/SettingsSidebar';
import SettingsSaveBar from './components/SettingsSaveBar';
import { ALL_SECTIONS } from './sections';
import { useSettingsStore } from './hooks/useSettings';
import './SettingsPage.scss';

export default function SettingsPage() {
  const { section } = useParams<{ section: string }>();

  const load = useSettingsStore((s) => s.load);
  const loaded = useSettingsStore((s) => s.loaded);
  const loadError = useSettingsStore((s) => s.loadError);

  useEffect(() => {
    void load();
  }, [load]);

  const active = ALL_SECTIONS.find((s) => s.id === section);
  if (!active) {
    return <Navigate to="/settings/general" replace />;
  }

  const ActiveComponent = active.component;

  return (
    <div className="settings-page">
      <aside className="settings-sidebar">
        <div className="sidebar-brand">
          <h1>设置</h1>
          <p>配置 Astra 的工作方式、模型行为与本地环境</p>
        </div>
        <SettingsSidebar active={active.id} />
      </aside>

      <div className="settings-main">
        <div className="settings-content" key={active.id}>
          {loadError && (
            <div className="load-error">设置加载失败：{loadError}</div>
          )}
          {loaded && <ActiveComponent />}
        </div>
        <SettingsSaveBar />
      </div>
    </div>
  );
}
