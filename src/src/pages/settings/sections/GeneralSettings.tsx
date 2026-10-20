/**
 * 常规：外观主题（即时生效）+ 项目扫描深度（草稿保存）
 */
import { Check } from 'lucide-react';
import { useThemeStore } from '@/lib/theme';
import SettingsSection from '../components/SettingsSection';
import SettingsRow from '../components/SettingsRow';
import { useSettingsStore } from '../hooks/useSettings';

const themeOptions: { value: 'light' | 'dark' | 'system'; label: string }[] = [
  { value: 'light', label: '浅色' },
  { value: 'dark', label: '深色' },
  { value: 'system', label: '跟随系统' },
];

export default function GeneralSettings() {
  const currentTheme = useThemeStore((s) => s.mode);
  const setThemeMode = useThemeStore((s) => s.setMode);

  const draft = useSettingsStore((s) => s.draft);
  const update = useSettingsStore((s) => s.update);

  return (
    <SettingsSection iconId="general" title="常规" desc="调整外观和项目扫描的默认行为">
      <SettingsRow stacked title="外观主题" desc="选择应用的显示模式，切换立即生效">
        <div className="theme-grid" role="radiogroup" aria-label="外观主题">
          {themeOptions.map((opt) => (
            <button
              key={opt.value}
              role="radio"
              aria-checked={currentTheme === opt.value}
              className={`theme-option${currentTheme === opt.value ? ' selected' : ''}`}
              onClick={() => setThemeMode(opt.value)}
            >
              {/* 迷你主题预览：固定色值展示目标主题外观，不随当前主题变化 */}
              <span className={`theme-preview theme-preview--${opt.value}`}>
                <span className="theme-preview-bar">
                  <i /><i /><i />
                </span>
                <span className="theme-preview-body">
                  <span className="theme-preview-hero" />
                  <span className="theme-preview-line" />
                  <span className="theme-preview-line short" />
                </span>
              </span>
              <span className="theme-meta">
                <span className="theme-name">{opt.label}</span>
                {currentTheme === opt.value && <Check size={15} strokeWidth={2.5} className="theme-check" />}
              </span>
            </button>
          ))}
        </div>
      </SettingsRow>

      <SettingsRow
        title="项目扫描深度"
        desc="点击项目卡片查看详情时，递归遍历子目录的最大层级（1–5，默认 3）"
      >
        <input
          type="number"
          min={1}
          max={5}
          value={draft?.scanDepth ?? 3}
          className="agent-perm-input"
          onChange={(e) => {
            const n = Number(e.target.value);
            if (Number.isFinite(n)) update({ scanDepth: Math.min(5, Math.max(1, Math.floor(n))) });
          }}
        />
      </SettingsRow>
    </SettingsSection>
  );
}
