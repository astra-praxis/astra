//! 本地账户（多账户，热切换架构）。
//!
//! 三层边界中的「本地账户 = 数据归属边界」：每个账户拥有独立的数据目录与
//! astra-home（MCP/ACP 配置），账户身份由**后端进程内的活动绑定**决定，
//! 不由前端在 IPC 中任意指定。
//!
//! 目录布局：
//!
//! ```text
//! APPDATA/
//! ├── accounts.json              # 账户元数据（公共，不含任何敏感内容）
//! ├── .astra-instance.lock       # 全局单实例锁（pid + 存活检测）
//! └── accounts/<account_id>/
//!     ├── data/                  # 账户私有数据（原 app_data_dir 的全部落盘）
//!     └── astra-home/            # 账户私有 astra home（config.toml / pastes）
//! ```
//!
//! 未登录任何账户时保持 legacy 模式：所有落盘仍走原 app_data_dir /
//! `resolve_astra_home()`，旧版本行为完全不变。
//!
//! 热切换（账户切换 = 进程内换绑 + 前端重载）：`account_switch` 验证密码
//! → 取消运行中 Run / 清理会话缓存与扩展连接 / 关闭记忆宿主 → 重绑活动
//! 账户（数据目录与 ASTRA_HOME 随之切换）→ 重建记忆宿主。所有落盘路径
//! 都经 `data_dir()` / `astra_home()` 动态解析，换绑后自动指向新账户；
//! 前端随后整页重载，全部 store 重建，不重启进程。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use tauri::Manager;

use super::secret;

/// 账户元数据（公共库内容，禁止放入任何敏感信息——设计文档 §3.2）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    /// 不可变随机 ID，账户主键（显示名称不是身份凭证）
    pub account_id: String,
    pub display_name: String,
    /// Unix 毫秒时间戳
    pub created_at: i64,
    /// active | locked（锁定语义随 P0-3 加密上线，P0 仅占位）
    pub status: String,
    /// 是否已设置登录密码（读取时动态计算，不落盘）。
    /// false 的账户首次登录时需设置初始密码。
    #[serde(default, skip_serializing)]
    pub has_auth: bool,
}

/// 进程绑定的活动账户上下文（setup 阶段初始化一次，之后只读）
#[derive(Debug, Clone)]
pub(crate) struct AccountRuntime {
    pub account_id: String,
    /// 账户私有数据目录（替代 app_data_dir 的全部落盘）
    pub data_dir: PathBuf,
    /// 账户私有 astra home（config.toml / pastes）
    pub astra_home: PathBuf,
}

static ACTIVE: RwLock<Option<AccountRuntime>> = RwLock::new(None);

/// 已登出标志：用户点了「切换用户」但尚未登录其他账户。此时保持旧绑定
/// （避免后台写入落到 legacy 目录），account_status 返回 login 模式让
/// 前端渲染账户选择器；下次 account_switch 成功后清除。
static LOGGED_OUT: AtomicBool = AtomicBool::new(false);

/// 从启动参数解析 `--account=<account_id>` 并绑定活动账户。
/// 无参数 → legacy 模式（Ok(None)），保持旧版本行为。
pub(crate) fn init_from_args(
    app_data_root: &std::path::Path,
) -> Result<Option<AccountRuntime>, String> {
    let arg = std::env::args()
        .find(|a| a.starts_with("--account="))
        .map(|a| a["--account=".len()..].to_string());
    resolve_account(app_data_root, arg)
}

/// 按 account_id 解析账户运行时（从 init_from_args 拆出以便单测）。
/// 指定 ID 但账户不存在 / ID 非法 → Err（拒绝启动，不静默回退 legacy——
/// 带着明确账户意图启动却落到别的数据目录比失败更危险）。
fn resolve_account(
    app_data_root: &std::path::Path,
    account_id: Option<String>,
) -> Result<Option<AccountRuntime>, String> {
    let Some(account_id) = account_id else {
        return Ok(None);
    };
    if account_id.is_empty()
        || !account_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(format!("非法的账户 ID: {account_id}"));
    }
    let accounts = read_accounts(app_data_root)?;
    if !accounts.iter().any(|a| a.account_id == account_id) {
        return Err(format!("账户不存在: {account_id}"));
    }
    Ok(Some(AccountRuntime {
        data_dir: account_dir(app_data_root, &account_id).join("data"),
        astra_home: account_dir(app_data_root, &account_id).join("astra-home"),
        account_id,
    }))
}

