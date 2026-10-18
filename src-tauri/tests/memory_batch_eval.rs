//! Batch conversation evaluation of the memory feature (formation + recall).
//! Temp encrypted store — the real app memory is never touched.
//!
//! Phase A: feed ~18 conversation utterances (explicit + implicit) through
//! engine.observe with the real OnnxJudge — measures what formation keeps.
//! Phase B: ~22 labeled recall probes (verbatim / paraphrase / unrelated /
//! contradiction-update) through the embedding channel — top-1 accuracy on
//! positives, false positives on negatives, plus hint() gate sanity.
//!
//! Run: MEMORY_EVAL=1 cargo test --test memory_batch_eval -- --ignored --nocapture
//! Env: ASTRA_MEMORY_MODEL_DIR, BGE_MODEL_DIR, ASTRA_MEMORY_ORT_DYLIB

use std::sync::Arc;

use agent_core::memory::MemoryProvider;
use astra_memory::decision::DecisionModel;
use astra_memory::embed::{BgeEmbedder, Embedder};
use astra_memory::engine::MemoryEngine;
use astra_memory::judge_onnx::{OnnxJudge, OnnxJudgeConfig};
use astra_memory::model::SubjectRef;
use astra_memory::protocol::RecallQuery;
use astra_memory::store_sqlite::SqliteStore;
use astra_memory_provider::AstraMemoryProvider;

const NOW: u64 = 1_800_000_000;

/// (utterance, note) — what a user might actually say in conversation.
const SEEDS: &[(&str, &str)] = &[
    ("记住我喜欢深色主题", "显式·偏好"),
    ("我平时写代码主要用 Rust", "隐式·事实"),
    ("我在做一个叫 astra 的桌面 AI 助手项目", "隐式·项目"),
    ("以后回答尽量简短一点，别写大段文字", "隐式·偏好"),
    ("我叫韩斌", "隐式·事实"),
    ("我对花生过敏，千万别给我推荐含花生的东西", "隐式·健康事实"),
    ("我的显卡是 3060Ti，32G 内存", "隐式·硬件"),
    ("别自动帮我提交代码，commit 之前必须问我", "隐式·偏好"),
    ("我打算下个月把 astra 打包到 Windows 上", "隐式·目标"),
    ("记住我用 vim 键位，不要给我 emacs 建议", "显式·偏好"),
    ("这个项目必须支持内网离线运行", "隐式·项目约束"),
    ("我女朋友叫小雨，她生日在三月", "隐式·关系"),
    // ---- Cognitive Layer v1 seeds: second project + attribution + reversal
    (
        "我在做一个叫 campus-ai 的校园 AI 入口平台",
        "隐式·项目·campus-ai",
    ),
    (
        "campus-ai 用 Rust Gateway 做安全决策引擎",
        "隐式·项目上下文·campus-ai",
    ),
    ("astra 用 rmcp 实现 MCP client", "隐式·项目上下文·astra"),
    (
        "campus-ai 不用 Rust Gateway 了，安全决策引擎改成 rmcp",
        "矛盾更新·campus-ai",
    ),
    ("帮我打开 README", "瞬态·应被 observer 丢弃"),
    ("把光标移到行尾", "瞬态·应被 observer 丢弃"),
];

/// (probe, expected top-1 keyword or None for must-not-hit)
const PROBES: &[(&str, Option<&str>)] = &[
    ("我喜欢什么主题？", Some("深色")),
    ("界面用哪种配色比较合适", Some("深色")), // 改写，零词法重叠
    ("帮我把编辑器调成暗色模式", Some("深色")), // 改写「暗色模式」
    ("你写代码常用什么语言？", Some("Rust")), // 人称改写
    ("我能吃花生做的零食吗？", Some("过敏")), // 事实推理
    ("给我推荐一个坚果礼盒", Some("过敏")),   // 间接安全相关
    ("我叫什么名字？", Some("韩斌")),         // 逐字事实
    ("你对我的回复长度有什么要求吗", Some("简短")), // 偏好改写
    ("commit 前需要先问你吗", Some("问我")),  // 偏好改写
    ("我下个月有什么计划？", Some("Windows")), // 目标
    ("我的电脑是什么显卡？", Some("3060Ti")), // 硬件事实
    ("vim 还是 emacs？", Some("vim")),        // 逐字偏好
    ("astra 部署有什么网络要求？", Some("离线")), // 项目约束改写
    ("小雨的生日是什么时候？", Some("三月")), // 关系事实
    ("今天天气怎么样？", None),               // 无关·应 0 命中
    ("帮我写一个快速排序算法", None),         // 无关·应 0 命中
    ("解释一下 TCP 三次握手", None),          // 无关·应 0 命中
    ("现在几点了？", None),                   // 无关·应 0 命中
];

