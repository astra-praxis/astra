# Agent project image recognition

## Usage

1. Select a provider/model whose **model and gateway** support vision and OpenAI-compatible `image_url` data URLs. Astra does not infer this from model names or probe support automatically.
2. In the Agent conversation settings, enable **图片识别** (off by default). This is explicit consent to send project images to the selected model and store their normalized content in local SQLite history. The preference is saved in localStorage.
3. Start a **new conversation**, select the project directory, and ask: `请用 read_file 读取 images/example.png，描述图片中的内容。` A project-relative path is sufficient; no upload/drag-and-drop UI is needed. Do not use image `offset`/`limit`.
4. The existing tool artifact card shows the source path and normalized PNG format, dimensions and encoded byte count. It is not a thumbnail or proof of a successful model response. Inspect the tool/model error if recognition fails.

## Session configuration

`AgentStartReq.image_recognition` defaults to false. Only explicit true selects `ImageEncoding::UserImageUrl`; otherwise the backend uses `Disabled`.

The host persists `host_image_config` (enabled, endpoint, resolved model; no API key) in the SQLite thread metadata under the existing lifecycle lock, **before** cached-thread reuse. A mismatch rejects the request before model invocation. This also prevents an image-enabled cached thread from silently using a different destination than the selected model. Model/endpoint changes likewise require a new conversation. Reverting to the bound configuration allows continuation.

Changing the checkbox only changes the preference for new conversations, not existing consent. The toolbar labels it “新会话设置”, not the current thread's effective state. Existing sessions keep their binding across unload/restart. Legacy sessions without metadata are treated as image-disabled and cannot be upgraded in place. Disabling the preference is not a history scrub: start a new disabled conversation to stop using images. Existing enabled conversations cannot continue with the disabled setting.

## Privacy and limits

- PNG, JPEG and WebP project files only; SDK canonicalization/root checks still apply. The registered filesystem tools are unchanged.
- Source ceilings: 20 MiB, 24 million pixels, 16,384 px edge. Normalization resizes to at most 1,568 px edge and at most 2 MiB PNG per image. Active model context is limited to four images. Decoder/output limits can reject otherwise valid files.
- Animated images become stills. EXIF orientation, profiles and metadata are not guaranteed. No OCR tool, PDF rasterization, remote image download or automatic compatibility fallback is added.
- Normalized visible image content goes to the selected provider, including later context and compaction requests. It is stored inline in `agent_threads.sqlite3` message JSON, including possible WAL/backups. Deleting the source image does not remove this history. Re-encoding is not anonymization.
- No new encryption, quota, secure erasure or remote retention control is provided. Deleting a session uses existing history deletion, not a guarantee of erasure from WAL/backups/provider storage. Do not downgrade to a pre-image SDK with image histories.
- Events/artifacts contain metadata only, never image base64. No fake thumbnail or reread of the source file is used.

## Verification

Host tests cover default-off deserialization/backend capability, cached-session mismatch, destination mismatch, legacy behavior, restart persistence, actual `read_file` image delivery to a mock vision backend, metadata-only events, and restoring image context after deleting the source. They do not certify live cloud gateway compatibility.

Frontend tests cover preference persistence and image metadata rendering alongside the existing test suite. Run from `web`: `pnpm typecheck`, `pnpm build`, and all `src/**/*.test.mjs` files with Node's test runner. Run from `src-tauri`: `cargo check --locked` and `cargo test --locked --lib`.
