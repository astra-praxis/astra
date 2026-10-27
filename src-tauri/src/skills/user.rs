//! 用户技能库：<账户 astra-home>/skills/<技能名>/ 目录管理。
//!
//! 与 ~/.agents/skills（全局 Git 技能）不同，这里是应用自有的技能库，
//! 供「我的 Skills」页面导入/新建/多文件编辑，并供 Agent 通过
//! list_skills / read_skill 工具发现与读取、通过 /技能名 直接调用。
//!
//! 分层：读路径（摘要/读取/校验）与 Agent 工具共用 agent-core 的
//! skill_tools 实现（re-export，避免双份逻辑漂移）；本文件保留写操作
//! （新建/导入/同步/删除）与目录级元数据（display_name/file_count）。

pub use agent_core::skill_tools::{
    read_skill_content, summary_in, validate_rel_path, validate_skill_name,
};

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::AppHandle;

/// 目录递归统计/列举的条目上限（防异常大目录拖垮 UI）
const WALK_MAX_ENTRIES: usize = 1000;

#[derive(Debug, Serialize)]
pub struct UserSkillMeta {
    /// 规范名 = 目录名（/调用 与 read_skill 都用这个名字）
    pub name: String,
    /// 展示名 = SKILL.md frontmatter 的 name 字段（缺省回退目录名）
    pub display_name: String,
    pub description: String,
    pub path: String,
    pub file_count: usize,
    /// 技能是否可用：目录内存在 SKILL.md（read_skill / /调用 都依赖它）
    pub available: bool,
}

#[derive(Debug, Serialize)]
pub struct UserSkillFile {
    pub rel_path: String,
    pub size: u64,
}

/// Agent 侧的轻量摘要类型由 agent-core::skill_tools 提供（见文件头 re-export）

/// 全局技能同步报告（~/.agents/skills → 应用技能库，覆盖语义）
#[derive(Debug, Serialize)]
pub struct SyncReport {
    /// 本次新导入的技能名
    pub imported: Vec<String>,
    /// 已存在并被全局版本覆盖更新的技能名
    pub updated: Vec<String>,
    /// 跳过的 (技能名, 原因)
    pub skipped: Vec<(String, String)>,
}

// ===== 校验（实现在 agent-core::skill_tools，此处 re-export） =====

// ===== 目录级核心逻辑（可单测） =====

/// 从 SKILL.md frontmatter 提取指定字段（name/description 等），单行、去可选引号
fn parse_frontmatter_field(content: &str, field: &str) -> String {
    let content = content.trim();
    if !content.starts_with("---") {
        return String::new();
    }
    let Some(rest) = content.strip_prefix("---") else {
        return String::new();
    };
    let Some(end) = rest.find("---") else {
        return String::new();
    };
    let prefix = format!("{field}:");
    for line in rest[..end].lines() {
        if let Some(v) = line.trim().strip_prefix(&prefix) {
            return v.trim().trim_matches('"').trim_matches('\'').to_string();
        }
    }
    String::new()
}

fn parse_description(content: &str) -> String {
    parse_frontmatter_field(content, "description")
}

fn skill_description(skill_dir: &Path) -> String {
    fs::read_to_string(skill_dir.join("SKILL.md"))
        .map(|c| parse_description(&c))
        .unwrap_or_default()
}

/// SKILL.md frontmatter 的 name 字段（可选；展示用，规范名始终是目录名）
fn skill_frontmatter_name(skill_dir: &Path) -> Option<String> {
    let n = fs::read_to_string(skill_dir.join("SKILL.md"))
        .map(|c| parse_frontmatter_field(&c, "name"))
        .unwrap_or_default();
    if n.is_empty() {
        None
    } else {
        Some(n)
    }
}

/// 递归统计技能内文件数（跳过隐藏目录与 .git）
fn count_files(dir: &Path, budget: &mut usize) -> usize {
    let mut count = 0;
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    for entry in entries.flatten() {
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        let p = entry.path();
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        if p.is_dir() {
            count += count_files(&p, budget);
        } else {
            count += 1;
        }
    }
    count
}

