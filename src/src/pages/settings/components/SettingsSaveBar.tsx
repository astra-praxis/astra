/**
 * 底部保存栏：三态（已同步 / 未保存 / 失败）+ 保存成功闪现。
 * 草稿为空（尚未加载完成）时不渲染。
 */
import { Check, RotateCcw, Save, TriangleAlert } from 'lucide-react';
import { settingsEqual, useSettingsStore } from '../hooks/useSettings';

export default function SettingsSaveBar() {
  const draft = useSettingsStore((s) => s.draft);
  const baseline = useSettingsStore((s) => s.baseline);
  const saving = useSettingsStore((s) => s.saving);
  const savedFlash = useSettingsStore((s) => s.savedFlash);
  const saveError = useSettingsStore((s) => s.saveError);
  const save = useSettingsStore((s) => s.save);
  const discard = useSettingsStore((s) => s.discard);

  if (!draft || !baseline) return null;

  const dirty = !settingsEqual(draft, baseline);
  const hasError = saveError.length > 0;

  let dotClass = '';
  let statusText = '设置已同步到当前配置。更改会在保存后生效，外观主题则立即生效。';
  if (hasError) {
    dotClass = ' error';
    statusText = saveError;
  } else if (dirty) {
    dotClass = ' dirty';
    statusText = '有未保存的更改，保存后生效。';
  } else if (savedFlash) {
    dotClass = ' saved';
    statusText = '已保存。';
  }

  return (
    <footer className={`settings-savebar${dirty ? ' is-dirty' : ''}`}>
      <div className="savebar-status">
        <span className={`savebar-dot${dotClass}`} />
        <span className="savebar-text">{statusText}</span>
      </div>
      {(dirty || hasError) && (
        <div className="savebar-actions">
          <button className="btn-discard" onClick={discard} disabled={saving}>
            <RotateCcw size={14} strokeWidth={2} />
            放弃更改
          </button>
          <button className="btn-save" onClick={() => void save()} disabled={saving}>
            {saving ? (
              '保存中...'
            ) : savedFlash ? (
              <>
                <Check size={16} strokeWidth={3} />
                已保存
              </>
            ) : (
              <>
                <Save size={16} strokeWidth={2.5} />
                保存设置
              </>
            )}
          </button>
        </div>
      )}
      {!dirty && !hasError && savedFlash && (
        <div className="savebar-actions">
          <span className="savebar-saved-hint">
            <Check size={14} strokeWidth={3} />
          </span>
        </div>
      )}
      {hasError && !dirty && (
        <div className="savebar-actions">
          <span className="savebar-error-hint">
            <TriangleAlert size={14} strokeWidth={2} />
            修改未丢失，可修正后重试或放弃更改
          </span>
        </div>
      )}
    </footer>
  );
}
