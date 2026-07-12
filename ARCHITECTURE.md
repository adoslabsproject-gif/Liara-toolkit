# Liara (zeli-local) — Architecture (as-built)

Cross-platform (**Android + macOS/Windows desktop**) **local-first** personal AI assistant.
Everything — model, reasoning, memory, tools — runs **on-device**. The network is touched only
when an inherently-online tool (web, weather, email) is invoked.

Quality bar: enterprise / "opera d'arte". One Rust core, many frontends.
App version: **0.2.5**. Tauri v2 + React/TS frontend + Rust core + in-process llama.cpp.

> ## Status (2026-07-11) — this is a CURRENT-STATE document
> Unlike the earlier draft, the sections below describe what is **actually built and in the tree**,
> not a target. Where something is still planned it is marked **PLANNED** explicitly.

---

## Core principles
1. **Local-first & private.** Model + reasoning + memory live on-device. At-rest data is encrypted.
2. **One Rust core, many frontends.** All logic in `app/src-tauri/src` compiles into the app on every
   platform. The UI (`app/src`) is a thin React layer over typed Tauri commands + an event stream.
3. **Trait boundaries.** `Engine`, `Tool`, `Memory` are swap points — implementations change without
   touching callers (this is what avoids the "rebuild 1000 times").
4. **Streaming-first.** Tokens, tool calls, memory updates all stream to the UI as events.
5. **Reliability over cleverness.** GBNF grammar forces valid tool-call JSON; every crash class the
   mobile GPU exposed has a named guard with an anti-regression test. Comments say *why*, not *what*.
6. **Training == runtime (anti-drift).** The `dump_tools` / `dump_prompt` bins export the REAL tool
   catalog and system prompt; the LoRA dataset consumes them verbatim, so the fine-tune is aligned to
   the exact prompt the app uses at inference.

---

## Repository map
```
app/
  src/                       React/TS frontend (App.tsx, Chart.tsx, i18n.ts)
  src-tauri/
    src/
      lib.rs                 app setup, AppState, Tauri command registration
      commands/              thin Tauri command handlers (one file per domain)
      core/                  the engine-agnostic Rust core (all the logic)
      bin/                   dump_tools, dump_prompt (anti-drift exporters), smoke
    vendor/llama-cpp-sys-2/  vendored llama.cpp (patched build.rs + Mali dlsym fix)
lora/                        dataset generation + LoRA/DPO/KTO training pipeline (Python/MLX)
models/                      local GGUF models (not committed; gitignored)
```

### Core modules (`app/src-tauri/src/core/`)
- `engine/` — inference behind the `Engine` trait.
  - `llama.rs` — `LlamaEngine` (llama.cpp via `llama-cpp-2`, **in-process**). Persistent KV-cache
    contexts (prefix caching across turns), 2 generation slots (0 = conversation, 1 = auxiliary:
    extraction/judge/summarize, capped ctx to avoid OOM), a mean-pooled embedding context, and a
    lazily-loaded `mtmd` projector for vision.
  - `vision.rs` — `VisionEngine`, a **separate** desktop VL model + mmproj (Android instead uses the
    unified VL engine so it never swaps models → avoids the Adreno OpenCL teardown crash).
  - `utf8.rs` — `Utf8Stream`: incremental UTF-8 assembly for streamed tokens, drops truly-invalid
    bytes (anti-jam) instead of stalling.
  - `mod.rs` — `Engine` trait (`generate`/`embed`/`describe`/`has_vision`/`is_gemma`), `GenOptions`,
    and a single shared `LlamaBackend` (llama.cpp forbids multiple `init()`).
- `agent/` — the ReAct loop (`agent_loop.rs`), prompt formatting (`format.rs`), output parsing
  (`parse.rs`). Handles two prompt dialects (Qwen ChatML + Gemma), tool-forcing, consent, intent
  guards, and streaming with tool-call suppression.
- `tools/` — `ToolRegistry` + 24 built-in tools + dynamic MCP tools. `registry.rs` is the single
  source of truth (exported by `dump_tools`). GBNF grammar generated from the tool set.
