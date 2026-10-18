//! 宿主侧记忆集成（astra-memory → 星野桌面）。
//!
//! 分层（astra-memory docs/08 + docs/02 铁律）：
//! - `MemoryHost` 拥有 `MemoryEngine`：加密 SQLite 存储（app_data_dir/memory.sqlite，
//!   主密钥在 memory.key，缺失时用两个 UUIDv4 生成 32 字节后落盘）。
//! - `AstraMemoryProvider` 适配 agent-core 冻结的 `MemoryProvider`，经
//!   `ThreadConfig.memory` 注入每个 Run —— core 侧自动做 recall 注入
//!   （REMEMBERED CONTEXT，见 astra-core run.rs），store/observe 不经 core。
//! - `observe` 是宿主能力：Run 结束后把用户输入喂进 Formation 管线。
//!   v2 全量 formation：加载进程内 OnnxJudge（fp16 图 + onnxruntime
//!   load-dynamic），普通消息也走 judge 决策（kind/importance/stable 等）；
//!   模型目录或 dylib 缺失时降级回 v1（仅显式通道 + recall 粗排，D21）。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use astra_memory::decision::DecisionModel;
use astra_memory::embed::{BgeEmbedder, Embedder};
use astra_memory::judge_onnx::{OnnxJudge, OnnxJudgeConfig};
use astra_memory::model::SubjectRef;
use astra_memory::protocol::{ForgetRequest, RecallHit, RecallQuery, RememberRequest};
use astra_memory::store_sqlite::SqliteStore;
use astra_memory::{MemoryEngine, MemoryKind};
use astra_memory_provider::AstraMemoryProvider;

const KEY_FILE: &str = "memory.key";
const DB_FILE: &str = "memory.sqlite";
/// 管理接口单次返回的条数上限（recall 的 context-budget 同源）
const DEFAULT_SEARCH_LIMIT: usize = 50;

/// bundle resources 里模型目录 / onnxruntime dylib 的固定落点
/// （与 tauri.conf.json 的 resources 映射一致）。
const MODEL_RESOURCE_DIR: &str = "memory-model";
/// embedding 模型（bge-small-zh-v1.5 int8）在 bundle resources 里的落点。
const EMBED_RESOURCE_DIR: &str = "embed-model";

/// Judge 部署物路径（bundle resources 或环境变量覆盖）。
///
/// 解析顺序：`ASTRA_MEMORY_MODEL_DIR`（开发机 / 换模型不重打包）→
/// bundle resource dir 下的 `memory-model/`；dylib 同理
/// （`ASTRA_MEMORY_ORT_DYLIB` → 各平台固定文件名）。任一缺失只影响
/// judge 加载，MemoryHost 降级 v1，不影响存储与显式通道。
pub struct MemoryModelPaths {
    pub model_dir: Option<PathBuf>,
    pub ort_dylib: Option<PathBuf>,
    /// embedding 模型目录（recall 语义通道）；缺失只降级为词法 recall。
    pub embed_dir: Option<PathBuf>,
}

impl MemoryModelPaths {
    pub fn resolve(resource_dir: Option<&Path>) -> Self {
        let model_dir = std::env::var_os("ASTRA_MEMORY_MODEL_DIR")
            .map(PathBuf::from)
            .or_else(|| {
                resource_dir
                    .map(|r| r.join(MODEL_RESOURCE_DIR))
                    .filter(|p| p.is_dir())
            });
        let ort_dylib = std::env::var_os("ASTRA_MEMORY_ORT_DYLIB")
            .map(PathBuf::from)
            .or_else(|| {
                resource_dir.and_then(|r| {
                    ort_dylib_candidates()
                        .into_iter()
                        .map(|name| r.join("onnxruntime").join(name))
                        .find(|p| p.is_file())
                })
            });
        let embed_dir = std::env::var_os("ASTRA_MEMORY_EMBED_DIR")
            .map(PathBuf::from)
            .or_else(|| {
                resource_dir
                    .map(|r| r.join(EMBED_RESOURCE_DIR))
                    .filter(|p| p.is_dir())
            });
        Self {
            model_dir,
            ort_dylib,
            embed_dir,
        }
    }
}

