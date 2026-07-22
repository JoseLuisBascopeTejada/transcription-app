# Meeting Transcriber — Project Plan

Open-source, cross-platform desktop app for live and file-based meeting transcription
with speaker diarization, local-first storage, and one-click installation.

- **License:** MIT (permissive, maximizes adoption and contribution; avoids GPL
  copyleft friction for downstream users who may want to embed/fork it)
- **Repository host:** GitHub
- **Privacy stance:** 100% local processing by default. No account, no telemetry,
  no cloud dependency required to use the app. Cloud STT is an opt-in toggle only.

This plan is designed to be executed by a multi-agent team running on
free/low-tier AI models. See [SKILLS.md](SKILLS.md) for the full agent
roster and skills matrix, and `skills/*.md` for the bug-prevention rule
files each agent must apply. **Every task below that touches a
skill-covered area explicitly names the skill file to load before starting
— this is not optional context, it is a required input to the task.**

---

## 1. Goals & Non-Goals

**Goals**
- Ship a single installer per OS (Windows `.msi`, macOS `.dmg`, Linux `.AppImage`/`.deb`)
  that "just works" with no manual dependency setup (no "install Python first").
- Live microphone transcription + pre-recorded file transcription (mp3, wav, mp4, m4a).
- Automatic speaker labeling (Speaker 1, Speaker 2…) with user rename support.
- Transcripts stored locally in a single lightweight SQLite file — no server, no cloud DB.
- Final transcript auto-copied to clipboard on completion.
- Fully open source, easy for outside contributors to build and run from source.

**Non-goals (v1)**
- True word-level real-time diarization (deferred — see architecture note in section 3).
- Multi-user/team sync or cloud backup (may become an optional plugin later, not core).
- Mobile app.

---

## 2. Technology Stack

| Layer | Technology | Rationale |
|---|---|---|
| Desktop shell / GUI | **Tauri 2 (Rust)** + **React** (or Svelte) frontend | Small binaries (no bundled Chromium runtime like Electron), low RAM footprint, and — critically for "ease of installation" — `tauri-bundler` produces native installers (MSI/NSIS, DMG, AppImage/deb) out of the box with a single `tauri build` command |
| Native orchestration | **Rust** | Audio device capture, file I/O, SQLite access, clipboard, process management for the Python sidecar |
| ML/audio processing | **Python 3.11**, packaged as a **PyInstaller one-file sidecar binary** bundled inside the Tauri app | Whisper and pyannote ecosystems are Python-native; bundling as a sidecar means end users never install Python themselves |
| Speech-to-text | **faster-whisper** (CTranslate2 backend), model default `distil-large-v3` | Best accuracy/speed trade-off cross-platform (CPU + NVIDIA GPU), MIT-licensed, works identically on Win/Mac/Linux |
| Speaker diarization | **pyannote.audio 4.0 (Community-1 pipeline)** | Open-source, local, current best-in-class open diarization accuracy; requires a free Hugging Face token (handled in first-run setup wizard) |
| Audio decode/normalize | **ffmpeg** (bundled binary) | Converts any input (mp3/mp4/m4a/wav) to 16kHz mono WAV for Whisper |
| Local database | **SQLite** (via `rusqlite` in Rust) | Single-file, zero-config, ACID, queryable, negligible size for text data |
| Audio storage (optional) | **Opus** codec, low bitrate, via ffmpeg | ~40x smaller than raw WAV if user opts to retain audio |
| Clipboard | **arboard** (Rust crate) | Cross-platform clipboard write, no extra runtime |
| Live audio capture | **cpal** (Rust crate) | Cross-platform mic access matching Tauri's Rust core |
| CI/CD & release builds | **GitHub Actions** (`tauri-action`) | Builds signed installers for all 3 OSes automatically on tag/release |
| Packaging Python sidecar | **PyInstaller** | Produces a self-contained Python executable with faster-whisper/pyannote bundled, no user-side pip install |

---

## 3. Architecture

### 3.1 High-level component diagram

