//! Temp diagnostic: run the REAL judge (fp16 ONNX) against the REAL app
//! memory store and print judge relevance scores for a probe query.
//! Env: MEMORY_PROBE (query), ASTRA_MEMORY_MODEL_DIR, ASTRA_MEMORY_ORT_DYLIB.
//! Run: cargo test --test dump_judge -- --ignored --nocapture

#[cfg(test)]
mod judge_probe {
    use astra_memory::engine::MemoryEngine;
    use astra_memory::judge_onnx::{OnnxJudge, OnnxJudgeConfig};
    use astra_memory::protocol::RecallQuery;
    use astra_memory::store_sqlite::SqliteStore;

    #[tokio::test]
    #[ignore]
    async fn judge_probe_real_db() {
        let dir = dirs::data_dir().expect("data dir").join("com.astra.app");
        let key = std::fs::read(dir.join("memory.key")).expect("key file");
        let store = SqliteStore::open(&dir.join("memory.sqlite"), &key).expect("open store");

        let model_dir =
            std::env::var("ASTRA_MEMORY_MODEL_DIR").expect("set ASTRA_MEMORY_MODEL_DIR");
        std::env::set_var(
            "ORT_DYLIB_PATH",
            std::env::var("ASTRA_MEMORY_ORT_DYLIB").expect("set ASTRA_MEMORY_ORT_DYLIB"),
        );
        let judge = OnnxJudge::load(OnnxJudgeConfig::new(&model_dir)).expect("judge load");
        eprintln!("judge loaded: {}", judge.model_file());

        let probe = std::env::var("MEMORY_PROBE").unwrap_or_else(|_| "我喜欢深色主题吗".into());

        // Print the judge's raw relevance for every candidate to see why the
        // cull at min_relevance (=0.25) drops lexical-strong matches.
        let atoms = {
            use astra_memory::store::MemoryStore;
            let all = {
                let s =
                    astra_memory::store_sqlite::SqliteStore::open(&dir.join("memory.sqlite"), &key)
                        .expect("reopen store");
                s.search("", None, true, 500).await.expect("search")
            };
            let state = astra_memory::decision::DecisionState {
                conversation: probe.clone(),
                candidates: all.iter().map(|a| a.content.clone()).collect(),
            };
            let judge_model: &dyn astra_memory::decision::DecisionModel = &judge;
            let r = judge_model
                .decide(state, astra_memory::decision::DecisionQuestions::recall())
                .await
                .expect("judge decide");
            eprintln!("=== judge relevance for probe={probe:?} ===");
            for (a, rel) in all.iter().zip(&r.relevance) {
                eprintln!("  relevance={rel:.4} [{:?}] {}", a.kind, a.content);
            }
            all
        };
        eprintln!("candidates={} ", atoms.len());
        let engine = MemoryEngine::new(
            std::sync::Arc::new(store),
            Some(std::sync::Arc::new(judge)
                as std::sync::Arc<dyn astra_memory::decision::DecisionModel>),
        );
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let hits = engine
            .recall(
                &RecallQuery {
                    text: probe.clone(),
                    subject: None,
                    kinds: vec![],
                    limit: 5,
                },
                now,
            )
            .await
            .expect("recall");
        eprintln!("=== recall(probe={probe:?}) → {} hits ===", hits.len());
        for h in &hits {
            eprintln!("  score={:.2} [{:?}] {}", h.strength, h.kind, h.content);
        }
    }
}
