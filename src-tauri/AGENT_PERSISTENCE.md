# Agent thread persistence (host integration)

## Storage and ownership

- One lazily opened SDK `SqliteThreadStore` per `AgentState`, at
  `app.path().app_data_dir()/agent_threads.sqlite3` (including SQLite WAL/SHM sidecars).
- `ThreadRecord.id == AgentStartReq.session_id`. IDs must be nonempty, stable, and
  never reused for a different UI session. Missing records are explicitly created;
  existing records are restored with SDK `Thread::load` and newly assembled runtime
  configuration. No SDK implementation is copied or modified.
- SQLite contains full model messages, including tool calls/results. UI cards in
  `agent_sessions.json` remain separate and are **not** a model-history source.
- Old UI-only sessions cannot recover model context from before this integration.
  The host does not invent tool messages from presentation entries.
- SDK write-through happens at Run completion/cancellation. Forced termination
  during a Run can lose that Run; a terminal model event alone is not a flush barrier.

## Frontend IPC integration required

Existing command names and argument shapes remain valid (Tauri supplies state):

1. `agent_start_run` returns the session ID as before. Concurrent sends, resets and
   deletes are rejected while admission is occupied, including cancelled tails.
   This intentionally preserves the host's single-active-Run model.
2. `agent_cancel` requests cancellation; it does **not** make the host idle immediately.
3. Listen for **`agent-run-settled`** with
   `{ session_id: string, persistence_error: string | null }`. This host lifecycle
   notification follows SDK event-stream closure/write-through. Keep send/delete/
   reset disabled until settlement, not merely the SDK terminal event. SDK envelopes
   on `agent-event` are unchanged.
4. `agent_status` now returns
   `{ running, session_id, persistence_errors: Record<string, string> }`.
   `running` includes cancellation/drain. Use the retained errors when mounting or
   recovering from a missed lifecycle notification; show persistence failures to
   the user instead of implying history is saved.
5. `agent_reset_session({ sessionId })` is now fallible and unloads memory only.
   To stop and unload: cancel, await settlement, then await reset. It refuses to
   discard a thread after a failed write. A later successful Run retries the SDK's
   unsaved suffix and can clear the error. Reset never deletes SQLite history.
6. `delete_agent_session({ id })` deletes SDK thread/artifact history and then the
   UI JSON record. Await success before removing the UI item. While any Run is
   active/draining it rejects deletion. Explicit deletion may discard unsaved data.
   No new delete command or handler registration is required.

The UI files were intentionally not changed in this backend-only task.

## Failure and concurrency behavior

- Initialization/create/load/delete errors reject their command; no in-memory fallback.
- A host `ThreadStore` decorator retains SDK get/update/append/messages failures,
  including title initialization errors that the SDK otherwise ignores. Run-tail
  errors are available in settlement notification and status. Unsaved memory stays
  loaded rather than being silently discarded.
- An owned Tokio mutex guard spans admission through receiver drain. No standard
  mutex guard crosses an await. Cancel/status use separate short-lived locks.
- UI JSON list/save/delete operations are serialized. Process-local deletion
  tombstones reject delayed saves/sends for deleted IDs.
- SQLite and JSON are not a cross-store transaction: SQLite deletion succeeds
  first. If the JSON write fails, the command reports failure and can be retried;
  delayed saves cannot resurrect that session within the process. Do not reuse IDs.

## Tests

`src/agent_threads_tests.rs` exercises the real host manager and SDK SQLite store:
full tool execution → write-through → reopen → full model-request history; stable
IDs, isolation, unload/reload, idempotent deletion, deletion tombstones, storage
errors, invalid paths/IDs and cancellation/drain lifecycle exclusion.

```sh
/Users/hanbiao/.cargo/bin/cargo check --tests --locked --offline -j 2
/Users/hanbiao/.cargo/bin/cargo test --lib agent_threads::tests --locked --offline -j 1
```