- `memory/` — SQLite persistence + an in-RAM decrypted vector index. Structured profile,
  auto-extracted facts, episodes, conversations, notes, per-tool permissions, location; plus the
  **v2** semantic/temporal store (see below).
- `crypto/` — AES-256-GCM at rest (`mod.rs`), Android hardware-keystore key wrapping via a JNI bridge
  (`android_keystore.rs`).
- `email/` — IMAP receive (`imap.rs`) + SMTP send (`smtp.rs`) over rustls; encrypted store (`store.rs`,
  `query.rs`) with INBOX/SENT/TRASH folders and soft-delete.
- `calendar/` — local agenda (SQLite), Italian relative-date parsing, encrypted title/notes.
- `audio/` — Piper TTS + whisper STT + silero VAD via sherpa-onnx (`mod.rs`, `vad.rs`, `text.rs`).
- `mcp.rs` — minimal MCP host: spawn configured stdio MCP servers, discover their tools, expose them
  as consent-gated dynamic tools.
- `extract.rs` — PDF → text (feeds `fs_read` and RAG). `eval.rs` — deterministic scorecard (routing /
  extraction / punctuation / memory recall). `paths.rs` — single source of truth for model locations.
  `android_ctx.rs` — JNI `ndk_context` init so audio doesn't panic.

### Commands (`app/src-tauri/src/commands/`)
`generate` (streaming ReAct + memory formation), `vision` (describe image → ReAct), `audio`
(TTS/STT), `email`, `calendar`, `memory`, `consent` (+ permissions), `rag` (`ingest_document`),
`download` (resumable model download + SHA256), `model_files` (`delete_model`, on-demand audio
`extract_audio`).

---

## Inference

- **In-process llama.cpp** (not a sidecar): Android cannot spawn arbitrary binaries, so the engine
  compiles into the app on every target — one code path everywhere. (Locked decision, no rework.)
- **Persistent contexts / prefix caching.** Backend + model are leaked to `'static` so the KV-cache
  contexts survive across turns; only the diverging suffix is re-decoded each turn. Trade-off
  (documented): switching model = restart the app.
- **Two prompt dialects.** Qwen ChatML (with an optional empty-`<think>` prefill to toggle Qwen3
  reasoning) and **Gemma** (`<|turn>` template, system folded into the first user turn) — selected by
  `Engine::is_gemma()`. This stops Gemma from imitating ChatML markers and leaking `<|im_end|>`.
- **Minimal system prompt.** ~110 tokens (distilled from ~690). Per-tool rules are **interiorized into
  the weights** by the dataset, not carried in the prompt — a short prompt means a short prefill,
  which is what keeps the mobile Adreno OpenCL backend from crashing on a long prompt.