/// 递归列举技能内文件（相对路径，跳过隐藏与 .git，按路径排序，封顶 WALK_MAX_ENTRIES）
fn walk_files(dir: &Path, rel_prefix: &str, out: &mut Vec<UserSkillFile>) {
    if out.len() >= WALK_MAX_ENTRIES {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with('.') {
                return None;
            }
            Some((n, e.path()))
        })
        .collect();
    names.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, path) in names {
        if out.len() >= WALK_MAX_ENTRIES {
            return;
        }
        let rel = if rel_prefix.is_empty() {
            name.clone()
        } else {
            format!("{rel_prefix}/{name}")
        };
        if path.is_dir() {
            walk_files(&path, &rel, out);
        } else if let Ok(meta) = fs::metadata(&path) {
            out.push(UserSkillFile {
                rel_path: rel,
                size: meta.len(),
            });
        }
    }
}

/// 列出技能库目录下的全部技能
pub fn list_in(dir: &Path) -> Vec<UserSkillMeta> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut skills: Vec<UserSkillMeta> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if !path.is_dir() {
                return None;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                return None;
            }
            let mut budget = WALK_MAX_ENTRIES;
            Some(UserSkillMeta {
                display_name: skill_frontmatter_name(&path).unwrap_or_else(|| name.clone()),
                description: skill_description(&path),
                path: path.to_string_lossy().to_string(),
                file_count: count_files(&path, &mut budget),
                available: path.join("SKILL.md").is_file(),
                name,
            })
        })
        .collect();
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

/// 新建技能：创建目录 + SKILL.md 脚手架
pub fn create_skill_in(dir: &Path, name: &str, description: &str) -> Result<(), String> {
    validate_skill_name(name)?;
    let target = dir.join(name.trim());
    if target.exists() {
        return Err(format!("技能 '{name}' 已存在"));
    }
    fs::create_dir_all(&target).map_err(|e| format!("创建技能目录失败: {e}"))?;
    let desc = description.trim();
    let md = format!(
        "---\nname: {name}\ndescription: {desc}\n---\n\n# {name}\n\n（在这里描述这个技能的用途、输入与执行步骤）\n"
    );
    fs::write(target.join("SKILL.md"), md).map_err(|e| format!("写入 SKILL.md 失败: {e}"))?;
    Ok(())
}

/// 递归复制目录（跳过 .git）
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("创建目录失败: {e}"))?;
    let entries = fs::read_dir(src).map_err(|e| format!("读取源目录失败: {e}"))?;
    for entry in entries.flatten() {
        let sp = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy() == ".git" {
            continue;
        }
        let dp = dst.join(&name);
        if sp.is_dir() {
            copy_dir_recursive(&sp, &dp)?;
        } else {
            fs::copy(&sp, &dp).map_err(|e| format!("复制 {} 失败: {e}", name.to_string_lossy()))?;
        }
    }
    Ok(())
}

/// 导入技能：把外部目录整体复制进技能库（重名报错；缺 SKILL.md 时补脚手架）
pub fn import_skill_into(dir: &Path, source: &Path) -> Result<String, String> {
    if !source.is_dir() {
        return Err("源目录不存在或不是目录".into());
    }
    let raw = source
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    validate_skill_name(&raw)?;
    let name = raw.trim().to_string();
    let target = dir.join(&name);
    if target.exists() {
        return Err(format!("技能 '{name}' 已存在，请先重命名源文件夹"));
    }
    copy_dir_recursive(source, &target)?;
    // 无 SKILL.md 的目录导入后补一个脚手架，保证 / 调用与 AI 读取都有说明入口
    if !target.join("SKILL.md").exists() {
        let md = format!(
            "---\nname: {name}\ndescription: （导入的技能，请补充描述）\n---\n\n# {name}\n"
        );
        let _ = fs::write(target.join("SKILL.md"), md);
    }
    Ok(name)
}

/// 同步全局技能到技能库：~/.agents/skills 下每个含 SKILL.md 的目录
/// 复制进 lib_dir。已存在的先删后拷（以全局为准覆盖），不存在的新增。
/// 单个技能失败不中断整体，记入 skipped。
pub fn sync_global_skills_into(lib_dir: &Path, global_dir: &Path) -> Result<SyncReport, String> {
    if !global_dir.is_dir() {
        return Err("全局技能目录不存在或不是目录".into());
    }
    let mut report = SyncReport {
        imported: Vec::new(),
        updated: Vec::new(),
        skipped: Vec::new(),
    };
    let entries = fs::read_dir(global_dir).map_err(|e| format!("读取全局技能目录失败: {e}"))?;
    for entry in entries.flatten() {
        let src = entry.path();
        let raw = entry.file_name().to_string_lossy().to_string();
        // 只处理含 SKILL.md 的目录；隐藏目录/散文件一律跳过
        if !src.is_dir() || !src.join("SKILL.md").is_file() {
            continue;
        }
        if let Err(e) = validate_skill_name(&raw) {
            report.skipped.push((raw, e));
            continue;
        }
        let name = raw.trim().to_string();
        let target = lib_dir.join(&name);
        let existed = target.exists();
        // 覆盖前必须先删干净，否则全局侧已删除的文件会残留在旧副本里
        if existed {
            if let Err(e) = fs::remove_dir_all(&target) {
                report
                    .skipped
                    .push((name.clone(), format!("清理旧版本失败: {e}")));
                continue;
            }
        }
        match copy_dir_recursive(&src, &target) {
            Ok(()) => {
                if existed {
                    report.updated.push(name);
                } else {
                    report.imported.push(name);
                }
            }
            Err(e) => {
                report.skipped.push((name, e));
            }
        }
    }
    Ok(report)
}