```
┌─────────────────────────────────────────────────────────────┐
│                      Tauri Shell (Rust)                      │
│  ┌───────────────┐   ┌───────────────┐   ┌────────────────┐  │
│  │  React UI      │   │ Audio Capture │   │  SQLite Store  │  │
│  │ (live view,    │   │ (cpal, mic)   │   │  (rusqlite)    │  │
│  │  file upload,  │   └──────┬────────┘   └────────┬───────┘  │
│  │  history,      │          │                     │          │
│  │  rename UI)    │          ▼                     │          │
│  └───────┬────────┘   Audio buffer/                │          │
│          │             temp WAV file                │          │
│          │                   │                      │          │
│          ▼                   ▼                      │          │
│   ┌──────────────────────────────────────┐          │          │
│   │      Tauri command layer (Rust)       │◄─────────┘          │
│   │  orchestrates pipeline + clipboard    │                     │
│   └──────────────┬────────────────────────┘                     │
└──────────────────┼──────────────────────────────────────────────┘
                    │  local IPC (stdio/HTTP over localhost)
                    ▼
     ┌───────────────────────────────────────────┐
     │      Python Sidecar (PyInstaller binary)   │
     │  ┌───────────────┐   ┌───────────────────┐ │
     │  │ faster-whisper │   │ pyannote.audio 4.0 │ │
     │  │ (transcription)│   │ (diarization)      │ │
     │  └───────────────┘   └───────────────────┘ │
     │              ffmpeg (normalize audio)        │
     └───────────────────────────────────────────┘
```

### 3.2 Pipeline flow (shared by both modes)

```
AudioSource (Microphone stream | Uploaded file)
   → ffmpeg normalize → 16kHz mono WAV
   → faster-whisper
        - Live mode: incremental, VAD-chunked (~5–10s windows), streamed to UI as partial text
        - File mode: single batch pass over the whole file
   → [on completion] pyannote.audio batch diarization over the full recording
   → Reconciliation: align diarization speaker turns to Whisper segment timestamps
   → Write meeting + segments + speakers to SQLite
   → Format transcript as plain text → write to system clipboard (arboard)
   → UI renders final labeled transcript, speakers renamable inline
```

**Design note on diarization timing:** true real-time diarization is still an
immature, mostly cloud-only capability. This app diarizes *after* the recording
finishes (batch, on the full audio) even in Live Mode — this keeps accuracy high
without needing a cloud dependency, while the live transcript itself still streams
in near-real-time so the user isn't staring at a blank screen.

### 3.3 Data model (SQLite)

```sql
CREATE TABLE meetings (
  id INTEGER PRIMARY KEY,
  title TEXT NOT NULL,
  created_at TEXT NOT NULL,
  duration_seconds INTEGER,
  source_type TEXT CHECK(source_type IN ('live','file')),
  audio_path TEXT              -- NULL if audio was discarded post-transcription
);

CREATE TABLE speakers (
  id INTEGER PRIMARY KEY,
  meeting_id INTEGER REFERENCES meetings(id) ON DELETE CASCADE,
  label TEXT NOT NULL,          -- e.g. "Speaker 1" or user-renamed "Maria"
  embedding BLOB                -- optional, for cross-meeting voice matching
);

CREATE TABLE segments (
  id INTEGER PRIMARY KEY,
  meeting_id INTEGER REFERENCES meetings(id) ON DELETE CASCADE,
  speaker_id INTEGER REFERENCES speakers(id),
  start_ms INTEGER NOT NULL,
  end_ms INTEGER NOT NULL,
  text TEXT NOT NULL
);

CREATE INDEX idx_segments_meeting ON segments(meeting_id);
```

This exact schema is the authoritative source referenced by
`skills/sqlite-schema-integrity.md` — any task that writes SQL must paste
it into context rather than working from memory of it.

### 3.4 Storage footprint strategy
- Default: audio is deleted after transcription completes; only text + metadata persist (tens of KB per meeting).
- Optional "keep audio" setting: re-encode to Opus at 16–24kbps mono before storing (~10–15MB/hour instead of ~600MB raw WAV).