/// setup 阶段调用：绑定启动参数指定的活动账户（如有的话）。
pub(crate) fn bind(runtime: Option<AccountRuntime>) {
    if let Ok(mut guard) = ACTIVE.write() {
        *guard = runtime;
    }
}

/// 热切换：替换活动账户绑定（account_switch 专用）。
fn rebind(runtime: Option<AccountRuntime>) {
    if let Ok(mut guard) = ACTIVE.write() {
        *guard = runtime;
    }
}

/// 当前绑定的账户（legacy 模式为 None）。快照式返回（内部 RwLock）。
pub(crate) fn active() -> Option<AccountRuntime> {
    ACTIVE.read().ok().and_then(|guard| guard.clone())
}

/// 账户私有数据目录。**这是全部账户私有落盘的唯一入口**——
/// 新增存储必须经此函数取路径，禁止直接调用 `app_data_dir()`
/// （见 docs/MULTI_ACCOUNT_INVENTORY.md §2 白名单规则）。
/// legacy 模式（无账户启动）返回原 app_data_dir，行为与旧版本一致。
pub(crate) fn data_dir(manager: &impl tauri::Manager<tauri::Wry>) -> Result<PathBuf, String> {
    if let Some(rt) = active() {
        std::fs::create_dir_all(&rt.data_dir).map_err(|e| format!("创建账户数据目录失败: {e}"))?;
        return Ok(rt.data_dir.clone());
    }
    manager
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取应用数据目录失败: {e}"))
}

/// 账户私有 astra home（config.toml / pastes）。
/// legacy 模式回退到 astra-core 的 resolve_astra_home()（~/.astra，ASTRA_HOME 可覆盖）。
pub(crate) fn astra_home() -> PathBuf {
    if let Some(rt) = active() {
        let _ = std::fs::create_dir_all(&rt.astra_home);
        return rt.astra_home.clone();
    }
    agent_core::astra_config::resolve_astra_home()
}

/// 账户目录：APPDATA/accounts/<account_id>/
fn account_dir(app_data_root: &std::path::Path, account_id: &str) -> PathBuf {
    app_data_root.join("accounts").join(account_id)
}

// ===== 账户元数据存储（APPDATA/accounts.json） =====

fn accounts_file(app_data_root: &std::path::Path) -> PathBuf {
    app_data_root.join("accounts.json")
}

fn read_accounts(app_data_root: &std::path::Path) -> Result<Vec<Account>, String> {
    let path = accounts_file(app_data_root);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| format!("读取账户列表失败: {e}"))?;
    let mut accounts: Vec<Account> =
        serde_json::from_str(&raw).map_err(|e| format!("解析账户列表失败: {e}"))?;
    // has_auth 是运行时状态（登录密码凭据文件是否存在），不落盘、每次读取时计算
    for a in accounts.iter_mut() {
        a.has_auth = secret::has_auth(&account_dir(app_data_root, &a.account_id).join("data"));
    }
    Ok(accounts)
}