/// 各平台 onnxruntime 动态库的候选文件名（bundle 里的 `onnxruntime/` 目录）。
fn ort_dylib_candidates() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &["libonnxruntime.dylib", "libonnxruntime.1.31.0.dylib"]
    } else if cfg!(target_os = "windows") {
        &["onnxruntime.dll"]
    } else {
        &["libonnxruntime.so", "libonnxruntime.so.1.31.0"]
    }
}

pub struct MemoryHost {
    engine: Arc<MemoryEngine>,
    provider: Arc<AstraMemoryProvider>,
    db_path: PathBuf,
    /// 实际加载的 judge 图文件名（如 `laya_fp16.onnx`）；None = v1 降级。
    judge_model: Option<String>,
}

impl MemoryHost {
    /// 打开（或首次创建）加密存储并装配引擎。任何失败都由调用方降级为
    /// memory-off（agent 照常运行，只是没有 recall 注入）。judge 加载失败
    /// 只降级为 v1（无模型 formation），不影响存储与显式通道。
    pub fn init(dir: &Path, models: &MemoryModelPaths) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("create memory dir: {e}"))?;
        let key = load_or_create_key(&dir.join(KEY_FILE))?;
        let db_path = dir.join(DB_FILE);
        let store =
            SqliteStore::open(&db_path, &key).map_err(|e| format!("open memory store: {e}"))?;
        let judge = load_judge(models);
        let judge_model = judge.as_ref().map(|j| j.model_file().to_owned());
        let model: Option<Arc<dyn DecisionModel>> =
            judge.map(|j| Arc::new(j) as Arc<dyn DecisionModel>);
        let embedder = load_embedder(models);
        let mut engine = MemoryEngine::new(Arc::new(store), model);
        if let Some(e) = embedder {
            engine = engine.with_embedder(Arc::new(e) as Arc<dyn Embedder>);
        }
        if let Some(p) = load_proposer() {
            engine = engine.with_proposer(p);
        }
        let engine = Arc::new(engine);
        let provider = Arc::new(AstraMemoryProvider::new(Arc::clone(&engine)));
        Ok(Self {
            engine,
            provider,
            db_path,
            judge_model,
        })
    }

    /// Formation 管线入口（宿主 observe 用；agent 模型永不接触）。
    pub(crate) fn engine(&self) -> &Arc<MemoryEngine> {
        &self.engine
    }

    /// agent-core `MemoryProvider` 适配器（注入 ThreadConfig.memory 用）。
    pub fn provider(&self) -> Arc<AstraMemoryProvider> {
        Arc::clone(&self.provider)
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
}

/// 加载 embedding 模型（recall 语义通道，bge-small-zh-v1.5 int8）。
/// ORT_DYLIB_PATH 由 load_judge 设置；judge 缺席时这里兜底设置 ——
/// 两个模型共用同一个 dylib。任何失败都返回 None 降级词法 recall，
/// 不影响存储与显式通道（D21）。
fn load_embedder(models: &MemoryModelPaths) -> Option<BgeEmbedder> {
    let (Some(embed_dir), Some(dylib)) = (&models.embed_dir, &models.ort_dylib) else {
        eprintln!(
            "[memory] embedder disabled (lexical-only recall): embed_dir={:?} dylib={:?}",
            models.embed_dir, models.ort_dylib
        );
        return None;
    };
    if std::env::var_os("ORT_DYLIB_PATH").is_none() {
        std::env::set_var("ORT_DYLIB_PATH", dylib);
    }
    // 同 load_judge：ort dylib 加载失败是 panic，必须在启动帧截住。
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        BgeEmbedder::load(embed_dir, 2)
    })) {
        Ok(Ok(e)) => {
            eprintln!("[memory] embedder loaded: {}", embed_dir.display());
            Some(e)
        }
        Ok(Err(error)) => {
            eprintln!("[memory] embedder load failed, lexical-only recall: {error}");
            None
        }
        Err(payload) => {
            let msg = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("ort dylib load panic");
            eprintln!("[memory] embedder load panicked, lexical-only recall: {msg}");
            None
        }
    }
}

