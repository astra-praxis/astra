//! Temp diagnostic: open the real app memory store and dump all atoms,
//! plus simulate a Run-start recall with a probe query.
//! Run: cargo test --test dump_memory -- --ignored --nocapture

#[cfg(test)]
mod real_db_dump {
    use astra_memory::embed::{BgeEmbedder, Embedder};
    use astra_memory::engine::MemoryEngine;
    use astra_memory::protocol::RecallQuery;
    use astra_memory::store::MemoryStore;
    use astra_memory::store_sqlite::SqliteStore;

    #[tokio::test]
    #[ignore]
    async fn dump_real_memory() {
        let dir = dirs::data_dir().expect("data dir").join("com.astra.app");
        let key = std::fs::read(dir.join("memory.key")).expect("key file");
        let store = SqliteStore::open(&dir.join("memory.sqlite"), &key).expect("open store");
        let hits = store.search("", None, true, 500).await.expect("search");
        eprintln!("=== {} memories in store ===", hits.len());
        for a in &hits {
            eprintln!(
                "[{:?}|{:?}] strength={:.2} conf={:.2} created={} {}",
                a.status, a.kind, a.strength, a.confidence, a.created_at, a.content
            );
        }

        // Simulate what a Run-start recall would inject (probe = args or default).
        let probe =
            std::env::var("MEMORY_PROBE").unwrap_or_else(|_| "这个项目是用什么语言写的？".into());
        // Embedder (semantic channel) loads when BGE_MODEL_DIR + dylib are set;
        // otherwise lexical-only — same degrade rule as the host.
        let embedder: Option<BgeEmbedder> = match (
            std::env::var("BGE_MODEL_DIR"),
            std::env::var("ASTRA_MEMORY_ORT_DYLIB").or_else(|_| std::env::var("ORT_DYLIB_PATH")),
        ) {
            (Ok(model_dir), Ok(dylib)) => {
                if std::env::var_os("ORT_DYLIB_PATH").is_none() {
                    std::env::set_var("ORT_DYLIB_PATH", dylib);
                }
                match BgeEmbedder::load(std::path::Path::new(&model_dir), 2) {
                    Ok(e) => {
                        eprintln!("embedder loaded: {model_dir}");
                        Some(e)
                    }
                    Err(e) => {
                        eprintln!("embedder load failed, lexical-only: {e}");
                        None
                    }
                }
            }
            _ => {
                eprintln!("BGE_MODEL_DIR not set → lexical-only recall");
                None
            }
        };
        let engine = match embedder {
            Some(e) => MemoryEngine::new(std::sync::Arc::new(store), None)
                .with_embedder(std::sync::Arc::new(e) as std::sync::Arc<dyn Embedder>),
            None => MemoryEngine::new(std::sync::Arc::new(store), None),
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let recalled = engine
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
        eprintln!("=== recall probe: {probe:?} → {} hits ===", recalled.len());
        for h in &recalled {
            eprintln!("  score={:.2} [{:?}] {}", h.strength, h.kind, h.content);
        }
    }
}