fn write_accounts(app_data_root: &std::path::Path, accounts: &[Account]) -> Result<(), String> {
    std::fs::create_dir_all(app_data_root).map_err(|e| format!("创建数据目录失败: {e}"))?;
    let json =
        serde_json::to_string_pretty(accounts).map_err(|e| format!("序列化账户失败: {e}"))?;
    std::fs::write(accounts_file(app_data_root), json).map_err(|e| format!("写入账户列表失败: {e}"))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 生成不可变随机账户 ID（uuid v4 去连字符；uuid 已在依赖树中）
fn generate_account_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

// ===== 全局单实例锁 =====

/// 单实例锁守卫：持有时进程独占实例；Drop 时释放。
/// 锁文件写 pid，持锁进程崩溃后由 pid 存活检测判定锁失效（可抢回）。
#[derive(Debug)]
pub(crate) struct InstanceLock {
    path: PathBuf,
    held: bool,
}

impl InstanceLock {
    /// 尝试获取全局实例锁。已有存活实例持锁 → Err（含持有者 pid 提示）。
    /// 锁文件存在但 pid 已死 → 判定陈旧锁，抢回。
    pub(crate) fn acquire(app_data_root: &std::path::Path) -> Result<Self, String> {
        std::fs::create_dir_all(app_data_root).map_err(|e| format!("创建数据目录失败: {e}"))?;
        let path = app_data_root.join(".astra-instance.lock");
        if path.exists() {
            if let Ok(raw) = std::fs::read_to_string(&path) {
                if let Ok(pid) = raw.trim().parse::<i32>() {
                    if pid != std::process::id() as i32 && pid_alive(pid) {
                        return Err(format!(
                            "Astra 已在运行（pid {pid}）。同一安装同时只允许一个实例。"
                        ));
                    }
                }
            }
            // 陈旧锁（持锁进程已死或内容损坏）：覆盖抢回
        }
        std::fs::write(&path, std::process::id().to_string())
            .map_err(|e| format!("写入实例锁失败: {e}"))?;
        Ok(InstanceLock { path, held: true })
    }

    /// 主动释放（仅测试使用；热切换架构下生产路径不再释放，进程退出时经 Drop 清理）。
    #[cfg(test)]
    pub(crate) fn release(&mut self) {
        self.held = false;
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        if self.held {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// pid 存活检测（不杀进程，只探测）
fn pid_alive(pid: i32) -> bool {
    #[cfg(unix)]
    {
        // kill(pid, 0)：权限不足返回 EPERM 也说明进程存在
        unsafe {
            libc::kill(pid, 0) == 0
                || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid as u32);
            if handle.is_null() {
                return false;
            }
            let mut exit_code: u32 = 0;
            let alive =
                windows_sys::Win32::System::Threading::GetExitCodeProcess(handle, &mut exit_code)
                    != 0
                    && exit_code == STILL_ACTIVE;
            CloseHandle(handle);
            alive
        }
    }
}

// ===== Tauri 命令 =====

#[derive(Serialize)]
pub struct AccountStatus {
    /// account：已绑定账户；legacy：无账户启动（旧数据目录）；
    /// login：登出态（切换用户中，前端渲染账户选择器）。
    /// 账户选择由前端依据 accounts 是否为空决定渲染创建页还是选择页。
    pub mode: String,
    pub account_id: Option<String>,
    pub display_name: Option<String>,
    pub accounts: Vec<Account>,
    /// 当前数据目录是否设置了访问密码
    pub password_protected: bool,
    /// 设了密码且本进程尚未解锁（前端渲染解锁界面）
    pub locked: bool,
}

#[tauri::command]
pub fn account_status(app: tauri::AppHandle) -> Result<AccountStatus, String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取应用数据目录失败: {e}"))?;
    let accounts = read_accounts(&root)?;
    // 登出态（切换用户中）：保持旧绑定不写错目录，但让前端渲染选择器
    let logged_out = LOGGED_OUT.load(Ordering::Relaxed);
    let dir = crate::app::accounts::data_dir(&app).ok();
    let (password_protected, locked) = match &dir {
        Some(dir) => (
            crate::app::secret::has_auth(dir),
            crate::app::secret::is_locked(dir),
        ),
        None => (false, false),
    };
    let mode = match (logged_out, active()) {
        (true, _) => "login".to_string(),
        (false, Some(_)) => "account".to_string(),
        (false, None) => "legacy".to_string(),
    };
    let current = if logged_out { None } else { active() };
    let display_name = current.as_ref().and_then(|rt| {
        accounts
            .iter()
            .find(|a| a.account_id == rt.account_id)
            .map(|a| a.display_name.clone())
    });
    Ok(AccountStatus {
        mode,
        account_id: current.map(|rt| rt.account_id),
        display_name,
        accounts,
        password_protected,
        locked: locked && !logged_out,
    })
}

#[tauri::command]
pub fn account_list(app: tauri::AppHandle) -> Result<Vec<Account>, String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取应用数据目录失败: {e}"))?;
    read_accounts(&root)
}

/// 创建账户并设置登录密码（6-128 字符）。
/// 首个账户会承接 legacy 数据：迁移（含明文 KEK）完成后 auth_init 承接 KEK
/// 并用密码重新包装——旧密文（providers.json 等）依然可解。
#[tauri::command]
pub fn account_create(
    app: tauri::AppHandle,
    display_name: String,
    password: String,
) -> Result<Account, String> {
    let name = display_name.trim();
    if name.is_empty() || name.chars().count() > 32 {
        return Err("账户名称需为 1-32 个字符".into());
    }
    secret::validate_password(&password)?;
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取应用数据目录失败: {e}"))?;
    let mut accounts = read_accounts(&root)?;
    if accounts.len() >= 16 {
        return Err("账户数量已达上限（16）".into());
    }
    // 首个账户承接 legacy 单用户数据（P0-4）：必须在 push 之前判定
    let was_first_account = accounts.is_empty();
    let account = Account {
        account_id: generate_account_id(),
        display_name: name.to_string(),
        created_at: now_ms(),
        status: "active".into(),
        has_auth: false,
    };
    accounts.push(account.clone());
    write_accounts(&root, &accounts)?;
    // 预建账户目录结构（data / astra-home），保证 create 即可启动
    let dir = account_dir(&root, &account.account_id);
    std::fs::create_dir_all(dir.join("data")).map_err(|e| format!("创建账户数据目录失败: {e}"))?;
    std::fs::create_dir_all(dir.join("astra-home"))
        .map_err(|e| format!("创建账户 astra-home 失败: {e}"))?;
    // 首个账户承接 legacy 单用户数据（P0-4）：必须先于 auth_init 执行，
    // 这样 auth_init 才能承接迁移过来的明文 KEK（旧密文才能解）
    if was_first_account {
        match crate::app::migration::migrate_legacy_for_current_platform(
            &root,
            &dir,
            &account.account_id,
        ) {
            Ok(report) => app_log!(
                info,
                "accounts",
                "legacy migration done: moved={} copied={} skipped={} failed={}",
                report.moved.len(),
                report.copied.len(),
                report.skipped.len(),
                report.failed.len()
            ),
            Err(e) => app_log!(error, "accounts", "legacy migration failed: {e}"),
        }
    }
    // 设置登录密码：承接 legacy KEK（若有）或生成新 KEK，密码包装后落 auth 文件
    let data_dir = dir.join("data");
    secret::auth_init(&data_dir, &password)?;
    Ok(account)
}

/// 热切换的进程收尾：取消运行中 Run + 终止 dev 服务器进程树。
/// account_switch / account_logout 共用（dev 服务器属于旧账户的项目会话，
/// 换用户后不应存活；子进程不会随状态重置自动消亡）。
fn switch_cleanup_processes(app: &tauri::AppHandle) {
    let dev_state = app.state::<crate::app::dev_mode::DevServerState>();
    let mut servers = dev_state.lock().unwrap_or_else(|e| e.into_inner());
    for (_, mut server) in servers.drain() {
        let port = server.port;
        crate::app::dev_mode::terminate_process_tree(&mut server.child);
        if let Some(p) = port {
            crate::app::dev_mode::kill_processes_on_port(p);
        }
    }
}

/// 热切换：进程内换绑到目标账户，不重启进程。
///
/// 验证密码 → 取消运行中 Run / 清理会话缓存与扩展连接 / 关闭记忆宿主 →
/// 重绑活动账户（data_dir / astra_home / ASTRA_HOME 随之切换）→
/// 缓存新 KEK → 日志重定向 → 重建记忆宿主。前端收到 Ok 后整页重载，
/// 全部 store 重建；会话存储在下次 open 时自动指向新账户目录。
///
/// 老账户（无登录凭据，legacy 时期创建）兼容：首次登录即设置初始密码
/// （auth_init 承接其现有文件 KEK，旧密文不受影响）。
#[tauri::command]
pub async fn account_switch(
    app: tauri::AppHandle,
    account_id: String,
    password: String,
) -> Result<(), String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取应用数据目录失败: {e}"))?;
    let accounts = read_accounts(&root)?;
    let account = accounts
        .iter()
        .find(|a| a.account_id == account_id)
        .cloned()
        .ok_or_else(|| format!("账户不存在: {account_id}"))?;
    let data_dir = account_dir(&root, &account_id).join("data");
    // 登录验证：已有凭据 → 验证密码取 KEK；无凭据（老账户）→ 首次登录设初始密码
    let kek = if secret::has_auth(&data_dir) {
        secret::auth_verify(&data_dir, &password)?
    } else {
        secret::auth_init(&data_dir, &password)?
    };

    // 1. 清理旧账户的进程内状态（Run / 会话缓存 / 扩展连接 / 记忆宿主）
    let agent_state = app.state::<crate::agent::AgentState>();
    crate::agent::state::reset_for_account_switch(&agent_state).await;
    switch_cleanup_processes(&app);

    // 2. 换绑活动账户：所有 data_dir()/astra_home() 调用从此指向新账户
    let runtime = AccountRuntime {
        data_dir: data_dir.clone(),
        astra_home: account_dir(&root, &account_id).join("astra-home"),
        account_id: account_id.clone(),
    };
    rebind(Some(runtime));
    std::env::set_var(
        "ASTRA_HOME",
        account_dir(&root, &account_id).join("astra-home"),
    );

    // 3. 新账户的密钥与宿主初始化
    secret::cache_kek(kek);
    LOGGED_OUT.store(false, Ordering::Relaxed);
    if let Ok(dir) = crate::app::accounts::data_dir(&app) {
        crate::logging::init(&dir);
    }
    crate::init_memory_host(&app);
    app_log!(
        info,
        "accounts",
        "account switched: {} ({})",
        account.display_name,
        account.account_id
    );
    Ok(())
}