/// 写入技能内文件（父目录自动创建）
pub fn write_skill_file(dir: &Path, name: &str, rel: &str, content: &str) -> Result<(), String> {
    validate_skill_name(name)?;
    validate_rel_path(rel)?;
    let skill_dir = dir.join(name.trim());
    if !skill_dir.is_dir() {
        return Err(format!("技能 '{name}' 不存在"));
    }
    let full = agent_core::ensure_within_root(&skill_dir, Path::new(rel))
        .map_err(|e| format!("路径校验失败: {e}"))?;
    let target = skill_dir.join(full);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    fs::write(target, content).map_err(|e| format!("写入文件失败: {e}"))
}

/// 删除技能内单个文件
pub fn delete_skill_file(dir: &Path, name: &str, rel: &str) -> Result<(), String> {
    validate_skill_name(name)?;
    validate_rel_path(rel)?;
    let skill_dir = dir.join(name.trim());
    if !skill_dir.is_dir() {
        return Err(format!("技能 '{name}' 不存在"));
    }
    let full = agent_core::ensure_within_root(&skill_dir, Path::new(rel))
        .map_err(|e| format!("路径校验失败: {e}"))?;
    let target = skill_dir.join(full);
    if !target.is_file() {
        return Err(format!("文件不存在: {rel}"));
    }
    fs::remove_file(target).map_err(|e| format!("删除文件失败: {e}"))
}

/// 删除整个技能目录
pub fn delete_skill_dir(dir: &Path, name: &str) -> Result<(), String> {
    validate_skill_name(name)?;
    let target = dir.join(name.trim());
    if !target.is_dir() {
        return Err(format!("技能 '{name}' 不存在"));
    }
    fs::remove_dir_all(target).map_err(|e| format!("删除技能失败: {e}"))
}

// ===== Tauri 命令薄包装 =====

/// 应用技能库目录（账户模式为 accounts/<id>/astra-home/skills，
/// legacy 回退 ~/.config/astra/skills；不存在则创建）。
/// 用户技能是用户自持内容，与 ~/.agents/skills（全局 Git 技能）心智模型一致；
/// agent.rs 也用它来注册技能发现/读取工具。
pub fn app_skills_dir(_app: &AppHandle) -> Result<PathBuf, String> {
    // 账户模式下为 accounts/<id>/astra-home/skills（用户技能随账户隔离）；
    // legacy 模式保持 ~/.config/astra/skills 旧位置不变
    let dir = if let Some(rt) = crate::app::accounts::active() {
        rt.astra_home.join("skills")
    } else {
        let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
        home.join(".config").join("astra").join("skills")
    };
    fs::create_dir_all(&dir).map_err(|e| format!("创建技能库目录失败: {e}"))?;
    Ok(dir)
}

#[tauri::command]
pub fn list_user_skills(app: AppHandle) -> Result<Vec<UserSkillMeta>, String> {
    Ok(list_in(&app_skills_dir(&app)?))
}

// ===== 内置技能（随应用分发） =====

/// 内置记忆技能：让 agent 知道自己拥有跨会话记忆能力，并提供三个工具
/// （memory_remember / memory_search / memory_forget）的使用时机与约束。
/// 目录名避开常用名，降低被 sync_global_skills 同名覆盖的概率。
const BUILTIN_MEMORY_SKILL_NAME: &str = "astra-memory";
const BUILTIN_MEMORY_SKILL_MD: &str = r#"---
name: astra-memory
description: Astra 跨会话记忆系统的使用指南。说明自动记忆如何工作，以及 memory_remember（主动记住）、memory_search（主动查记忆）与 memory_forget（遗忘过时记忆）三个工具何时用、怎么用。当用户要求记住/查找/忘记信息、询问之前的决定/偏好/排查经验，或需要判断哪些信息值得长期保存时使用本 skill。
---

