//! P0-4：legacy 单用户数据 → 首个账户的一次性迁移。
//!
//! 触发时机：`account_create` 创建**第一个**账户时执行一次。gate 阻断期间
//! legacy 数据目录无进程写入，迁移窗口安全；迁移只做 rename（同一卷原子），
//! rename 失败回退 copy 并保留原件——任何时刻旧数据都可恢复，支持重跑
//! （源缺失即跳过）。完成后写 marker（APPDATA/.legacy-migration.json），
//! 防止后续账户创建误触发。
//!
//! 迁移范围（见 docs/MULTI_ACCOUNT_INVENTORY.md 归属矩阵）：
//! - APPDATA 根 → 账户 `data/`：settings/providers/threads/sessions/events/
//!   日志/memory/向量索引/generation/下载与 dev 历史（含 sqlite 的 -wal/-shm）
//! - `~/.astra/config.toml` → 账户 `astra-home/`：**copy 保留原件**
//!   （其他工具可能共用该配置，不能搬走）
//! - `~/.astra/pastes`、`~/.config/astra/skills` → 账户 `astra-home/`：
//!   rename 优先，失败回退 copy+保留

use serde::{Deserialize, Serialize};
use std::path::Path;

const MARKER_FILE: &str = ".legacy-migration.json";

/// APPDATA 根下的 legacy 数据文件（agent_threads.sqlite3 由 DB 名单单独补充）。
const DATA_FILES: &[&str] = &[
    "settings.json",
    "providers.json",
    "agent_sessions.json",
    "agent_events.log",
    "astra.log",
    "astra.log.old",
    "memory-cme.sqlite",
    "memory.key",
    "vector_index.db",
    "generation.json",
    "download_history.json",
    "dev_history.json",
];

/// 带 -wal / -shm 伴生文件的 sqlite 库。
const DATA_DBS: &[&str] = &[
    "agent_threads.sqlite3",
    "memory-cme.sqlite",
    "vector_index.db",
];

#[derive(Debug, Serialize, Deserialize)]
struct MigrationMarker {
    status: String,
    account_id: String,
    migrated_at: i64,
}

/// 迁移结果摘要（账户创建命令里记日志用）。
#[derive(Debug, Default, Serialize)]
pub(crate) struct MigrationReport {
    pub moved: Vec<String>,
    pub copied: Vec<String>,
    pub skipped: Vec<String>,
    pub failed: Vec<String>,
}

/// legacy 数据源（拆出以便单测注入临时目录）。
pub(crate) struct LegacySources<'a> {
    /// legacy 数据目录（原 app_data_dir，即 APPDATA 根）
    pub app_data_root: &'a Path,
    /// legacy astra home（config.toml / pastes）
    pub astra_home: &'a Path,
    /// legacy 用户技能目录（~/.config/astra/skills）
    pub skills_dir: &'a Path,
}

/// 当前平台的 legacy 源定位（生产入口；单测直接构造 LegacySources）。
pub(crate) fn migrate_legacy_for_current_platform(
    app_data_root: &Path,
    account_root: &Path,
    account_id: &str,
) -> Result<MigrationReport, String> {
    let astra_home = agent_core::astra_config::resolve_astra_home();
    let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
    let sources = LegacySources {
        app_data_root,
        astra_home: &astra_home,
        skills_dir: &home.join(".config").join("astra").join("skills"),
    };
    migrate_legacy_to_account(&sources, account_root, account_id)
}

/// 已完成过迁移？（防止重复触发）
pub(crate) fn legacy_migration_done(app_data_root: &Path) -> bool {
    marker_path(app_data_root)
        .exists()
        .then(|| {
            std::fs::read_to_string(marker_path(app_data_root))
                .ok()
                .and_then(|raw| serde_json::from_str::<MigrationMarker>(&raw).ok())
                .map(|m| m.status == "done")
        })
        .flatten()
        .unwrap_or(false)
}

fn marker_path(app_data_root: &Path) -> std::path::PathBuf {
    app_data_root.join(MARKER_FILE)
}