/// LLM Cognitive Proposer（docs/11 §3，Cognitive Layer v1）：可选 ——
/// env `ASTRA_MEMORY_PROPOSER_URL`（OpenAI 兼容 base url）+ 可选
/// `ASTRA_MEMORY_PROPOSER_MODEL` / `ASTRA_MEMORY_PROPOSER_KEY`。env 缺失
/// 时返回 None，行为与纯确定性管线完全一致；只在 pipeline 未落盘的
/// Observed/Discarded 事件上被 best-effort 调用（见 engine）。
fn load_proposer() -> Option<Arc<dyn astra_memory::proposer::CognitiveProposer>> {
    let base_url = std::env::var("ASTRA_MEMORY_PROPOSER_URL").ok()?;
    let model =
        std::env::var("ASTRA_MEMORY_PROPOSER_MODEL").unwrap_or_else(|_| "default".to_string());
    let key = std::env::var("ASTRA_MEMORY_PROPOSER_KEY").ok();
    eprintln!(
        "[memory] llm proposer enabled: url={} model={}",
        base_url, model
    );
    Some(Arc::new(astra_memory::proposer::OpenAiCompatProposer::new(
        &base_url,
        &model,
        key.as_deref(),
    )))
}

/// 加载进程内 OnnxJudge：先设 ORT_DYLIB_PATH（ort load-dynamic 在首次
/// init 时读取，必须发生在任何 ort 初始化之前），再按目录契约装载。
/// 任何失败都返回 None 降级 v1 —— 日志打原因，不 panic、不静默。
fn load_judge(models: &MemoryModelPaths) -> Option<OnnxJudge> {
    let (Some(model_dir), Some(dylib)) = (&models.model_dir, &models.ort_dylib) else {
        eprintln!(
            "[memory] judge disabled (v1 explicit-only): model_dir={:?} dylib={:?}",
            models.model_dir, models.ort_dylib
        );
        return None;
    };
    if std::env::var_os("ORT_DYLIB_PATH").is_none() {
        // edition 2021：set_var 安全；进程生命周期内只设一次
        std::env::set_var("ORT_DYLIB_PATH", dylib);
    }
    // ort 的 dylib 加载失败是 panic（load_dynamic::init → unwrap_or_else
    // panic!），不是 Err。panic 若一路 unwind 到 ObjC 的 extern "C" 回调帧
    // （did_finish_launching）会 abort 整个 app —— 必须在这里截住。
    // AssertUnwindSafe：judge 构造过程无跨 panic 的不变量需要保护。
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        OnnxJudge::load(OnnxJudgeConfig::new(model_dir))
    })) {
        Ok(Ok(judge)) => {
            eprintln!("[memory] judge loaded: {}", judge.model_file());
            Some(judge)
        }
        Ok(Err(error)) => {
            eprintln!("[memory] judge load failed, falling back to v1: {error}");
            None
        }
        Err(payload) => {
            let msg = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("ort dylib load panic");
            eprintln!("[memory] judge load panicked, falling back to v1: {msg}");
            None
        }
    }
}

