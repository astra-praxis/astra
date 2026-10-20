/**
 * 搜索：搜索引擎选择；仅 SearXNG 时展示实例地址
 */
import SettingsSection from '../components/SettingsSection';
import SettingsRow from '../components/SettingsRow';
import { useSettingsStore } from '../hooks/useSettings';

export default function SearchSettings() {
  const draft = useSettingsStore((s) => s.draft);
  const update = useSettingsStore((s) => s.update);

  if (!draft) return null;

  return (
    <SettingsSection iconId="search" title="搜索" desc="Agent 联网搜索（search_web）使用的后端">
      <SettingsRow
        title="搜索引擎"
        desc="默认 Bing（一般可直连）。DuckDuckGo 部分网络不可达；SearXNG 需自建或指定实例地址（需开启 format=json）。保存后新会话生效。"
      >
        <select
          value={draft.searchEngine}
          className="agent-perm-input agent-perm-input--select"
          onChange={(e) => update({ searchEngine: e.target.value })}
        >
          <option value="bing">Bing（默认）</option>
          <option value="duckduckgo">DuckDuckGo</option>
          <option value="searxng">SearXNG（自建实例）</option>
        </select>
      </SettingsRow>

      {draft.searchEngine === 'searxng' && (
        <SettingsRow
          title="SearXNG 实例地址"
          desc="SearXNG 实例的基地址（如 https://searx.example.org），实例需允许 JSON 输出。"
        >
          <input
            type="text"
            value={draft.searchEndpoint}
            placeholder="https://searx.example.org"
            className="agent-perm-input agent-perm-input--wide"
            onChange={(e) => update({ searchEndpoint: e.target.value })}
          />
        </SettingsRow>
      )}
    </SettingsSection>
  );
}