fn load_judge() -> Option<OnnxJudge> {
    let model_dir = std::env::var("ASTRA_MEMORY_MODEL_DIR").ok()?;
    if std::env::var_os("ORT_DYLIB_PATH").is_none() {
        if let Ok(dylib) = std::env::var("ASTRA_MEMORY_ORT_DYLIB") {
            std::env::set_var("ORT_DYLIB_PATH", dylib);
        }
    }
    match OnnxJudge::load(OnnxJudgeConfig::new(std::path::Path::new(&model_dir))) {
        Ok(j) => Some(j),
        Err(e) => {
            eprintln!("judge load failed ({e}) — formation runs degraded");
            None
        }
    }
}

/// Deterministic stand-in for the LLM cognitive proposer: maps message
/// substrings the judge missed to the proposals a real model would emit.
/// Evidence/entity grounding is still done by the engine (Rust), so the
/// wiring under test is the production one.
struct TableProposer;

#[async_trait::async_trait]
impl astra_memory::proposer::CognitiveProposer for TableProposer {
    async fn propose(
        &self,
        input: &astra_memory::proposer::ProposerInput,
    ) -> astra_memory::Result<Vec<astra_memory::consolidation::ClaimProposal>> {
        use astra_memory::consolidation::{ClaimProposal, ProposalOperation};
        let c = &input.event.content;
        let op = if c.contains("astra 用 rmcp") {
            Some(ProposalOperation::CreateClaim {
                subject_entity: "project:astra".into(),
                predicate: "uses".into(),
                object: "astra 用 rmcp 实现 MCP client".into(),
            })
        } else {
            None
        };
        Ok(op
            .map(|operation| {
                ClaimProposal::new(
                    "prp_table".into(),
                    operation,
                    "scripted proposer (judge blind spot)",
                )
            })
            .into_iter()
            .collect())
    }
}

fn load_embedder() -> Option<BgeEmbedder> {
    let model_dir = std::env::var("BGE_MODEL_DIR").ok()?;
    if std::env::var_os("ORT_DYLIB_PATH").is_none() {
        if let Ok(dylib) = std::env::var("ASTRA_MEMORY_ORT_DYLIB") {
            std::env::set_var("ORT_DYLIB_PATH", dylib);
        }
    }
    match BgeEmbedder::load(std::path::Path::new(&model_dir), 2) {
        Ok(e) => Some(e),
        Err(e) => {
            eprintln!("embedder load failed ({e}) — recall runs lexical-only");
            None
        }
    }
}