- **Crash guards (each with an anti-regression test):** message-aware context-overflow trimming that
  keeps the system head + recent tail on ChatML boundaries; a chunked prefill; a decode stop one step
  before `n_ctx` (llama.cpp throws a C++ exception Rust can't catch → SIGABRT); `Utf8Stream` anti-jam.
- **Sampling.** Anti-loop chain (repeat + frequency/presence penalty over a 256-token window) so small
  models don't degenerate into "reasoning-papyrus" loops; greedy + light penalty for temp-0 extraction;
  variable seed on the conversational path (so "Regenerate" actually varies).
- **GBNF** lazily constrains tool-call JSON once the model emits `<tool_call>`.

---

## Agent — ReAct loop (`agent/agent_loop.rs`)
Up to 5 tool steps per turn: stream answer → detect `<tool_call>` → (consent gate if sensitive) →
execute → feed `<tool_response>` back → repeat. Extras that make a small local model reliable:
- **Tool-forcing.** A named URL/domain forces `web_fetch`; a clear search intent forces `web_search`
  (a small model tends to *write* the call as text instead of executing it). ~100% on unambiguous cases.
- **Intent guards.** received vs sent email, local (email/agenda/file) vs web disambiguation.
- **Streaming suppression** hides the tool-call JSON from the user while streaming the prose around it.
- **Tool-result cap** (6000 chars) sits above the web tools' own budgets (anti context-overflow).

---

## Memory (the pillar)
Two layers over one encrypted SQLite DB, with an in-RAM **decrypted** vector index loaded once at open
(recall is an in-memory cosine — it never rescans+decrypts the DB per turn).
- **v1 tables:** structured `profile`, `facts` (auto-extracted, injected each turn), `episodes`,
  `conversations`, `notes`, `settings` (location + per-tool `perm:` consent).
- **v2 `memories`:** `{kind, text, embedding, importance, created_at, valid_until}`.
  - **Recall score = similarity × importance × recency** (recency half-life 45d). Facts are excluded
    from recall (already injected via the profile block) — no double injection.
  - **Temporal knowledge graph:** a contradicting fact on the same topic *supersedes* the old one
    (`valid_until` set; removed from both the profile table and the vector store). Semantic dedup on
    near-duplicate rephrasings.
  - **Reflection:** every N turns, consolidate recent episodes into durable insights; episodes and
    reflections are pruned to bounds.
- **Memory formation runs detached** after "done" so the next turn is never blocked; it *skips* if a
  new turn is already running (GPU-busy flag) to avoid two concurrent decodes on the mobile GPU (ANR).

---

## Data & privacy
- **Encryption at rest: DONE.** AES-256-GCM (`enc:v1:` envelope, random 96-bit nonce per message).
  `encrypt` is **fail-closed** — a value that can't be encrypted is not written in the clear.
  Legacy plaintext rows are transparently migrated on next write.
- **Master key.** Desktop: OS keystore (hardware-backed where available). Android: the key is wrapped
  by the **AndroidKeyStore hardware** via a JNI `KeystoreBridge` (with a sandbox-file fallback).
- **Network safety.** SSRF guard on `web_fetch`/`web_search` re-checks the host on every redirect hop
  and pins the resolver inside ureq (closes DNS-rebinding). Path tools are confined to `$HOME` with a
  lexical-normalize + canonicalize traversal guard. Geo-IP fallback is HTTPS.
- **Frontend.** Strict CSP (`script-src 'self'`, `object-src 'none'`) + `rehype-sanitize` on model
  output — HTML surfaced from the web can't execute in the WebView or reach `invoke()`.
- **Consent.** Server-enforced `ConsentGate` (blocks the agent on a Condvar until the user decides;
  90s timeout denies). Sensitive tools are argument-aware; decisions persist as per-tool permissions.

---

## Tools (24 built-in + MCP)
`datetime`, `calculator`, `web_fetch`, `web_search`, `weather`, `set_location`,
`email_recent`, `email_sent`, `email_search`, `email_reply`, `email_draft`,
`calendar_add`, `calendar_list`, `calendar_search`, `calendar_delete`,
`fs_list`, `fs_read`, `fs_search`, `fs_write`, `fs_move`, `fs_delete`,
`note_add`, `note_list`, `note_search`.
Only the tools relevant to the request are rendered into the prompt (email/calendar always on; fs/note
gated by keyword). **MCP** servers configured via `LIARA_MCP` add their tools dynamically (all
consent-gated). The registry is the single source of truth (`dump_tools` → `tools_catalog.json`).

## Vision, Audio, RAG
- **Vision.** Qwen2.5-VL + mmproj via llama.cpp `mtmd`. Android: the unified VL engine describes the
  image itself (no model swap → no OpenCL crash), CLIP encoder forced to CPU (Adreno VLM bug). The
  image description is then fed into the normal ReAct loop, so vision + tools work in one turn.
- **Audio.** Piper TTS + whisper STT + silero VAD (sherpa-onnx, offline). Downloaded on-demand
  (`liara-audio.zip`, extracted with zip-slip guard) so the APK/DMG stay light. On Android the WebView
  captures the mic and plays audio (native cpal/rodio abort there).
- **RAG.** `ingest_document` (PDF via `extract.rs`, or text) → sentence-aware chunking → embed → stored
  in a `doc` namespace so ingested content can't crowd out personal memory in recall.

---

## Platform specifics
- **Android.** All-GPU OpenCL on Adreno; **partial offload** on weak GPUs (Mali/entry Snapdragon) so
  the compositor doesn't starve → no ANR. GGML Vulkan "advanced shaders" disabled; async disabled
  (destroyed-mutex race); `dlsym` lazy-load of `clCreateBufferWithProperties` so the lib loads on Mali
  (OpenCL 2.0). 16 KB page-size alignment (Samsung/Android 15). JNI `ndk_context` init for audio.
- **Desktop.** Metal on Apple; `mlock` the weights (anti-thrashing). `LIARA_MODELS_DIR` points at the
  app data dir in release (writable on any Mac) — the dev fallback is the project folder.
- **Model delivery.** Not bundled — downloaded on first run (resumable, SHA256-verified, `.part`
  ownership so a version bump re-downloads cleanly). A `models.json` endpoint exists **(frontend
  wiring PLANNED)**.
- **macOS distribution.** App is currently **ad-hoc signed** → Gatekeeper flags it as "damaged" on
  every other Mac. Real fix: **notarization (Developer ID + notarytool) — PLANNED.**

---

## Model lineup & training

Liara ships as a family, selected per device capability; the app already speaks the two prompt
dialects they need (Qwen ChatML / Gemma).

| Model | Params | Role | Dialect |
|-------|--------|------|---------|
| Ternary (BitNet-style) | 1.58B | weakest / no-GPU tier | (llama.cpp BitNet path) |
| Qwen2.5-VL | 1.7B | balanced mobile, text + **vision** | Qwen ChatML |
| Qwen2.5-VL | 4B | advanced mobile / desktop, text + vision | Qwen ChatML |
| Gemma | 4B | desktop | Gemma `<|turn>` |
| Gemma | 12B | high-end desktop | Gemma `<|turn>` |

**Dataset (`lora/`).** ~250k examples across **SFT + DPO + KTO**, generated against the REAL tool
catalog and system prompt (anti-drift). Design lessons already encoded: no combinatorial "stamp"
examples (they overfit → regurgitation), real conversation + web/tool chains (search → extract → admit
when not found), the full 24-tool catalog in every example (training == runtime). Pipeline is
MLX-based (Apple Silicon) with `gen_*` / `gold_*` generators, `combine_v3`, and per-model train scripts.

---

## Roadmap — what's left ("the missing plan")
Most of the original build order is **done** (engine, chat, memory v1+v2, tools/MCP, encryption, GBNF,
vision, audio, consent, Android build). What remains:

1. **Multi-model training matrix.** One 250k dataset → 5 fine-tunes across 2 dialects and 3 objectives
   (SFT→DPO/KTO). Needs: a consistent per-model prompt export (dialect-aware `dump_prompt`), a training
   recipe per size/family, and an eval gate (`eval.rs` scorecard + live checks) each model must pass
   before shipping. **This is the main open work item.**
2. **Ternary 1.58B inference path.** Confirm the vendored llama.cpp BitNet kernels run on the target
   devices; it becomes the true no-GPU/weak tier.
3. **Android inference stability.** Track the open "Rust cannot catch foreign exceptions" prefill crash
   on some Adreno devices (independent of `n_ctx`) — do not ship an APK until closed.
4. **macOS notarization** so the DMG opens for anyone without terminal workarounds.
5. **Dynamic model catalog.** Wire the frontend to `fetch_models` (`models.json`) so the model list is
   server-driven instead of hardcoded in `App.tsx`.
6. **RAG domain packs** (PLANNED): downloadable pre-built packs (chunks + embeddings + optional domain
   tools + prompt), sharing the embedding/vector stack with memory.
7. **Frontend refactor** (PLANNED): split the ~1.5k-line `App.tsx` into hooks/panels.