# Astra 跨会话记忆

你拥有跨会话记忆能力，分两部分：

## 自动记忆（无需操作）

每次对话结束后，系统会自动把当前会话压缩为摘要并提炼长期记忆
（用户偏好、确认过的决策、经工具验证的事实、排查经验）。下一次对话开始
前，相关记忆会自动注入你的上下文（REMEMBERED CONTEXT 区块）。

- 不要在对话里主动复述这个机制，除非用户问起。
- REMEMBERED CONTEXT 里可能有「历史版本/已被取代」标记的记忆：带 superseded
  标记的不是当前事实，不要当作现行结论使用。

## 主动记忆工具

### memory_remember：立即写入一条长期记忆

**该用**：
- 用户明确要求记住：「记住：以后都用 pnpm」「这个标书以后都按这个格式」
- 对话中确认的决策：「那就决定用 SQLite 了」——用户表现出就此定论的语气时
- 经工具验证的环境事实：实际执行确认过的路径、版本、配置
- 排查后确认的问题原因与修复方案（记「原因 + 修复方法 + 验证时间」，
  不只记结论）

**不该用**：
- 闲聊、寒暄、情绪表达
- 一次性任务（「帮我改下这行代码」）
- 普通知识问答（答案本身没有用户/项目特殊性）
- 尚未验证的猜测——工具还没跑过、用户还没确认的事，不要写成 fact

**content 写法**：一条完整、自包含的话。写清主体与范围（哪个项目、什么
环境），不要用代词指代。对比：
- 差：「路径在 /opt/homebrew/bin」（哪个工具的路径？）
- 好：「astra 项目的 pandoc 路径是 /opt/homebrew/bin/pandoc（2026-10 验证）」

**kind 选择**：preference=用户偏好（默认）｜decision=已确认决策｜
fact=环境/项目事实｜experience=经验/解决方案｜task=待续任务。

注意：写入即生效并永久保存；同一内容重复提交只会强化，不会产生重复。

**复杂经验的内容分层**（经验含多个可复用点时，一条记忆内用固定行结构，
便于检索与按需展开）：

```
标题：DOCX 图片 OCR — macOS（一句话，含实体词，便于检索）
摘要：适用场景与总体策略（一两行）
执行内容：具体命令、参数、限制、失败恢复方法
```

正文首行即标题行。检索命中的是标题与摘要，需要执行细节时 agent
再读全文——不要把命令埋在叙事段落里。

### memory_forget：让过时记忆退出召回

**该用**：
- 工具路径/环境已变更，旧 fact 不再成立（新事实已写入后，旧的应退出）
- 决策被用户明确推翻，旧 decision 仍会被自动注入时
- 用户明确说「别再提这个了」「把这个忘掉」

**用法**：先用 memory_search 拿到记忆 id，再传给 memory_forget。

**不该用**：
- 不确定是否真的过时——先向用户确认，不要凭猜测遗忘
- 只是因为检索结果不顺眼——遗忘是不可轻易逆转的上下文变更

**边界**：工具只能逻辑遗忘（立即退出召回，历史行保留供审计）。
彻底删除（连历史一起清除）只能在用户设置界面操作，用户要求时如实说明。
遗忘后可用 memory_search 确认已搜不到。

### memory_search：主动查记忆

**该用**：用户提到「上次/之前/我们之前决定的/我记得好像」且对话上下文里
没有答案时，先查再答，不要凭空猜测。

**query 写法**：必须包含具体实体词（项目名、工具名、路径片段等）。记忆
检索是词法匹配，纯中文问句（如「上次那个问题怎么办」）会检索不到；多给
几个关键词（工具名 + 项目名）。

**结果为空**：如实告知没有找到相关记忆，可以换实体词重试一次；不要编造。

## 作用域

- 项目记忆只在本项目的会话中可见（以项目根目录名隔离）。
- 没有打开项目时（纯聊天），只能读写全局记忆（用户偏好等）。
- 在无项目会话里搜不到某个项目的事实是正常隔离行为，不是记忆丢失。
- 用户偏好类（如语言、格式习惯）写全局即可；项目相关事实/决策会自动
  绑定当前项目。
"#;

