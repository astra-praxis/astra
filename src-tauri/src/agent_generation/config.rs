use std::path::{Path, PathBuf};


use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};


// ===== 配置（generation.json）=====

/// 生成供应商配置；文件缺失 = 默认值（disabled）
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GenerationConfig {
    pub enabled: bool,
    pub base_url: String,
    pub api_key: String,
    pub model_id: String,
}

impl GenerationConfig {
    /// 归一化（trim + 去尾部 /），set_config 落盘前调用
    pub(crate) fn normalized(mut self) -> Self {
        self.base_url = self.base_url.trim().trim_end_matches('/').to_string();
        self.api_key = self.api_key.trim().to_string();
        self.model_id = self.model_id.trim().to_string();
        self
    }

    /// enabled 且配置完整（api_key / model_id 非空）才可用
    pub(crate) fn usable(&self) -> bool {
        self.enabled && !self.api_key.is_empty() && !self.model_id.is_empty()
    }
}

pub(crate) fn read_config_file(path: &Path) -> GenerationConfig {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub(crate) fn write_config_file(path: &Path, config: &GenerationConfig) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败: {e}"))?;
    }
    let content = serde_json::to_string_pretty(&config.clone().normalized()).unwrap_or_default();
    std::fs::write(path, content).map_err(|e| format!("写入配置文件失败: {e}"))
}

fn generation_config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取应用数据目录失败: {e}"))?;
    Ok(dir.join("generation.json"))
}

/// 读取生成配置（文件缺失/损坏 → 默认 disabled，不报错）
pub fn load_config(app: &AppHandle) -> GenerationConfig {
    match generation_config_path(app) {
        Ok(path) => read_config_file(&path),
        Err(_) => GenerationConfig::default(),
    }
}

/// 读取生成配置（可注入路径，便于测试与命令复用）
#[tauri::command]
pub fn generation_get_config(app: AppHandle) -> Result<GenerationConfig, String> {
    let path = generation_config_path(&app)?;
    Ok(read_config_file(&path))
}

/// 保存生成配置。新配置对之后新建的会话生效（已开 Thread 的工具集合冻结）。
#[tauri::command]
pub fn generation_set_config(app: AppHandle, config: GenerationConfig) -> Result<(), String> {
    let path = generation_config_path(&app)?;
    write_config_file(&path, &config)
}
