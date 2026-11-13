//! 跨平台进程辅助：Windows 闪窗抑制、子进程 PATH 补全。
//!
//! Windows 控制台闪窗抑制：
//! GUI 子系统的宿主进程（Tauri 桌面）spawn 控制台子系统程序（`cmd`、
//! `git`、`node`、`npx`、`.cmd` 批处理入口…）时，Windows 默认为子进程
//! 新建控制台窗口，表现为对话框闪一下黑框。加 `CREATE_NO_WINDOW` 创建
//! 标志即可抑制；pipe 重定向与输出采集不受影响。
//!
//! 例外：需要展示 GUI 的启动（编辑器、浏览器）其子进程本身是 GUI 子系统
//! 不弹窗，但 `.cmd` 包装层（code.cmd）会先弹 cmd 黑框，统一抑制。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// PATH 分隔符：Unix 是 `:`，Windows 是 `;`。
const PATH_SEP: char = if cfg!(windows) { ';' } else { ':' };

/// 给 `std::process::Command` 加 Windows `CREATE_NO_WINDOW`；其他平台无操作。
pub(crate) fn hide_window(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

/// 补全继承来的 `PATH`，供所有 spawn 子进程的地方统一使用。
///
/// 打包后的 Tauri 应用由 Finder / 开始菜单启动时拿不到用户 shell 配置，
/// 继承的 `PATH` 里常常没有 `pnpm`、`node`、`fnm`/`nvm` 管理的版本目录。
/// Unix 下把常见工具目录排在继承的 `PATH` 之前，Windows 下继承的 `PATH`
/// 通常完整，原样返回。
pub(crate) fn full_path() -> String {
    let existing = std::env::var("PATH").unwrap_or_default();
    if cfg!(windows) {
        return existing;
    }
    let home = std::env::var("HOME").unwrap_or_default();
    compose_path(&tool_dirs(&home), &existing)
}

/// 常见工具安装目录（Unix 专用）。`home` 为空时不加用户级目录。
fn tool_dirs(home: &str) -> Vec<String> {
    let mut extra: Vec<String> = vec![
        "/usr/local/bin".into(),
        "/opt/homebrew/bin".into(),
        "/opt/homebrew/sbin".into(),
        "/opt/local/bin".into(),
        "/usr/bin".into(),
        "/bin".into(),
    ];

    if home.is_empty() {
        return extra;
    }

    // npm global
    extra.push(format!("{}/.npm-global/bin", home));

    // ===== fnm (Fast Node Manager) =====
    // 现代版 fnm 默认目录: ~/.local/share/fnm/
    // default alias: ~/.local/share/fnm/aliases/default → node-versions/vXX.X.X/installation
    // 传统路径: ~/Library/fnm/
    extra.push(format!("{}/.local/share/fnm/aliases/default/bin", home));
    extra.push(format!("{}/Library/fnm/aliases/default/bin", home));
    // glob 所有版本目录（最新开发的 fallback）
    for base in &[
        format!("{}/.local/share/fnm/node-versions", home),
        format!("{}/Library/fnm/node-versions", home),
    ] {
        if let Ok(entries) = std::fs::read_dir(base) {
            for entry in entries.flatten() {
                let bin = entry.path().join("installation/bin");
                if bin.is_dir() {
                    extra.push(bin.to_string_lossy().to_string());
                }
            }
        }
    }

    // ===== nvm =====
    // nvm 没有 current 软链，需 glob 版本目录
    let nvm_dir_base = format!("{}/.nvm/versions/node", home);
    if let Ok(entries) = std::fs::read_dir(&nvm_dir_base) {
        for entry in entries.flatten() {
            let bin = entry.path().join("bin");
            if bin.is_dir() {
                extra.push(bin.to_string_lossy().to_string());
            }
        }
    }
    // 允许用户自定义 NVM_DIR
    if let Ok(nvm_dir) = std::env::var("NVM_DIR") {
        let nvm_versions = format!("{}/versions/node", nvm_dir);
        if let Ok(entries) = std::fs::read_dir(&nvm_versions) {
            for entry in entries.flatten() {
                let bin = entry.path().join("bin");
                if bin.is_dir() {
                    extra.push(bin.to_string_lossy().to_string());
                }
            }
        }
    }

    // ===== volta =====
    extra.push(format!("{}/.volta/bin", home));

    // ===== pnpm global =====
    extra.push(format!("{}/Library/pnpm", home));

    extra
}

/// 按平台分隔符拼装 `PATH`：`extra` 优先，`existing` 中的目录追加在后。
///
/// 关键点：**不能**把继承来的 `PATH` 整串当成单个元素交给
/// `std::env::join_paths`。该函数见到「元素内含分隔符」会返回 `Err`，
/// 早先的实现用 `unwrap_or_default()` 接住，于是得到**空 `PATH`**；
/// 随后 `Command::new("sh")` 在 `posix_spawnp` 查找阶段就失败，
/// 报 `No such file or directory (os error 2)`——表现出来是「所有命令都启动不了」，
/// 而工作目录明明是存在的，极易误判为路径问题。
///
/// 这里先按分隔符拆开 `existing`，与 `extra` 合并去重（`extra` 顺序优先），
/// 再拼接；万一仍拼不出来（某目录名本身含分隔符），退回 `existing`，
/// 绝不返回空串。
fn compose_path(extra: &[String], existing: &str) -> String {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();

    for dir in extra {
        push_dir(&mut dirs, &mut seen, Path::new(dir));
    }
    for dir in std::env::split_paths(existing) {
        push_dir(&mut dirs, &mut seen, &dir);
    }

    std::env::join_paths(&dirs)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| existing.to_string())
}