/// 执行迁移（幂等：marker 已 done 或源缺失都会安全跳过）。
pub(crate) fn migrate_legacy_to_account(
    sources: &LegacySources<'_>,
    account_root: &Path,
    account_id: &str,
) -> Result<MigrationReport, String> {
    if legacy_migration_done(sources.app_data_root) {
        return Ok(MigrationReport::default());
    }
    let data_dir = account_root.join("data");
    let astra_home = account_root.join("astra-home");
    std::fs::create_dir_all(&data_dir).map_err(|e| format!("创建账户 data 目录失败: {e}"))?;
    std::fs::create_dir_all(&astra_home).map_err(|e| format!("创建账户 astra-home 失败: {e}"))?;

    let mut report = MigrationReport::default();

    // 1. APPDATA 根的数据文件 → 账户 data/
    let mut names: Vec<String> = DATA_FILES.iter().map(|s| s.to_string()).collect();
    for db in DATA_DBS {
        names.push(format!("{db}-wal"));
        names.push(format!("{db}-shm"));
    }
    for name in &names {
        move_file(
            &sources.app_data_root.join(name),
            &data_dir.join(name),
            name,
            &mut report,
        );
    }

    // 2. config.toml：copy 保留原件（其他工具可能共用）
    copy_file(
        &sources.astra_home.join("config.toml"),
        &astra_home.join("config.toml"),
        "config.toml",
        &mut report,
    );

    // 3. pastes / skills 目录：rename 优先，失败回退 copy+保留
    move_dir(
        &sources.astra_home.join("pastes"),
        &astra_home.join("pastes"),
        "pastes",
        &mut report,
    );
    move_dir(
        &sources.skills_dir,
        &astra_home.join("skills"),
        "skills",
        &mut report,
    );

    // 4. 写 marker（无论实际迁了多少——源缺失也是合法终态）
    let marker = MigrationMarker {
        status: "done".into(),
        account_id: account_id.into(),
        migrated_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0),
    };
    let json =
        serde_json::to_string_pretty(&marker).map_err(|e| format!("序列化 marker 失败: {e}"))?;
    std::fs::write(marker_path(sources.app_data_root), json)
        .map_err(|e| format!("写入迁移 marker 失败: {e}"))?;
    Ok(report)
}

/// 单文件搬移：源缺失/目标已存在 → 跳过；rename 失败回退 copy+保留。
fn move_file(src: &Path, dest: &Path, label: &str, report: &mut MigrationReport) {
    if !src.exists() {
        report.skipped.push(label.into());
        return;
    }
    if dest.exists() {
        report.skipped.push(format!("{label} (dest exists)"));
        return;
    }
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::rename(src, dest) {
        Ok(()) => report.moved.push(label.into()),
        Err(_) => match std::fs::copy(src, dest) {
            Ok(_) => report
                .copied
                .push(format!("{label} (rename failed, original kept)")),
            Err(e) => report.failed.push(format!("{label}: {e}")),
        },
    }
}

/// 目录搬移：同 move_file 语义，递归 copy 回退。
fn move_dir(src: &Path, dest: &Path, label: &str, report: &mut MigrationReport) {
    if !src.exists() {
        report.skipped.push(label.into());
        return;
    }
    if dest.exists() {
        report.skipped.push(format!("{label} (dest exists)"));
        return;
    }
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::rename(src, dest) {
        Ok(()) => report.moved.push(label.into()),
        Err(_) => match copy_recursive(src, dest) {
            Ok(()) => report
                .copied
                .push(format!("{label} (rename failed, original kept)")),
            Err(e) => report.failed.push(format!("{label}: {e}")),
        },
    }
}

/// 单文件复制（仅 config.toml 这类必须保留原件的共享配置）。
fn copy_file(src: &Path, dest: &Path, label: &str, report: &mut MigrationReport) {
    if !src.exists() {
        report.skipped.push(label.into());
        return;
    }
    if dest.exists() {
        report.skipped.push(format!("{label} (dest exists)"));
        return;
    }
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::copy(src, dest) {
        Ok(_) => report.copied.push(label.into()),
        Err(e) => report.failed.push(format!("{label}: {e}")),
    }
}