/// 内置找装技能：改写自 Zed 的 find-skills（skills.sh 生态），让 agent 在
/// 对话里帮用户搜技能/装技能时走 GitHub API + tarball 快速通道，
/// 避免 fetch 抓网页 / git clone 整仓导致的长时间等待。
///
/// 安装目标路径不是常量：技能库目录随账户隔离（账户模式在
/// accounts/<id>/astra-home/skills），写入时用 {SKILLS_DIR} 占位符替换为
/// 当前实际目录，避免硬编码 legacy 路径把技能装到读不到的地方。
const BUILTIN_FIND_SKILLS_NAME: &str = "astra-find-skills";
const BUILTIN_FIND_SKILLS_MD_TEMPLATE: &str = r#"---
name: astra-find-skills
description: 技能搜索与安装指南。当用户想找技能或装技能（「帮我装个 xx 技能」「有没有处理 xx 的技能」）时使用本 skill：如何搜索开源技能生态、如何用 tarball 快速安装进本机技能库、装完如何验证与生效。禁止用 git clone 整仓或逐文件抓取的方式安装。
---

# 查找与安装技能

用户说「帮我装个 xx 技能」「有没有 xx 的技能」时，按本说明操作。核心原则：
**走 GitHub API + tarball 快速通道。不要 git clone 整仓，不要用 fetch 抓
github.com 网页（会拿到海量 HTML 且常被 Cloudflare 403 拦截），更不要逐个
文件下载再写入。**

前提：安装需要 shell 工具（curl / tar）。未打开项目时 shell 工具不可用——
引导用户先打开一个项目再说一句「继续装」，或让用户在「我的技能」页手动导入。

本机技能库目录（安装目标，已按当前账户解析）：

```
{SKILLS_DIR}
```

## 第一步：搜索候选

用 fetch 调 GitHub API（返回 JSON，快且稳定）：

```
https://api.github.com/search/repositories?q=<关键词>+topic:agent-skills&sort=stars&per_page=5
```

- 关键词用英文实体词（react、docx、pdf、deploy…），多词用 + 连接
- 该 topic 搜不到合适结果时，去掉 topic 限定再试，或先用 search_web
  搜「skills.sh <关键词>」定位到对应 GitHub 仓库
- 已知可信来源优先：anthropics/skills、vercel-labs/agent-skills、
  ComposioHQ/awesome-claude-skills；目录站：https://skills.sh/

## 第二步：向用户展示候选

安装前先给用户过目：技能名 + 一句话作用 + 来源（owner/repo）+ 星数
（GitHub API 结果里自带 stargazers_count）。只推荐可信来源；星数低或来源
不明的要注明风险。用户确认后再装。

## 第三步：安装（tarball 快速通道）

1. 拿默认分支（fetch）：`https://api.github.com/repos/<owner>/<repo>` →
   读 `default_branch` 字段
2. 下载并解包（shell 一条命令）：

   ```
   curl -sL https://codeload.github.com/<owner>/<repo>/tar.gz/refs/heads/<branch> -o /tmp/astra-skill.tgz && mkdir -p /tmp/astra-skill-src && tar -xzf /tmp/astra-skill.tgz -C /tmp/astra-skill-src
   ```

3. 在解包目录里找到含 SKILL.md 的技能子目录（有的仓库根目录就是一个
   技能，有的集中在 skills/ 之类子目录下），用 shell cat 看它的 SKILL.md
   frontmatter，拿到 name 与 description
4. 拷进技能库（目标目录名 = frontmatter 里的 name）：

   ```
   cp -R /tmp/astra-skill-src/<repo>-<branch>/<技能子目录> {SKILLS_DIR}/<name>
   ```

   目标已存在同名技能时，先问用户是否覆盖，不要直接覆盖。技能目录必须
   直接位于技能库目录下且根层有 SKILL.md，不要多套一层目录。
5. 验证：调 list_skills 确认新技能已出现
6. 清理：`rm -rf /tmp/astra-skill.tgz /tmp/astra-skill-src`

备选：机器上有 node 时可用 `npx skills add <owner>/<repo>@<技能名> -g -y`
（装到 ~/.agents/skills），装完同样要复制进上面的技能库目录才生效。

## 注意

- 涉及项目外路径（/tmp、技能库目录）的命令会弹用户确认，属正常安全流程，
  向用户说明即可，不要试图绕过。
- 技能摘要是在会话创建时注入的：当前会话里用 read_skill 读出新装技能的
  说明并直接照做即可；**/技能名 直接调用要新开会话才能看到**，装完告知
  用户这一点。
- 提醒用户可在「我的技能」页查看与编辑刚装的技能。
"#;