### 3.5 Ease-of-installation strategy
- End users download **one installer file** per OS from GitHub Releases — no Python, no pip, no manual model downloads before first launch.
- On first launch, a **setup wizard**:
  1. Downloads the selected Whisper model (`distil-large-v3` by default, size ~750MB) with a progress bar.
  2. Prompts for a free Hugging Face token (one-time, needed for pyannote's gated model) with a direct link and copy-paste field — stored locally in the app config, never transmitted anywhere else.
  3. Downloads pyannote's Community-1 pipeline weights.
- All subsequent runs are fully offline.

---

## 4. Agents & Skills Reference

Full detail lives in [SKILLS.md](SKILLS.md). Quick-reference mapping of
agent → skill files they must load, used throughout Section 5:

| Agent | Skill files to load for relevant tasks |
|---|---|
| Orchestrator / Planning | None (no code-level skill file; works from this PLAN.md + SKILLS.md directly) |
| Frontend (React/Tauri UI) | `skills/tauri-ipc-contract.md` |
| Backend (Rust shell + DB) | `skills/audio-capture-lifecycle.md`, `skills/sqlite-schema-integrity.md`, `skills/tauri-ipc-contract.md` |
| ML / Python Sidecar | `skills/sidecar-model-lifecycle.md`, `skills/diarization-reconciliation.md` |
| DevOps / Packaging | `skills/sidecar-packaging.md` |
| QA / Testing | `skills/test-fixture-scoping.md`, plus the skill file matching whatever component is under test |
| Documentation | None (no code-level skill file) |

**Rule for the Orchestrator when dispatching a task:** if a task falls under
any row in Section 5 marked "Skill required," the dispatch prompt must name
the exact skill file path and instruct the agent to read it in full before
writing any code. This is a hard requirement, not a suggestion — it's how
low-tier models avoid the specific bugs each skill file documents.

---

## 5. Repository Structure

```
meeting-transcriber/
├── src-tauri/              # Rust shell: commands, audio capture, sqlite, clipboard
│   ├── src/
│   ├── Cargo.toml
│   └── tauri.conf.json
├── src/                     # React frontend
│   ├── components/
│   ├── pages/
│   └── App.tsx
├── sidecar/                 # Python ML sidecar
│   ├── transcribe.py        # faster-whisper wrapper
│   ├── diarize.py           # pyannote.audio wrapper
│   ├── reconcile.py         # align transcript + diarization
│   ├── requirements.txt
│   └── build_sidecar.spec   # PyInstaller spec
├── skills/                  # Agent bug-prevention rule files (see SKILLS.md)
│   ├── sidecar-model-lifecycle.md
│   ├── diarization-reconciliation.md
│   ├── audio-capture-lifecycle.md
│   ├── sqlite-schema-integrity.md
│   ├── tauri-ipc-contract.md
│   ├── sidecar-packaging.md
│   └── test-fixture-scoping.md
├── .github/
│   └── workflows/
│       ├── ci.yml           # lint/test on PR
│       └── release.yml      # tauri-action, builds installers on tag
├── docs/
│   ├── ARCHITECTURE.md
│   └── CONTRIBUTING.md
├── LICENSE                  # MIT
├── README.md
└── PLAN.md                  # this file
```

---

## 6. Detailed Step-by-Step Breakdown

Each task below is tagged with its **Agent**, **Mode** (Plan/Build), and,
where applicable, **Skill required** — the exact file the assigned agent
must read before starting. Tasks with no skill tag are either
orchestration/docs work or are simple enough (per SKILLS.md's L1/L2
experience-level guidance) not to need one.

### Phase 0 — Project Setup (Week 1)

| # | Task | Agent | Mode | Skill required |
|---|---|---|---|---|
| 0.1 | Initialize GitHub repo, MIT `LICENSE`, `README.md`, `.gitignore` | Documentation | Build | — |
| 0.2 | Scaffold Tauri 2 project with React + TypeScript template | Frontend | Build | — |
| 0.3 | Set up `sidecar/` Python project (venv, `requirements.txt`: faster-whisper, pyannote.audio, ffmpeg-python) | ML/Sidecar | Build | — |
| 0.4 | Configure GitHub Actions CI: lint (`clippy`, `eslint`, `ruff`), basic build check on push/PR | DevOps | Build | — |
| 0.5 | Write `CONTRIBUTING.md` and issue/PR templates, including a note pointing contributors to `skills/*.md` before touching covered areas | Documentation | Build | — |
| 0.6 | Break Phase 1–3 into atomic per-agent tasks, confirm no task spans two agents' responsibilities | Orchestrator | Plan | — |

### Phase 1 — Core Audio Pipeline (Weeks 2–3)

| # | Task | Agent | Mode | Skill required |
|---|---|---|---|---|
| 1.1 | Implement mic capture via `cpal`; write raw PCM to a temp WAV file, resampled/downmixed to 16kHz mono regardless of device native format | Backend | Build | **`skills/audio-capture-lifecycle.md`** — read in full before writing the capture stream setup or the WAV write path |
| 1.2 | Implement file-upload handling (accept mp3/wav/mp4/m4a); normalize via ffmpeg to 16kHz mono WAV before passing to sidecar | Backend | Build | — (ffmpeg normalization only, no cpal involved; skill not required, but must match the same 16kHz mono target defined in 1.1) |
| 1.3 | Python: `transcribe.py` — load faster-whisper model once at sidecar startup, expose stdio/HTTP interface accepting an audio path + mode (`live-chunk` \| `full-file`), return JSON segments with timestamps | ML/Sidecar | Build | **`skills/sidecar-model-lifecycle.md`** — model must be loaded once at startup per this skill's checklist, not per request |
| 1.4 | Rust ⇄ Python: implement sidecar process management (spawn on app start, communicate via local HTTP on `127.0.0.1:<random-port>`) | Backend | Build | **`skills/tauri-ipc-contract.md`** applies to the Rust-side command wrapping this call; define and document the exact JSON request/response shape before implementation |
| 1.5 | Wire the recording start/stop Tauri commands to the frontend | Frontend | Build | **`skills/tauri-ipc-contract.md`** — command names and payload types must be defined jointly with task 1.4's output before this task starts |
| 1.6 | **Milestone check:** can record from mic OR upload a file and get a raw (undiarized) transcript printed to the UI | QA | Build | **`skills/test-fixture-scoping.md`** — any automated test written here must use session-scoped model fixtures per this skill |

### Phase 2 — Speaker Diarization (Weeks 4–5)

| # | Task | Agent | Mode | Skill required |
|---|---|---|---|---|
| 2.1 | Python: `diarize.py` — load pyannote.audio Community-1 pipeline once at sidecar startup, run on a completed recording, return speaker turn segments (start, end, speaker_id) | ML/Sidecar | Build | **`skills/sidecar-model-lifecycle.md`** — same load-once rule applies to the diarization pipeline as to Whisper |
| 2.2 | Python: `reconcile.py` — merge Whisper segment timestamps with diarization turns into final `[Speaker N] text` structure, using overlap-duration matching (not start-time containment) | ML/Sidecar | Build | **`skills/diarization-reconciliation.md`** — read in full; this is explicitly flagged as the highest-risk logic in the project |
| 2.3 | Handle the Hugging Face token setup flow (config file, validation, clear error messaging if missing/invalid) | ML/Sidecar | Build | **`skills/sidecar-model-lifecycle.md`** — HF token handling section specifically |
| 2.4 | **Milestone check:** a finished recording produces a fully speaker-labeled transcript | QA | Build | **`skills/test-fixture-scoping.md`** and **`skills/diarization-reconciliation.md`** — reconciliation tests must use hand-constructed timestamp fixtures per the latter, not real audio |

### Phase 3 — Local Storage (Week 6)

| # | Task | Agent | Mode | Skill required |
|---|---|---|---|---|
| 3.1 | SQLite schema migration setup (`rusqlite`); create the `meetings`, `speakers`, `segments` tables exactly as defined in PLAN.md §3.3 | Backend | Build | **`skills/sqlite-schema-integrity.md`** — `PRAGMA foreign_keys = ON` must be set per this skill's checklist |
| 3.2 | Implement write path: meeting → speakers → segments, on pipeline completion, wrapped in a single transaction | Backend | Build | **`skills/sqlite-schema-integrity.md`** — transaction-wrapping requirement specifically |
| 3.3 | Implement read/query path: meeting history list, full transcript by id, search across meetings | Backend | Build | **`skills/sqlite-schema-integrity.md`** — column names must match §3.3 exactly |
| 3.4 | Implement optional Opus audio retention (ffmpeg re-encode) behind a settings toggle; default = discard audio | Backend | Build | — |
| 3.5 | **Milestone check:** transcripts persist across app restarts; history view lists past meetings; deleting a meeting cascades to its speakers/segments | QA | Build | **`skills/sqlite-schema-integrity.md`** — specifically verify cascade-delete behavior, the most common failure mode this skill documents |

### Phase 4 — GUI / UX (Weeks 7–8)

| # | Task | Agent | Mode | Skill required |
|---|---|---|---|---|
| 4.1 | Live Mode screen: record/stop controls, live-scrolling partial transcript, waveform/level indicator | Frontend | Build | **`skills/tauri-ipc-contract.md`** — the partial-transcript event listener must follow the typed-payload pattern in this skill |
| 4.2 | File Mode screen: drag-and-drop or file picker, progress indicator during processing | Frontend | Build | **`skills/tauri-ipc-contract.md`** |
| 4.3 | Transcript view: speaker-labeled, click-to-rename speaker labels (propagates across the meeting), timestamps toggle | Frontend | Build | **`skills/tauri-ipc-contract.md`** — rename action calls a Backend command; shape must be defined jointly with Backend first |
| 4.4 | History/library view: searchable list of past meetings | Frontend | Build | **`skills/tauri-ipc-contract.md`** |
| 4.5 | Settings screen: model selection, cloud STT opt-in toggle (clearly labeled, off by default), audio retention toggle, HF token management | Frontend | Build | **`skills/tauri-ipc-contract.md`** |
| 4.6 | **Milestone check:** feature-complete UI wired to the full pipeline | QA | Build | **`skills/test-fixture-scoping.md`** |

### Phase 5 — Clipboard & Polish (Week 9)

| # | Task | Agent | Mode | Skill required |
|---|---|---|---|---|
| 5.1 | Implement auto-copy: on pipeline completion, format transcript as plain text (`Speaker 1: ...` lines) and write via `arboard` | Backend | Build | — (arboard usage is a simple, single-purpose call; no dedicated skill file, but confirm it's triggered only after Phase 2's reconciliation step has finalized speaker labels) |
| 5.2 | Add a visible in-app toast/confirmation ("Transcript copied to clipboard") | Frontend | Build | **`skills/tauri-ipc-contract.md`** if this is driven by a backend-emitted event rather than a local state change |
| 5.3 | Add manual "Copy" and "Export as .txt/.md" buttons as fallback/convenience | Frontend | Build | **`skills/tauri-ipc-contract.md`** |
| 5.4 | Error handling pass: mic permission denied, unsupported file format, model download failure, low disk space | Backend + Frontend | Build | **`skills/audio-capture-lifecycle.md`** for the mic-permission case specifically; **`skills/tauri-ipc-contract.md`** for how errors propagate to the UI (must use the try/catch + user-visible error pattern, never a silent failure) |

### Phase 6 — Packaging & First-Run Experience (Weeks 10–11)

| # | Task | Agent | Mode | Skill required |
|---|---|---|---|---|
| 6.1 | Build PyInstaller spec for the Python sidecar (one-file binary per OS), with explicit hidden imports for ctranslate2/faster_whisper/torch/torchaudio/pyannote.audio/huggingface_hub | DevOps | Build | **`skills/sidecar-packaging.md`** — hidden-imports checklist must be followed exactly, this is the task it exists for |
| 6.2 | Configure `tauri.conf.json` to bundle the sidecar binary and ffmpeg as external resources | DevOps | Build | **`skills/sidecar-packaging.md`** — ffmpeg-as-data-file requirement |
| 6.3 | Build the first-run setup wizard (model download with progress bar, HF token entry) | Frontend + Backend | Build | **`skills/tauri-ipc-contract.md`** (Frontend side); **`skills/sidecar-model-lifecycle.md`** HF token section (Backend/ML side) |
| 6.4 | Configure `tauri-action` GitHub workflow to build signed installers for Windows (MSI/NSIS), macOS (DMG, notarization if available), and Linux (AppImage + .deb) on tagged releases | DevOps | Build | **`skills/sidecar-packaging.md`** — per-OS build requirement; do not assume one PyInstaller build is portable across platforms |
| 6.5 | **Milestone check:** run the packaged binary's `--self-test` flag (real transcription + real diarization call against the actual built binary, not the dev environment) on all 3 target OSes before marking packaging complete | QA | Build | **`skills/sidecar-packaging.md`** — "Validating a build before it ships" section specifically; build success alone is explicitly insufficient per this skill |

### Phase 7 — Open Source Readiness & Launch (Week 12)

| # | Task | Agent | Mode | Skill required |
|---|---|---|---|---|
| 7.1 | Finalize `README.md`: clear install instructions per OS, screenshots/GIF demo, feature list, privacy statement | Documentation | Build | — |
| 7.2 | Write `docs/ARCHITECTURE.md` (expanded version of Section 3 here) for contributors, including a pointer to `skills/*.md` for anyone touching a covered area | Documentation | Build | — |
| 7.3 | Add a `CODE_OF_CONDUCT.md` | Documentation | Build | — |
| 7.4 | Tag `v1.0.0`, publish GitHub Release with built installers attached | DevOps | Build | — |
| 7.5 | Optional: submit to relevant open-source directories (awesome-lists, Product Hunt, Hacker News "Show HN") | Documentation | Build | — |

---

## 7. Post-v1 Roadmap Ideas
- Cross-meeting speaker voice matching using stored embeddings (auto-suggest known speaker names).
- whisper.cpp backend option for Apple Silicon users wanting max Mac performance.
- Meeting summarization via a local LLM (e.g., small Ollama-served model) as an optional add-on.
- Export formats: PDF, DOCX, SRT/VTT subtitles.
- Plugin system for optional cloud sync (kept strictly opt-in, separate from core).

---

## 8. Risks & Mitigations

| Risk | Mitigation |
|---|---|
| pyannote's gated HF model adds install friction | Clear first-run wizard with direct link + token field; document as a one-time step in README. Covered by `skills/sidecar-model-lifecycle.md`. |
| Sidecar process complexity (Rust ⇄ Python) | Keep the IPC protocol dead simple (localhost HTTP + JSON); document it well for contributors. Covered by `skills/tauri-ipc-contract.md`. |
| Large model downloads on first run | Show progress, allow model-size choice (tiny/small for low-end machines), cache once downloaded |
| Diarization accuracy on noisy audio | Default to Community-1 (best open model); document limitation, leave door open for a future cloud-accuracy toggle |
| Silent speaker misattribution at boundary changes | Covered directly by `skills/diarization-reconciliation.md` — overlap-duration matching, not start-time containment |
| Packaged sidecar crashing only on end-user machines, not in dev | Covered directly by `skills/sidecar-packaging.md` — explicit hidden imports + mandatory `--self-test` validation gate |
| Cross-platform installer signing (macOS notarization costs money) | Ship unsigned Linux/Windows builds initially with clear "unsigned" warning docs; prioritize macOS signing once there's traction/funding |