#[tokio::test]
#[ignore = "needs model dirs + dylib; run with --ignored --nocapture"]
async fn batch_conversation_eval() {
    let judge = load_judge();
    let embedder = load_embedder();

    // Temp encrypted store.
    let dir = std::env::temp_dir().join(format!("astra-mem-eval-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut key = vec![0u8; 32];
    key[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    key[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    let store = SqliteStore::open(&dir.join("memory.sqlite"), &key).unwrap();

    let model: Option<Arc<dyn DecisionModel>> =
        judge.map(|j| Arc::new(j) as Arc<dyn DecisionModel>);
    let engine = {
        let e = match embedder {
            Some(e) => MemoryEngine::new(Arc::new(store), model)
                .with_embedder(Arc::new(e) as Arc<dyn Embedder>),
            None => MemoryEngine::new(Arc::new(store), model),
        };
        // Scripted proposer standing in for the LLM channel (docs/11 §3):
        // the real host builds OpenAiCompatProposer from env. The judge
        // discard on "astra 用 rmcp 实现 MCP client" (Observed) is exactly
        // the rule/judge blind spot the proposer supplements — this table
        // simulates what a real model would propose for it, and exercises
        // the engine→grounding→apply wiring end to end.
        Arc::new(e.with_proposer(Arc::new(TableProposer)))
    };

    // ===== Phase A: formation =====
    eprintln!("=== Phase A: observe {} utterances ===", SEEDS.len());
    let mut counts: std::collections::HashMap<String, usize> = Default::default();
    for (i, (msg, note)) in SEEDS.iter().enumerate() {
        let out = engine
            .observe(msg, &SubjectRef::user(), NOW + i as u64)
            .await
            .unwrap();
        let tag = match &out {
            astra_memory::formation::Outcome::Created { .. } => "Created",
            astra_memory::formation::Outcome::Reinforced { .. } => "Reinforced",
            astra_memory::formation::Outcome::Replaced { .. } => "Replaced",
            astra_memory::formation::Outcome::Observed => "Observed",
            astra_memory::formation::Outcome::Ignored => "Ignored",
            astra_memory::formation::Outcome::Discarded => "Discarded",
            astra_memory::formation::Outcome::RejectedPii => "RejectedPii",
        };
        *counts.entry(tag.to_string()).or_default() += 1;
        eprintln!("  [{tag:^10}] ({note}) {msg}");
    }
    eprintln!("  → summary: {counts:?}");

    let atoms = engine.list(&SubjectRef::user()).await.unwrap();
    eprintln!("=== {} long-term atoms stored ===", atoms.len());
    for a in &atoms {
        eprintln!(
            "  [{:?}] conf={:.2} imp={:.2} {}",
            a.kind, a.confidence, a.importance, a.content
        );
    }

    // ===== Phase B: recall probes =====
    eprintln!("=== Phase B: {} recall probes ===", PROBES.len());
    let mut top1_hits = 0usize;
    let mut top3_hits = 0usize;
    let mut positives = 0usize;
    let mut false_positives = 0usize;
    let mut negatives = 0usize;
    for (probe, expected) in PROBES {
        let hint_gate = engine.hint(probe);
        let hits = engine
            .recall(
                &RecallQuery {
                    text: probe.to_string(),
                    subject: None,
                    kinds: vec![],
                    limit: 3,
                },
                NOW + 100,
            )
            .await
            .unwrap();
        let rendered: Vec<String> = hits
            .iter()
            .map(|h| format!("({:.2}) {}", h.strength, h.content))
            .collect();
        eprintln!("  Q: {probe}  [hint={hint_gate}] → {} hits", hits.len());
        for r in &rendered {
            eprintln!("       {r}");
        }
        match expected {
            Some(kw) => {
                positives += 1;
                if hits.first().is_some_and(|h| h.content.contains(kw)) {
                    top1_hits += 1;
                }
                if hits.iter().any(|h| h.content.contains(kw)) {
                    top3_hits += 1;
                } else {
                    eprintln!("       ✗ MISS: expected keyword {kw:?} in top-3");
                }
            }
            None => {
                negatives += 1;
                if !hits.is_empty() {
                    false_positives += 1;
                    eprintln!("       ✗ FALSE POSITIVE");
                }
            }
        }
    }
    eprintln!(
        "=== recall metrics: top1 {top1_hits}/{positives}  top3 {top3_hits}/{positives}  false-positives {false_positives}/{negatives} ==="
    );

    // ===== Phase D: raw cosine distribution (threshold calibration) =====
    if let Some(e) = load_embedder() {
        let trunc = |s: &str| {
            let t: String = s.chars().take(12).collect();
            t
        };
        eprintln!("=== Phase D: raw cosines ===");
        let stored: Vec<String> = atoms.iter().map(|a| a.content.clone()).collect();
        let svecs: Vec<Vec<f32>> = stored
            .iter()
            .map(|s| e.embed(s).unwrap_or_default())
            .collect();
        for (probe, expected) in PROBES {
            let q = e.embed(probe).unwrap_or_default();
            let mut cos: Vec<(f32, &str)> = stored
                .iter()
                .zip(&svecs)
                .map(|(s, v)| (astra_memory::embed::cosine(&q, v), s.as_str()))
                .collect();
            cos.sort_by(|x, y| y.0.total_cmp(&x.0));
            let tag = match expected {
                Some(kw) => format!("POS({kw})"),
                None => "NEG".to_string(),
            };
            let top: Vec<String> = cos
                .iter()
                .take(2)
                .map(|(c, s)| format!("{c:.3} {}", trunc(s)))
                .collect();
            eprintln!("  [{tag}] {} → max {}", trunc(probe), top.join(" | "));
        }
        // Seed x seed matrix (consolidation channel calibration).
        eprintln!("  --- seed x seed ---");
        let seeds: Vec<&str> = SEEDS.iter().map(|(s, _)| *s).collect();
        let seed_vecs: Vec<Vec<f32>> = seeds
            .iter()
            .map(|s| e.embed(s).unwrap_or_default())
            .collect();
        for i in 0..seeds.len() {
            for j in (i + 1)..seeds.len() {
                let c = astra_memory::embed::cosine(&seed_vecs[i], &seed_vecs[j]);
                if c >= 0.55 {
                    eprintln!("  {:.3} {} × {}", c, trunc(seeds[i]), trunc(seeds[j]));
                }
            }
        }
    }

    // ===== Phase C: contradiction update round-trip =====
    eprintln!("=== Phase C: contradiction (D18) round-trip ===");
    let out = engine
        .observe(
            "我不用深色主题了，现在改成浅色主题",
            &SubjectRef::user(),
            NOW + 200,
        )
        .await
        .unwrap();
    eprintln!("  observe → {out:?}");
    let hits = engine
        .recall(&RecallQuery::new("我喜欢什么主题？", 3), NOW + 300)
        .await
        .unwrap();
    for h in &hits {
        eprintln!("  ({:.2}) {}", h.strength, h.content);
    }
    let outdated_gone = !hits
        .iter()
        .any(|h| h.content.contains("记住我喜欢深色主题"));
    let updated_present = hits.iter().any(|h| h.content.contains("浅色"));
    eprintln!(
        "  → old preference retired: {outdated_gone}, updated content recalled: {updated_present}"
    );

    // ===== Phase E: agent-visible context (provider fusion, docs/11 §6) =====
    // What the host actually injects as REMEMBERED CONTEXT lines: the
    // cognitive channel (project-state lines) ahead of the atom channel,
    // deduped — exactly what AstraMemoryProvider::search returns.
    eprintln!("=== Phase E: provider.search (REMEMBERED CONTEXT rendering) ===");
    let provider = AstraMemoryProvider::new(Arc::clone(&engine));
    for probe in [
        "astra 项目现在怎么打包？",
        "astra 的 MCP client 用什么实现？",
        "界面配色用哪种比较合适？",
        "下个月有什么计划？",
        "campus-ai 的安全决策引擎现在用什么？",
    ] {
        let entries = provider.search(probe, None, 5).await.unwrap();
        eprintln!("  Q: {probe}");
        if entries.is_empty() {
            eprintln!("       (no memory injected)");
        }
        for e in &entries {
            eprintln!("       - {}", e.content);
        }
    }

    assert!(positives > 0 && negatives > 0, "eval data must be labeled");

    // ===== Phase F: cognitive layer (multi-subject bridge, docs/11 §5) =====
    eprintln!("=== Phase F: cognitive layer checks ===");
    use astra_memory::consolidation::pred;
    use astra_memory::model::{ClaimStatus, EntityKind};
    let store = engine.store();
    let user = store
        .find_entity("user")
        .await
        .unwrap()
        .expect("user entity bridged");
    for name in ["astra", "campus-ai"] {
        let e = store
            .find_entity(name)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{name} project entity should exist"));
        assert_eq!(e.kind, EntityKind::Project);
    }
    let astra = store.find_entity("astra").await.unwrap().unwrap();
    let campus = store.find_entity("campus-ai").await.unwrap().unwrap();
    let astra_claims = store
        .claims_about(&astra.id, Some(ClaimStatus::Active))
        .await
        .unwrap();
    let campus_claims = store
        .claims_about(&campus.id, Some(ClaimStatus::Active))
        .await
        .unwrap();
    for (name, claims) in [("astra", &astra_claims), ("campus-ai", &campus_claims)] {
        eprintln!("  [{name}] {} active claims:", claims.len());
        for c in claims {
            eprintln!(
                "    {} {} (conf {:.2})",
                c.predicate, c.object, c.confidence
            );
        }
    }
    // Cross-project isolation: each project's cognition only speaks about
    // itself (跨项目不串 — the “打包 Windows” class of failures).
    assert!(
        !astra_claims.iter().any(|c| c.object.contains("campus-ai")),
        "astra cognition contaminated with campus-ai"
    );
    assert!(
        !campus_claims.iter().any(|c| c.object.contains("astra")),
        "campus-ai cognition contaminated with astra"
    );
    // Attribution: technology decisions sit on the RIGHT project.
    assert!(
        astra_claims
            .iter()
            .any(|c| c.predicate == pred::USES && c.object.contains("rmcp")),
        "astra should hold the rmcp uses claim"
    );
    // Reversal sync: campus-ai's single-valued uses claim must supersede
    // Rust Gateway with rmcp (claim-level Replaced, history kept).
    let campus_uses: Vec<_> = campus_claims
        .iter()
        .filter(|c| c.predicate == pred::USES)
        .collect();
    assert_eq!(
        campus_uses.len(),
        1,
        "single-valued uses claim must supersede, got {campus_uses:?}"
    );
    assert!(
        campus_uses[0].object.contains("rmcp"),
        "reversal did not sync the campus-ai uses claim: {}",
        campus_uses[0].object
    );
    // user ---> project edges exist for both projects.
    let edges = store.claim_relations_for(&user.id).await.unwrap();
    eprintln!("  user edges: {}", edges.len());
    for e in &edges {
        eprintln!("    {:?} -> {}", e.relation, e.to_entity);
    }
    assert!(edges
        .iter()
        .any(|e| e.to_entity == astra.id || e.to_entity == campus.id));
    eprintln!("=== Phase F: all cognitive-layer assertions passed ===");
}
