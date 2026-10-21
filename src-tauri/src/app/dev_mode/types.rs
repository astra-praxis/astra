use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Child;
use std::sync::Mutex;

// ===== 数据结构 =====

/// 卡片子目录信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevCardInfo {
    /// 显示名称（优先取 README.md 首行标题，否则同 folder_name）
    pub display_name: String,
    /// 文件夹名称
    pub folder_name: String,
    /// 卡片完整路径
    pub path: String,
    /// 前端子目录列表（可直接执行 pnpm dev 的目录）
    pub sub_dirs: Vec<DevSubDir>,
    /// 卡片类别: template / main / card
    pub category: String,
}

/// 卡片的前端子目录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevSubDir {
    /// 显示标签: Web / Mobile / Component / PC
    pub label: String,
    /// 类型 key: web / mobile / component / pc
    pub key: String,
    /// 完整工作目录路径（pnpm dev 的工作目录）
    pub work_dir: String,
    /// 是否有 package.json（确认是前端项目）
    pub has_package_json: bool,
}

/// 正在运行的开发服务器
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevServerInfo {
    /// 服务器唯一 ID
    pub id: String,
    /// 卡片名称
    pub card_name: String,
    /// 子目录类型 (web/mobile/component)
    pub subdir: String,
    /// 工作目录（卡片下的具体子目录完整路径）
    pub work_dir: String,
    /// 命令名称 (dev / install)
    pub command: String,
    /// 启动时间戳
    pub started_at: u64,
    /// 状态: running / stopped / error
    pub status: String,
    /// PID
    pub pid: Option<u32>,
    /// 监听端口（从输出日志解析）
    pub port: Option<u16>,
}

/// 内部管理的运行中进程
pub struct RunningDevProcess {
    #[allow(dead_code)]
    pub id: String,
    pub card_name: String,
    pub subdir: String,
    pub work_dir: String,
    pub command: String,
    pub child: Child,
    pub started_at: u64,
    pub pid: u32,
    /// 从输出日志解析的端口
    pub port: Option<u16>,
}

pub type DevServerState = Mutex<HashMap<String, RunningDevProcess>>;

/// 开发配置（从 master vue.config.js 读取）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevConfig {
    /// Cookie 值，如 "WISCPSID=a2acc58b-..."
    pub cookie: String,
    /// 后端代理地址，如 "https://i.ygu.edu.cn/"
    pub proxy_target: String,
    /// master devServer 端口
    pub port: Option<u16>,
}

/// 开发服务器历史记录条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevHistoryEntry {
    /// 工作目录（唯一标识）
    pub work_dir: String,
    /// 卡片名称
    pub card_name: String,
    /// 子目录 key (web/mobile/component)
    pub subdir_key: String,
    /// 启动命令
    pub command: String,
    /// 所属项目路径
    pub project_path: String,
    /// 最后启动时间戳
    pub last_started_at: u64,
}