/// 退出登录（切换用户）：清理当前账户的进程内状态并清除 KEK 缓存，
/// 登出态下 account_status 返回 login 模式，前端重载后渲染账户选择器。
/// 不换绑（保持旧目录绑定，避免后台写入落到 legacy 目录）；
/// 下次 account_switch 成功后清除登出标志。
#[tauri::command]
pub async fn account_logout(app: tauri::AppHandle) -> Result<(), String> {
    let agent_state = app.state::<crate::agent::AgentState>();
    crate::agent::state::reset_for_account_switch(&agent_state).await;
    switch_cleanup_processes(&app);
    secret::clear_kek();
    LOGGED_OUT.store(true, Ordering::Relaxed);
    app_log!(info, "accounts", "account logged out, showing selector");
    Ok(())
}

/// InstanceLock 的 Tauri State 包装（setup 阶段 acquire 后 manage）。
/// 热切换架构下锁终生持有（退出时 Drop 清理），Mutex 包装保留以便未来
/// 扩展锁定/恢复流程，字段本身暂无读取方。
#[allow(dead_code)]
pub(crate) struct InstanceLockState(pub(crate) std::sync::Mutex<InstanceLock>);

// ===== 账户登录密码（praxis 式登录：选用户 → 输密码 → 进应用） =====

