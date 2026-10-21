/**
 * 工具箱页面
 * 移植自 src/routes/tools/+page.svelte
 */
import { useState } from 'react';
import { Link, useNavigate } from 'react-router';
import { ChevronLeft, Search, X, ChevronRight } from 'lucide-react';
import './ToolsPage.scss';

interface Tool {
  id: string;
  name: string;
  description: string;
  icon: string;
  path: string;
  status: 'stable' | 'beta' | 'alpha';
}

const tools: Tool[] = [
  {
    id: 'downloader',
    name: '下载管理',
    description: '支持断点续传的文件下载工具，可管理多个下载任务',
    icon: '📥',
    path: '/tools/downloader',
    status: 'stable',
  },
  {
    id: 'nm-clean',
    name: 'node_modules 清理',
    description:
      '扫描工作空间中的所有 node_modules 文件夹，查看大小并一键清理，清理过程实时显示进度',
    icon: '🗑️',
    path: '/tools/nm',
    status: 'stable',
  },
];

function getStatusLabel(status: Tool['status']): { text: string; className: string } {
  switch (status) {
    case 'stable':
      return { text: '稳定', className: 'status-stable' };
    case 'beta':
      return { text: '测试', className: 'status-beta' };
    case 'alpha':
      return { text: '实验', className: 'status-alpha' };
    default:
      return { text: '', className: '' };
  }
}

export default function ToolsPage() {
  const navigate = useNavigate();
  const [searchQuery, setSearchQuery] = useState('');

  const q = searchQuery.toLowerCase();
  const filteredTools = searchQuery.trim()
    ? tools.filter(
        (t) => t.name.toLowerCase().includes(q) || t.description.toLowerCase().includes(q)
      )
    : tools;

  function openTool(tool: Tool) {
    navigate(tool.path);
  }

  return (
    <div className="tools-page">
      {/* 页头 */}
      <div className="page-header">
        <div className="header-left">
          <Link to="/settings" className="btn-back" title="返回设置">
            <ChevronLeft size={14} strokeWidth={2.5} />
            返回设置
          </Link>
          <div className="header-titles">
            <h1>工具箱</h1>
            <p className="page-subtitle">下载管理与 node_modules 清理</p>
          </div>
        </div>
      </div>

      {/* 搜索栏 */}
      <div className="search-section">
        <div className="search-box">
          <Search size={18} strokeWidth={2} className="search-icon" />
          <input
            type="text"
            placeholder="搜索工具..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
          />
          {searchQuery && (
            <button
              className="clear-btn"
              onClick={() => setSearchQuery('')}
              aria-label="清除搜索"
            >
              <X size={16} strokeWidth={2} />
            </button>
          )}
        </div>
        <div className="tools-count">{filteredTools.length} 个工具</div>
      </div>

      {/* 工具卡片列表 */}
      <div className="tools-grid">
        {filteredTools.map((tool) => {
          const statusInfo = getStatusLabel(tool.status);
          return (
            <div
              key={tool.id}
              className="tool-card"
              onClick={() => openTool(tool)}
              role="button"
              tabIndex={0}
              onKeyDown={(e) => e.key === 'Enter' && openTool(tool)}
            >
              <div className="tool-header">
                <span className="tool-icon">{tool.icon}</span>
                <div className="tool-info">
                  <h3 className="tool-name">
                    {tool.name}
                    <span className={`status-badge ${statusInfo.className}`}>
                      {statusInfo.text}
                    </span>
                  </h3>
                  <p className="tool-desc">{tool.description}</p>
                </div>
              </div>
              <div className="tool-footer">
                <span className="tool-action">
                  打开
                  <ChevronRight size={14} strokeWidth={2} />
                </span>
              </div>
            </div>
          );
        })}

        {filteredTools.length === 0 && (
          <div className="empty-state">
            <span className="empty-icon">🔍</span>
            <h3>未找到匹配的工具</h3>
            <p>尝试其他关键词搜索</p>
          </div>
        )}
      </div>
    </div>
  );
}
