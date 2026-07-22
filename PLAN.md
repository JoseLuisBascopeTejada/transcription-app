# Meeting Transcriber — Project Plan

Open-source, cross-platform desktop app for live and file-based meeting transcription
with speaker diarization, local-first storage, and one-click installation.

- **License:** MIT (permissive, maximizes adoption and contribution; avoids GPL
  copyleft friction for downstream users who may want to embed/fork it)
- **Repository host:** GitHub
- **Privacy stance:** 100% local processing by default. No account, no telemetry,
  no cloud dependency required to use the app. Cloud STT is an opt-in toggle only.

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

## 4. Repository Structure

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

## 5. Detailed Step-by-Step Breakdown

### Phase 0 — Project Setup (Week 1)
- [ ] Initialize GitHub repo, MIT `LICENSE`, `README.md`, `.gitignore`.
- [ ] Scaffold Tauri 2 project with React + TypeScript template.
- [ ] Set up `sidecar/` Python project (venv, `requirements.txt`: faster-whisper, pyannote.audio, ffmpeg-python).
- [ ] Configure GitHub Actions CI: lint (`clippy`, `eslint`, `ruff`), basic build check on push/PR.
- [ ] Write `CONTRIBUTING.md` and issue/PR templates for open-source contributors.

### Phase 1 — Core Audio Pipeline (Weeks 2–3)
- [ ] Rust: implement mic capture via `cpal`, write raw PCM to a temp WAV file.
- [ ] Rust: implement file-upload handling (accept mp3/wav/mp4/m4a) and pass path to sidecar.
- [ ] Python: `transcribe.py` — load faster-whisper model, expose a simple stdio/HTTP interface accepting an audio path and mode (`live-chunk` | `full-file`), returning JSON segments with timestamps.
- [ ] Rust ⇄ Python: implement sidecar process management (spawn on app start, communicate via local HTTP on `127.0.0.1:<random-port>` for simplicity and easy debugging).
- [ ] Wire up ffmpeg normalization step (any input → 16kHz mono WAV) before every transcription call.
- [ ] **Milestone:** can record from mic OR upload a file and get a raw (undiarized) transcript printed to the UI.

### Phase 2 — Speaker Diarization (Weeks 4–5)
- [ ] Python: `diarize.py` — load pyannote.audio Community-1 pipeline, run on a completed recording, return speaker turn segments (start, end, speaker_id).
- [ ] Python: `reconcile.py` — merge Whisper's word/segment timestamps with diarization turns to produce final `[Speaker N] text` structure.
- [ ] Handle the HF token setup flow (config file, validation, clear error messaging if missing/invalid).
- [ ] **Milestone:** a finished recording produces a fully speaker-labeled transcript.

### Phase 3 — Local Storage (Week 6)
- [ ] Rust: SQLite schema migration setup (`rusqlite` + `refinery` or hand-rolled migrations).
- [ ] Implement write path: meeting → speakers → segments, on pipeline completion.
- [ ] Implement read/query path: meeting history list, full transcript by id, search across meetings.
- [ ] Implement optional Opus audio retention (ffmpeg re-encode) behind a settings toggle; default = discard audio.
- [ ] **Milestone:** transcripts persist across app restarts; history view lists past meetings.

### Phase 4 — GUI / UX (Weeks 7–8)
- [ ] Live Mode screen: record/stop controls, live-scrolling partial transcript, waveform/level indicator.
- [ ] File Mode screen: drag-and-drop or file picker, progress indicator during processing.
- [ ] Transcript view: speaker-labeled, click-to-rename speaker labels (propagates across the meeting), timestamps toggle.
- [ ] History/library view: searchable list of past meetings.
- [ ] Settings screen: model selection, cloud STT opt-in toggle (with clear privacy explanation), audio retention toggle, HF token management.
- [ ] **Milestone:** feature-complete UI wired to the full pipeline.

### Phase 5 — Clipboard & Polish (Week 9)
- [ ] Implement auto-copy: on pipeline completion, format transcript as plain text (`Speaker 1: ...` lines) and write via `arboard`.
- [ ] Add a visible in-app toast/confirmation ("Transcript copied to clipboard").
- [ ] Add manual "Copy" and "Export as .txt/.md" buttons as fallback/convenience.
- [ ] Error handling pass: mic permission denied, unsupported file format, model download failure, low disk space.

### Phase 6 — Packaging & First-Run Experience (Weeks 10–11)
- [ ] Build PyInstaller spec for the Python sidecar (one-file binary per OS).
- [ ] Configure `tauri.conf.json` to bundle the sidecar binary as an external resource.
- [ ] Build the first-run setup wizard (model download with progress bar, HF token entry).
- [ ] Configure `tauri-action` GitHub workflow to build signed installers for Windows (MSI/NSIS), macOS (DMG, notarization if code-signing cert available), and Linux (AppImage + .deb) on tagged releases.
- [ ] **Milestone:** a clean machine can download the installer from GitHub Releases and run a full transcription within minutes, no manual setup beyond the wizard.

### Phase 7 — Open Source Readiness & Launch (Week 12)
- [ ] Finalize `README.md`: clear install instructions per OS, screenshots/GIF demo, feature list, privacy statement.
- [ ] Write `docs/ARCHITECTURE.md` (expanded version of section 3 here) for contributors.
- [ ] Add a `CODE_OF_CONDUCT.md`.
- [ ] Tag `v1.0.0`, publish GitHub Release with built installers attached.
- [ ] Optional: submit to relevant open-source directories (awesome-lists, Product Hunt, Hacker News "Show HN").

---

## 6. Post-v1 Roadmap Ideas
- Cross-meeting speaker voice matching using stored embeddings (auto-suggest known speaker names).
- whisper.cpp backend option for Apple Silicon users wanting max Mac performance.
- Meeting summarization via a local LLM (e.g., small Ollama-served model) as an optional add-on.
- Export formats: PDF, DOCX, SRT/VTT subtitles.
- Plugin system for optional cloud sync (kept strictly opt-in, separate from core).

---

## 7. Risks & Mitigations

| Risk | Mitigation |
|---|---|
| pyannote's gated HF model adds install friction | Clear first-run wizard with direct link + token field; document as a one-time step in README |
| Sidecar process complexity (Rust ⇄ Python) | Keep the IPC protocol dead simple (localhost HTTP + JSON); document it well for contributors |
| Large model downloads on first run | Show progress, allow model-size choice (tiny/small for low-end machines), cache once downloaded |
| Diarization accuracy on noisy audio | Default to Community-1 (best open model); document limitation, leave door open for a future cloud-accuracy toggle |
| Cross-platform installer signing (macOS notarization costs money) | Ship unsigned Linux/Windows builds initially with clear "unsigned" warning docs; prioritize macOS signing once there's traction/funding |