/// 32 字节主密钥：存在则读，不存在则生成并落盘（0600）。
/// 主密钥与库文件同目录——本地方便性取舍，密钥层级升级（Keychain /
/// 硬件绑定）留给 docs/07 的 KeyProvider 演进，接口已预留。
fn load_or_create_key(path: &Path) -> Result<Vec<u8>, String> {
    if let Ok(bytes) = std::fs::read(path) {
        if bytes.len() >= 32 {
            return Ok(bytes);
        }
        return Err(format!(
            "{} exists but is shorter than 32 bytes; refusing to silently rotate the key (existing data would be unreadable)",
            path.display()
        ));
    }
    // 两个 UUIDv4 = 32 字节随机（uuid crate 无 getrandom 特性也能用）
    let mut key = [0u8; 32];
    key[..16].copy_from_slice(&uuid::Uuid::new_v4().as_bytes()[..]);
    key[16..].copy_from_slice(&uuid::Uuid::new_v4().as_bytes()[..]);
    std::fs::write(path, key).map_err(|e| format!("write {KEY_FILE}: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(key.to_vec())
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ===== DTO（不把 astra-memory 内部结构直接暴露给前端） =====

#[derive(serde::Serialize)]
pub struct MemoryItem {
    pub id: String,
    pub content: String,
    pub kind: String,
    pub confidence: f32,
    pub strength: f32,
    pub created_at: u64,
}

#[derive(serde::Serialize)]
pub struct MemoryStatus {
    pub enabled: bool,
    /// 决策模型：onnx 图文件名（如 "laya_fp16.onnx"）；"none" = v1 降级
    pub model: String,
    pub db_path: String,
}

fn item_from_hit(hit: RecallHit) -> MemoryItem {
    MemoryItem {
        id: hit.id,
        content: hit.content,
        kind: serde_json::to_value(hit.kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "other".into()),
        confidence: hit.confidence,
        strength: hit.strength,
        created_at: hit.created_at,
    }
}

// ===== Tauri 命令（memory 关闭时统一报错，不静默） =====

fn host_or_err<'a>(
    state: &'a tauri::State<'_, crate::agent::AgentState>,
) -> Result<&'a MemoryHost, String> {
    state
        .memory
        .get()
        .ok_or_else(|| "memory is unavailable (init failed or not started)".to_string())
}

#[tauri::command]
pub fn memory_status(state: tauri::State<'_, crate::agent::AgentState>) -> MemoryStatus {
    match state.memory.get() {
        Some(host) => MemoryStatus {
            enabled: true,
            model: host.judge_model.clone().unwrap_or_else(|| "none".into()),
            db_path: host.db_path().display().to_string(),
        },
        None => MemoryStatus {
            enabled: false,
            model: "none".into(),
            db_path: String::new(),
        },
    }
}

/// 语义检索（跨全部 subject，最相关在前）；空查询明确拒绝。
#[tauri::command]
pub async fn memory_search(
    state: tauri::State<'_, crate::agent::AgentState>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<MemoryItem>, String> {
    let host = host_or_err(&state)?;
    let query = query.trim().to_owned();
    if query.is_empty() {
        return Err("query must not be empty".to_string());
    }
    let q = RecallQuery {
        text: query,
        subject: None,
        kinds: Vec::new(),
        limit: limit.unwrap_or(DEFAULT_SEARCH_LIMIT).min(500),
    };
    let hits = host
        .engine
        .recall(&q, now_secs())
        .await
        .map_err(|e| e.to_string())?;
    Ok(hits.into_iter().map(item_from_hit).collect())
}

/// 显式写入（UI「添加记忆」按钮）；与聊天里的「记住…」同一条显式通道，
/// PII 拦截、置信度钉 1.0 语义一致。
#[tauri::command]
pub async fn memory_remember(
    state: tauri::State<'_, crate::agent::AgentState>,
    content: String,
) -> Result<MemoryItem, String> {
    let host = host_or_err(&state)?;
    let req = RememberRequest {
        subject: SubjectRef::user(),
        kind: MemoryKind::Preference,
        content: content.trim().to_owned(),
        importance: None,
    };
    let hit = host
        .engine
        .remember(&req, now_secs())
        .await
        .map_err(|e| e.to_string())?;
    Ok(item_from_hit(hit))
}

/// 硬删除单条（唯一删除入口，级联证据与版本链）。
#[tauri::command]
pub async fn memory_forget(
    state: tauri::State<'_, crate::agent::AgentState>,
    id: String,
) -> Result<(), String> {
    let host = host_or_err(&state)?;
    let n = host
        .engine
        .forget(&ForgetRequest::One { id })
        .await
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("memory not found".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_roundtrip_and_short_key_rejected() {
        let dir = std::env::temp_dir().join(format!("astra-mem-key-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(KEY_FILE);
        let k1 = load_or_create_key(&path).unwrap();
        assert_eq!(k1.len(), 32);
        // 第二次读同一文件得到同一密钥
        let k2 = load_or_create_key(&path).unwrap();
        assert_eq!(k1, k2);
        // 过短密钥拒绝（防静默换钥导致旧库不可读）
        std::fs::write(&path, b"short").unwrap();
        assert!(load_or_create_key(&path).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn init_creates_store_and_explicit_roundtrip() {
        let dir = std::env::temp_dir().join(format!("astra-mem-host-{}", uuid::Uuid::new_v4()));
        // 无模型路径：显式通道 + v1 降级（judge 加载依赖 bundle 部署物，
        // 单测环境没有 —— 全管线冒烟见下方 ignored 测试）
        let host = MemoryHost::init(&dir, &MemoryModelPaths::resolve(None)).unwrap();
        assert_eq!(host.judge_model, None);
        // 显式「记住」→ observe 命中显式口令 → 可被检索
        let outcome = host
            .engine
            .observe("记住我喜欢深色主题", &SubjectRef::user(), 1_000)
            .await
            .unwrap();
        assert!(matches!(
            outcome,
            astra_memory::formation::Outcome::Created { .. }
        ));
        let hits = host
            .engine
            .recall(
                &RecallQuery {
                    text: "深色主题".into(),
                    subject: None,
                    kinds: Vec::new(),
                    limit: 5,
                },
                1_001,
            )
            .await
            .unwrap();
        assert!(hits.iter().any(|h| h.content.contains("深色主题")));
        // 普通消息（无模型）：Candidate 在 D21 降级路径只挂 observation，
        // 不形成长期记忆
        let outcome = host
            .engine
            .observe("今天天气不错", &SubjectRef::user(), 1_002)
            .await
            .unwrap();
        assert!(matches!(
            outcome,
            astra_memory::formation::Outcome::Observed | astra_memory::formation::Outcome::Ignored
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 全管线冒烟：真实模型 + dylib（bundle 语义）。跑法：
    /// ORT_DYLIB_PATH=<onnxruntime dylib> ASTRA_MEMORY_MODEL_DIR=<v3 目录> \
    ///   cargo test -p astra --lib memory -- --ignored --nocapture
    /// 首载 646M fp16 图约数十秒，只验装不进 CI。
    #[tokio::test]
    #[ignore]
    async fn judge_full_pipeline_when_model_available() {
        let paths = MemoryModelPaths::resolve(None);
        let (Some(_), Some(_)) = (&paths.model_dir, &paths.ort_dylib) else {
            panic!(
                "set ASTRA_MEMORY_MODEL_DIR and ASTRA_MEMORY_ORT_DYLIB (or ORT_DYLIB_PATH) to run"
            );
        };
        let dir = std::env::temp_dir().join(format!("astra-mem-judge-{}", uuid::Uuid::new_v4()));
        let host = MemoryHost::init(&dir, &paths).unwrap();
        let model = host.judge_model.clone().expect("judge must load");
        eprintln!("loaded judge: {model}");
        // 普通消息（无显式口令）：v2 下由 judge 决策形成长期记忆
        let outcome = host
            .engine
            .observe(
                "我最近在写一个 Rust 的 agent runtime，项目叫星野",
                &SubjectRef::user(),
                1_000,
            )
            .await
            .unwrap();
        eprintln!("observe outcome: {outcome:?}");
        let hits = host
            .engine
            .recall(
                &RecallQuery {
                    text: "星野项目用什么语言".into(),
                    subject: None,
                    kinds: Vec::new(),
                    limit: 5,
                },
                1_001,
            )
            .await
            .unwrap();
        eprintln!("recall hits (natural query): {}", hits.len());
        for h in &hits {
            eprintln!("  - [{:?}] {}", h.kind, h.content);
        }
        // 粗排是词元级（bigram containment，docs/03）：自然语言查询必须
        // 命中刚形成的记忆，否则全量 formation 的 recall 注入无意义。
        assert!(!hits.is_empty(), "natural-language recall must hit");
        std::fs::remove_dir_all(&dir).ok();
    }
}