/// 用当前技能库目录渲染内置找装技能正文。
fn builtin_find_skills_md(skills_dir: &Path) -> String {
    BUILTIN_FIND_SKILLS_MD_TEMPLATE.replace("{SKILLS_DIR}", &skills_dir.to_string_lossy())
}

/// 启动时确保内置技能存在：技能库中没有同名技能时写入；已存在则不动
/// （用户可能已自行修改，不覆盖）。best-effort：失败只影响技能提示，
/// 由调用方记日志。
pub fn ensure_builtin_skills(app: &AppHandle) -> Result<(), String> {
    let dir = app_skills_dir(app)?;
    ensure_builtin_skills_into(&dir)
}

/// ensure_builtin_skills 的可测内核：逐个写入缺失的内置技能，
/// 已有同名技能一律不动（用户可能已自行修改）。
fn ensure_builtin_skills_into(dir: &Path) -> Result<(), String> {
    let find_skills_md = builtin_find_skills_md(dir);
    for (name, md) in [
        (BUILTIN_MEMORY_SKILL_NAME, BUILTIN_MEMORY_SKILL_MD),
        (BUILTIN_FIND_SKILLS_NAME, find_skills_md.as_str()),
    ] {
        let skill_dir = dir.join(name);
        if skill_dir.join("SKILL.md").is_file() {
            continue;
        }
        fs::create_dir_all(&skill_dir).map_err(|e| format!("创建内置技能目录失败: {e}"))?;
        fs::write(skill_dir.join("SKILL.md"), md).map_err(|e| format!("写入内置技能失败: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub fn create_user_skill(
    app: AppHandle,
    name: String,
    description: String,
) -> Result<UserSkillMeta, String> {
    let dir = app_skills_dir(&app)?;
    create_skill_in(&dir, &name, &description)?;
    let path = dir.join(name.trim());
    let mut budget = WALK_MAX_ENTRIES;
    Ok(UserSkillMeta {
        display_name: name.trim().to_string(),
        description: description.trim().to_string(),
        path: path.to_string_lossy().to_string(),
        file_count: count_files(&path, &mut budget),
        available: path.join("SKILL.md").is_file(),
        name: name.trim().to_string(),
    })
}

#[tauri::command]
pub fn import_user_skill(app: AppHandle, source: String) -> Result<UserSkillMeta, String> {
    let dir = app_skills_dir(&app)?;
    let name = import_skill_into(&dir, Path::new(&source))?;
    let path = dir.join(&name);
    let mut budget = WALK_MAX_ENTRIES;
    Ok(UserSkillMeta {
        display_name: skill_frontmatter_name(&path).unwrap_or_else(|| name.clone()),
        description: skill_description(&path),
        path: path.to_string_lossy().to_string(),
        file_count: count_files(&path, &mut budget),
        available: path.join("SKILL.md").is_file(),
        name,
    })
}

/// 全局技能目录（Zed/agent 通用 ~/.agents/skills）
fn global_skills_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
    Ok(home.join(".agents").join("skills"))
}

#[tauri::command]
pub fn sync_global_skills(app: AppHandle) -> Result<SyncReport, String> {
    let global = global_skills_dir()?;
    if !global.is_dir() {
        return Err("未找到全局技能目录 ~/.agents/skills".into());
    }
    sync_global_skills_into(&app_skills_dir(&app)?, &global)
}

#[tauri::command]
pub fn list_user_skill_files(app: AppHandle, name: String) -> Result<Vec<UserSkillFile>, String> {
    let dir = app_skills_dir(&app)?;
    validate_skill_name(&name)?;
    let skill_dir = dir.join(name.trim());
    if !skill_dir.is_dir() {
        return Err(format!("技能 '{name}' 不存在"));
    }
    let mut files = Vec::new();
    walk_files(&skill_dir, "", &mut files);
    Ok(files)
}

#[tauri::command]
pub fn read_user_skill_file(
    app: AppHandle,
    name: String,
    rel_path: String,
) -> Result<String, String> {
    read_skill_content(&app_skills_dir(&app)?, &name, Some(&rel_path))
}

#[tauri::command]
pub fn write_user_skill_file(
    app: AppHandle,
    name: String,
    rel_path: String,
    content: String,
) -> Result<(), String> {
    write_skill_file(&app_skills_dir(&app)?, &name, &rel_path, &content)
}

#[tauri::command]
pub fn delete_user_skill_file(
    app: AppHandle,
    name: String,
    rel_path: String,
) -> Result<(), String> {
    delete_skill_file(&app_skills_dir(&app)?, &name, &rel_path)
}

#[tauri::command]
pub fn delete_user_skill(app: AppHandle, name: String) -> Result<(), String> {
    delete_skill_dir(&app_skills_dir(&app)?, &name)
}

// ===== 测试 =====

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("astra-user-skills-{label}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn ensure_builtin_skills_writes_all_and_never_overwrites() {
        let root = temp_root("ensure-builtin");
        ensure_builtin_skills_into(&root).unwrap();
        for name in [BUILTIN_MEMORY_SKILL_NAME, BUILTIN_FIND_SKILLS_NAME] {
            assert!(root.join(name).join("SKILL.md").is_file(), "{name} missing");
        }
        // 已存在的内置技能不覆盖（用户可能已自行修改）
        let p = root.join(BUILTIN_MEMORY_SKILL_NAME).join("SKILL.md");
        fs::write(&p, "user-edited").unwrap();
        ensure_builtin_skills_into(&root).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "user-edited");
    }

    #[test]
    fn skill_name_validation() {
        assert!(validate_skill_name("my-skill").is_ok());
        assert!(validate_skill_name("中文技能").is_ok());
        assert!(validate_skill_name("").is_err());
        assert!(validate_skill_name("  ").is_err());
        assert!(validate_skill_name(".hidden").is_err());
        assert!(validate_skill_name("..").is_err());
        assert!(validate_skill_name("a/b").is_err());
        assert!(validate_skill_name("a\\b").is_err());
    }

    #[test]
    fn rel_path_validation() {
        assert!(validate_rel_path("SKILL.md").is_ok());
        assert!(validate_rel_path("docs/guide.md").is_ok());
        assert!(validate_rel_path("").is_err());
        assert!(validate_rel_path("  ").is_err());
        assert!(validate_rel_path("/abs.md").is_err());
        assert!(validate_rel_path("a/../b.md").is_err());
        assert!(validate_rel_path("..\\x").is_err());
    }

    #[test]
    fn create_and_list_roundtrip() {
        let root = temp_root("create-list");
        create_skill_in(&root, "demo", "演示技能").unwrap();
        // 重名拒绝
        assert!(create_skill_in(&root, "demo", "x").is_err());

        let list = list_in(&root);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "demo");
        assert_eq!(list[0].description, "演示技能");
        assert_eq!(list[0].file_count, 1);

        // SKILL.md 脚手架可被 read_skill_content 读回
        let content = read_skill_content(&root, "demo", None).unwrap();
        assert!(content.contains("description: 演示技能"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn import_copies_and_scaffolds() {
        let root = temp_root("import");
        // 源目录必须在技能库之外（导入源在库内会与目标同名冲突）
        let src_root = temp_root("import-src");
        let src = src_root.join("src-skill");
        fs::create_dir_all(src.join("assets")).unwrap();
        fs::write(src.join("assets/a.txt"), "A").unwrap();
        fs::write(src.join("guide.md"), "# guide").unwrap();
        // .git 应被跳过
        fs::create_dir_all(src.join(".git")).unwrap();
        fs::write(src.join(".git/config"), "x").unwrap();

        let name = import_skill_into(&root, &src).unwrap();
        assert_eq!(name, "src-skill");
        let list = list_in(&root);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].file_count, 3); // assets/a.txt + guide.md + 补的 SKILL.md，.git 被跳过
                                           // 缺 SKILL.md → 补脚手架
        assert!(root.join("src-skill/SKILL.md").exists());

        // 重名导入拒绝
        assert!(import_skill_into(&root, &src).is_err());
        // 源不存在拒绝
        assert!(import_skill_into(&root, &src_root.join("nope")).is_err());
        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&src_root).unwrap();
    }

    #[test]
    fn read_write_delete_skill_file() {
        let root = temp_root("rw");
        create_skill_in(&root, "demo", "d").unwrap();

        write_skill_file(&root, "demo", "docs/note.md", "hello").unwrap();
        assert_eq!(
            read_skill_content(&root, "demo", Some("docs/note.md")).unwrap(),
            "hello"
        );
        let mut files = Vec::new();
        walk_files(&root.join("demo"), "", &mut files);
        assert_eq!(files.len(), 2);

        // 路径逃逸全部拒绝
        assert!(read_skill_content(&root, "demo", Some("../other/SKILL.md")).is_err());
        assert!(write_skill_file(&root, "demo", "/abs.md", "x").is_err());
        assert!(delete_skill_file(&root, "demo", "../x").is_err());
        // 未知技能拒绝
        assert!(read_skill_content(&root, "ghost", None).is_err());

        delete_skill_file(&root, "demo", "docs/note.md").unwrap();
        assert!(delete_skill_file(&root, "demo", "docs/note.md").is_err());

        delete_skill_dir(&root, "demo").unwrap();
        assert!(list_in(&root).is_empty());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn description_parser() {
        let md = "---\nname: x\ndescription: \"带引号的描述\"\n---\n\nbody";
        assert_eq!(parse_description(md), "带引号的描述");
        assert_eq!(parse_frontmatter_field(md, "name"), "x");
        assert_eq!(parse_frontmatter_field("no frontmatter", "name"), "");
        assert_eq!(parse_frontmatter_field("---\nbroken", "name"), "");
    }

    #[test]
    fn display_name_falls_back_to_dir_name() {
        let root = temp_root("display-name");
        // 有 frontmatter name
        let dir = root.join("dir-a");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            "---\nname: 标书撰写\ndescription: d\n---\n",
        )
        .unwrap();
        assert_eq!(skill_frontmatter_name(&dir), Some("标书撰写".to_string()));
        let list = list_in(&root);
        assert_eq!(list[0].name, "dir-a");
        assert_eq!(list[0].display_name, "标书撰写");
        // 无 frontmatter name → 回退目录名
        let dir_b = root.join("dir-b");
        fs::create_dir_all(&dir_b).unwrap();
        fs::write(dir_b.join("SKILL.md"), "# 无 frontmatter\n").unwrap();
        let list = list_in(&root);
        let b = list.iter().find(|s| s.name == "dir-b").unwrap();
        assert_eq!(b.display_name, "dir-b");
        fs::remove_dir_all(&root).unwrap();
    }

    fn make_global_skill(global: &Path, name: &str, desc: &str, extra: &[&str]) {
        let dir = global.join(name);
        fs::create_dir_all(&dir).unwrap();
        let md = format!("---\nname: {name}\ndescription: {desc}\n---\n\n# {name}\n");
        fs::write(dir.join("SKILL.md"), md).unwrap();
        for f in extra {
            let fp = dir.join(f);
            fs::create_dir_all(fp.parent().unwrap()).unwrap();
            fs::write(fp, "data").unwrap();
        }
    }

    #[test]
    fn sync_global_skills_imports_updates_and_skips() {
        let lib = temp_root("sync-lib");
        let global = temp_root("sync-global");

        make_global_skill(&global, "skill-a", "A 的描述", &["docs/guide.md"]);
        make_global_skill(&global, "skill-b", "B 的描述", &[]);
        // 无 SKILL.md 的目录静默跳过；. 开头目录会被名称校验拦下进 skipped
        fs::create_dir_all(global.join("no-md")).unwrap();
        fs::create_dir_all(global.join(".hidden")).unwrap();
        fs::write(
            global.join(".hidden/SKILL.md"),
            "---\nname: .hidden\ndescription: x\n---\n",
        )
        .unwrap();

        // 第一次同步：全部导入
        let report = sync_global_skills_into(&lib, &global).unwrap();
        assert_eq!(report.imported.len(), 2);
        assert!(report.imported.contains(&"skill-a".to_string()));
        assert!(report.updated.is_empty());
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].0, ".hidden");
        assert_eq!(list_in(&lib).len(), 2);
        assert!(lib.join("skill-a/docs/guide.md").exists());

        // 全局侧修改 skill-b 内容并新增文件；删除 skill-a 的一个文件 → 覆盖更新，删除同步生效
        fs::write(
            global.join("skill-b/SKILL.md"),
            "---\nname: skill-b\ndescription: B 新版\n---\n",
        )
        .unwrap();
        fs::write(global.join("skill-b/NEW.md"), "new").unwrap();
        fs::remove_file(global.join("skill-a/docs/guide.md")).unwrap();
        let report = sync_global_skills_into(&lib, &global).unwrap();
        assert!(report.imported.is_empty());
        assert_eq!(report.updated.len(), 2);
        assert!(report.updated.contains(&"skill-b".to_string()));
        assert!(!lib.join("skill-a/docs/guide.md").exists());
        assert!(lib.join("skill-b/NEW.md").exists());
        let list = list_in(&lib);
        let b = list.iter().find(|s| s.name == "skill-b").unwrap();
        assert_eq!(b.description, "B 新版");

        // 全局目录不存在报错
        assert!(sync_global_skills_into(&lib, &global.join("nope")).is_err());

        fs::remove_dir_all(&lib).unwrap();
        fs::remove_dir_all(&global).unwrap();
    }
}