/// 解锁：验证密码，KEK 进程内缓存；随后补做被锁定跳过的初始化
/// （记忆宿主）。前端解锁成功后刷新页面即可正常使用。
#[tauri::command]
pub fn account_unlock(app: tauri::AppHandle, password: String) -> Result<(), String> {
    let dir = crate::app::accounts::data_dir(&app)?;
    crate::app::secret::account_unlock(&dir, &password)?;
    app_log!(info, "accounts", "account unlocked");
    crate::init_memory_host(&app);
    Ok(())
}

/// 更换登录密码（需验证当前密码）。KEK 不变（仅重新包装），
/// 已加密数据无需重加密。
#[tauri::command]
pub fn account_password_change(
    app: tauri::AppHandle,
    old_password: String,
    new_password: String,
) -> Result<(), String> {
    let dir = crate::app::accounts::data_dir(&app)?;
    crate::app::secret::password_change(&dir, &old_password, &new_password)?;
    app_log!(info, "accounts", "login password changed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_root(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("astra-accounts-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn create_and_read_accounts_roundtrip() {
        let root = tmp_root("roundtrip");
        let mut accounts = read_accounts(&root).unwrap();
        assert!(accounts.is_empty());
        accounts.push(Account {
            account_id: "abc123".into(),
            display_name: "工作".into(),
            created_at: 1,
            status: "active".into(),
            has_auth: false,
        });
        write_accounts(&root, &accounts).unwrap();
        let got = read_accounts(&root).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].display_name, "工作");
        assert_eq!(got[0].account_id, "abc123");
    }

    #[test]
    fn resolve_account_none_is_legacy() {
        let root = tmp_root("legacy");
        assert!(resolve_account(&root, None).unwrap().is_none());
    }

    #[test]
    fn resolve_account_rejects_invalid_and_missing() {
        let root = tmp_root("reject");
        assert!(resolve_account(&root, Some("../evil".into())).is_err());
        assert!(resolve_account(&root, Some("".into())).is_err());
        assert!(resolve_account(&root, Some("nope".into())).is_err());
        // 合法格式但不存在 → 也拒绝
        assert!(resolve_account(&root, Some("abc123def456".into())).is_err());
    }

    #[test]
    fn resolve_account_resolves_created_account_dirs() {
        let root = tmp_root("resolve");
        let mut accounts = read_accounts(&root).unwrap();
        accounts.push(Account {
            account_id: "abc123".into(),
            display_name: "工作".into(),
            created_at: 1,
            status: "active".into(),
            has_auth: false,
        });
        write_accounts(&root, &accounts).unwrap();
        let rt = resolve_account(&root, Some("abc123".into()))
            .unwrap()
            .unwrap();
        assert_eq!(rt.account_id, "abc123");
        assert_eq!(
            rt.data_dir,
            root.join("accounts").join("abc123").join("data")
        );
        assert_eq!(
            rt.astra_home,
            root.join("accounts").join("abc123").join("astra-home")
        );
    }

    #[test]
    fn account_id_is_random_and_unique() {
        let a = generate_account_id();
        let b = generate_account_id();
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn instance_lock_excludes_second_holder() {
        let root = tmp_root("lock");
        let mut lock = InstanceLock::acquire(&root).unwrap();
        assert!(root.join(".astra-instance.lock").exists());
        lock.release();
        assert!(!root.join(".astra-instance.lock").exists());
        // 释放后可重新获取
        let again = InstanceLock::acquire(&root);
        assert!(again.is_ok());
        again.unwrap().release();
    }

    #[test]
    fn instance_lock_stale_pid_is_reclaimed() {
        let root = tmp_root("stale");
        // 写入一个几乎不可能存活的 pid（Linux pid 上限 4M+ / macOS 同理；
        // 用 u32::MAX 溢出截断后仍判死）。更稳妥：写当前 pid 的文件再手动
        // 模拟陈旧 —— 直接写无效 pid 内容。
        std::fs::write(root.join(".astra-instance.lock"), "999999999").unwrap();
        let mut lock = InstanceLock::acquire(&root).unwrap();
        assert!(root.join(".astra-instance.lock").exists());
        lock.release();
    }

    #[test]
    fn instance_lock_live_foreign_pid_is_rejected() {
        let root = tmp_root("live");
        // spawn 一个真实存活短命进程，其 pid 一定存活
        let mut child = std::process::Command::new(
            #[cfg(unix)]
            "/bin/sleep",
            #[cfg(not(unix))]
            "cmd",
        )
        .args(
            #[cfg(unix)]
            ["0.3"],
            #[cfg(not(unix))]
            ["/c", "ping", "-n", "1", "127.0.0.1"],
        )
        .spawn()
        .unwrap();
        std::fs::write(root.join(".astra-instance.lock"), child.id().to_string()).unwrap();
        let err = InstanceLock::acquire(&root).unwrap_err();
        assert!(err.contains("已在运行"));
        let _ = child.wait();
    }
}