/// 过滤空目录与含分隔符的目录（后者无法参与拼接），并去重。
fn push_dir(dirs: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>, raw: &Path) {
    if raw.as_os_str().is_empty() {
        return;
    }
    if raw.to_string_lossy().contains(PATH_SEP) {
        return;
    }
    let dir = raw.to_path_buf();
    if seen.insert(dir.clone()) {
        dirs.push(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 用平台分隔符把目录拼成 PATH 值，避免测试硬编码 `:` / `;`。
    fn join(dirs: &[&str]) -> String {
        std::env::join_paths(dirs.iter().map(Path::new))
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    fn parts(path: &str) -> Vec<PathBuf> {
        std::env::split_paths(path).collect()
    }

    #[test]
    fn compose_path_keeps_inherited_entries_as_separate_dirs() {
        // 回归：整串继承的 PATH 曾被当成单个元素传进 join_paths，
        // join_paths 报错后回落到空串，导致子进程一律 spawn 失败。
        let existing = join(&["/usr/bin", "/bin"]);
        let out = compose_path(&["/opt/homebrew/bin".to_string()], &existing);

        let out_dirs = parts(&out);
        assert!(!out.is_empty(), "PATH 不能为空");
        assert_eq!(out_dirs[0], PathBuf::from("/opt/homebrew/bin"));
        assert_eq!(out_dirs.len(), 3, "继承的两个目录要各自成为独立元素");
        assert!(out_dirs.contains(&PathBuf::from("/usr/bin")));
        assert!(out_dirs.contains(&PathBuf::from("/bin")));
    }

    #[test]
    fn compose_path_dedupes_extra_against_inherited() {
        let existing = join(&["/usr/bin", "/usr/bin", "/bin"]);
        let out = compose_path(&["/usr/bin".to_string()], &existing);
        let out_dirs = parts(&out);
        assert_eq!(
            out_dirs
                .iter()
                .filter(|p| **p == PathBuf::from("/usr/bin"))
                .count(),
            1
        );
        assert_eq!(out_dirs.len(), 2);
    }

    #[test]
    fn compose_path_skips_dirs_containing_separator() {
        let bad = format!("/tmp/a{}b", PATH_SEP);
        let out = compose_path(&[bad], "/usr/bin");
        assert_eq!(parts(&out), vec![PathBuf::from("/usr/bin")]);
    }

    #[test]
    fn compose_path_handles_empty_inherited() {
        let out = compose_path(&["/usr/bin".to_string()], "");
        assert_eq!(parts(&out), vec![PathBuf::from("/usr/bin")]);
    }

    #[test]
    fn full_path_is_never_empty() {
        // 回归：空 PATH 会让所有 `sh -c` 调用在 spawn 阶段报 os error 2
        let out = full_path();
        assert!(!out.is_empty(), "补全后的 PATH 不能为空");
        assert!(!parts(&out).is_empty());
    }
}