fn copy_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dest)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &dest.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(src, dest).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一套 legacy 数据：APPDATA 文件 + astra-home + skills，执行迁移后
    /// 验证文件归属、marker 与幂等重跑。
    #[test]
    fn legacy_migration_moves_files_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let app_root = tmp.path().join("APPDATA");
        let astra_home = tmp.path().join(".astra");
        let skills_dir = tmp.path().join(".config").join("astra").join("skills");
        let account_root = tmp.path().join("accounts").join("acc1");
        std::fs::create_dir_all(&app_root).unwrap();
        std::fs::create_dir_all(astra_home.join("pastes")).unwrap();
        std::fs::create_dir_all(skills_dir.join("my-skill")).unwrap();

        // 数据文件（含 sqlite 伴生文件）
        std::fs::write(app_root.join("settings.json"), "{}").unwrap();
        std::fs::write(app_root.join("providers.json"), "[]").unwrap();
        std::fs::write(app_root.join("agent_threads.sqlite3"), b"db").unwrap();
        std::fs::write(app_root.join("agent_threads.sqlite3-wal"), b"wal").unwrap();
        std::fs::write(app_root.join("memory.key"), b"key").unwrap();
        // 不存在的文件（memory-cme.sqlite 等）应跳过
        std::fs::write(astra_home.join("config.toml"), "[mcp]").unwrap();
        std::fs::write(astra_home.join("pastes").join("p.png"), b"img").unwrap();
        std::fs::write(skills_dir.join("my-skill").join("SKILL.md"), "# s").unwrap();

        let sources = LegacySources {
            app_data_root: &app_root,
            astra_home: &astra_home,
            skills_dir: &skills_dir,
        };
        let report = migrate_legacy_to_account(&sources, &account_root, "acc1").unwrap();

        // moved：数据文件落到账户 data/
        let data = account_root.join("data");
        assert!(data.join("settings.json").exists());
        assert!(data.join("agent_threads.sqlite3-wal").exists());
        assert!(data.join("memory.key").exists());
        assert!(
            !app_root.join("settings.json").exists(),
            "rename 后源文件应消失"
        );
        // astra-home：config.toml 是 copy（原件保留），pastes/skills 是搬移
        let home = account_root.join("astra-home");
        assert!(home.join("config.toml").exists());
        assert!(
            astra_home.join("config.toml").exists(),
            "config.toml 必须保留原件"
        );
        assert!(home.join("pastes").join("p.png").exists());
        assert!(home
            .join("skills")
            .join("my-skill")
            .join("SKILL.md")
            .exists());
        // 报告
        assert!(report.moved.contains(&"settings.json".to_string()));
        assert!(report.moved.contains(&"pastes".to_string()));
        assert!(report.copied.contains(&"config.toml".to_string()));
        assert!(report
            .skipped
            .iter()
            .any(|s| s.contains("memory-cme.sqlite")));
        assert!(report.failed.is_empty());
        // marker
        assert!(legacy_migration_done(&app_root));
        // 幂等：重跑返回空报告且不再改动
        let again = migrate_legacy_to_account(&sources, &account_root, "acc1").unwrap();
        assert!(again.moved.is_empty() && again.copied.is_empty());
    }

    #[test]
    fn missing_sources_still_completes_with_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let app_root = tmp.path().join("APPDATA");
        let account_root = tmp.path().join("accounts").join("acc1");
        std::fs::create_dir_all(&app_root).unwrap();
        let sources = LegacySources {
            app_data_root: &app_root,
            astra_home: &tmp.path().join(".astra"),
            skills_dir: &tmp.path().join(".config").join("astra").join("skills"),
        };
        let report = migrate_legacy_to_account(&sources, &account_root, "acc1").unwrap();
        assert!(report.moved.is_empty());
        assert!(report.failed.is_empty());
        assert!(legacy_migration_done(&app_root));
    }

    #[test]
    fn existing_dest_files_are_not_overwritten() {
        let tmp = tempfile::tempdir().unwrap();
        let app_root = tmp.path().join("APPDATA");
        let account_root = tmp.path().join("accounts").join("acc1");
        std::fs::create_dir_all(&app_root).unwrap();
        std::fs::create_dir_all(account_root.join("data")).unwrap();
        std::fs::write(app_root.join("providers.json"), "legacy").unwrap();
        std::fs::write(account_root.join("data").join("providers.json"), "account").unwrap();

        let sources = LegacySources {
            app_data_root: &app_root,
            astra_home: &tmp.path().join(".astra"),
            skills_dir: &tmp.path().join(".config").join("astra").join("skills"),
        };
        let report = migrate_legacy_to_account(&sources, &account_root, "acc1").unwrap();
        assert!(report.skipped.iter().any(|s| s.contains("dest exists")));
        let kept =
            std::fs::read_to_string(account_root.join("data").join("providers.json")).unwrap();
        assert_eq!(kept, "account", "账户已有数据绝不能被 legacy 覆盖");
    }
}
