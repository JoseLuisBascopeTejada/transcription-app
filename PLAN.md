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
- Live meeting transcription (Teams/Zoom/Google Meet, etc.) — **primary
  source is the call's system audio (other participants), captured via
  loopback**. The user's own microphone is an **optional toggle**, off by
  default, that adds a second synchronized stream when enabled — not a
  mandatory always-on second source. Plus pre-recorded file transcription
  (mp3, wav, mp4, m4a).
- Automatic speaker labeling: when the mic toggle is on, that stream is
  always labeled as the user directly (no diarization needed, since the
  source is already known); the system-audio stream is diarized and
  labeled Speaker 1, Speaker 2… with user rename support.
- **Background presence**: the app can run minimized to the system tray,
  detect when the user joins a supported meeting app, and surface a small
  always-on-top floating button suggesting "Start transcribing?" rather
  than requiring the user to open the app manually every time.
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
| Speech-to-text | **faster-whisper** (CTranslate2 backend), model default `small` | Confirmed via real benchmarking during development: `distil-large-v3` has severely degraded multilingual language detection (confidently mis-detects Spanish as English, even with the language forced explicitly), while `small` correctly detects/transcribes non-English speech AND runs faster on CPU — see §8 risks. MIT-licensed, works identically on Win/Mac/Linux. User-selectable in Settings (Phase 4) for those wanting higher accuracy at the cost of speed |
| Speaker diarization | **pyannote.audio 4.0 (Community-1 pipeline)** | Open-source, local, current best-in-class open diarization accuracy; requires a free Hugging Face token (handled in first-run setup wizard) |
| Audio decode/normalize | **ffmpeg** (bundled binary) | Converts any input (mp3/mp4/m4a/wav) to 16kHz mono WAV for Whisper |
| Local database | **SQLite** (via `rusqlite` in Rust) | Single-file, zero-config, ACID, queryable, negligible size for text data |
| Audio storage (optional) | **Opus** codec, low bitrate, via ffmpeg | ~40x smaller than raw WAV if user opts to retain audio |
| Clipboard | **arboard** (Rust crate) | Cross-platform clipboard write, no extra runtime |
| Live audio capture (mic) | **cpal** (Rust crate) | Cross-platform mic access matching Tauri's Rust core |
| Live audio capture (system/loopback) | **`wasapi` crate** (Rust, Windows-only for v1) | Captures other meeting participants' audio via WASAPI loopback — `cpal` alone does not expose loopback capture on Windows. macOS/Linux equivalents (Core Audio taps / PulseAudio-PipeWire monitor source) are a v1.x follow-up, see Section 3.6 |
| CI/CD & release builds | **GitHub Actions** (`tauri-action`) | Builds signed installers for all 3 OSes automatically on tag/release |
| Packaging Python sidecar | **PyInstaller** | Produces a self-contained Python executable with faster-whisper/pyannote bundled, no user-side pip install |
| Meeting-app detection | **`sysinfo` crate** (Rust) | Lightweight process-list polling to detect known meeting-app executables (Teams, Zoom); see §3.7 |
| System tray + overlay window | **Tauri 2 built-in tray icon API + a second `WebviewWindow`** | No extra crate needed — Tauri natively supports a tray icon and multiple windows (main app window + a small always-on-top overlay window), see §3.7 |

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

### 3.2 Pipeline flow

**File Mode** (single audio source, unchanged):
```
Uploaded file
   → ffmpeg normalize → 16kHz mono WAV
   → faster-whisper (single batch pass)
   → [on completion] pyannote.audio batch diarization over the full recording
   → Reconciliation: align diarization speaker turns to Whisper segment timestamps
   → Write meeting + segments + speakers to SQLite
   → Format transcript as plain text → write to system clipboard (arboard)
   → UI renders final labeled transcript, speakers renamable inline
```

**Live Mode** (system-audio loopback is always on and is the primary
source; mic is an optional toggle that adds a second parallel stream when
enabled — both share the recording-start clock from §3.6 whenever mic is on):
```
System-audio loopback stream (wasapi, Windows v1)   Mic stream (cpal) — ONLY if
   → always captured — this IS the meeting            the mic toggle is enabled
   → 16kHz mono, written to its own temp                 → 16kHz mono, written to
     WAV file, timestamped from                            its own temp WAV file,
     recording start                                       SAME start timestamp as
                                                             the system-audio stream
   → faster-whisper (incremental,                        → faster-whisper
     VAD-chunked ~5–10s windows,                            (incremental, same
     streamed to UI as partial text —                       chunking, always
     speaker not yet known, shown as                        attributed to "You" —
     "..." placeholder until                                no diarization needed
     diarization runs post-meeting)                         since the source is
                                                              already known)

                    │                              │
                    │        [meeting ends]         │
                    └──────────────┬───────────────┘
                                    ▼
        pyannote.audio batch diarization — runs ONLY on the
        system-audio stream (the mic stream's speaker is already
        known and is never sent through diarization)
                                    │
                                    ▼
        Reconciliation: align diarization speaker turns to the
        system-audio stream's Whisper segment timestamps
        (Speaker 1, Speaker 2, ... assigned here)
                                    │
                                    ▼
        Merge (SKIPPED if the mic toggle was off — in that case the
        reconciled system-audio transcript above IS the final transcript):
        interleave the mic stream's "You"-labeled segments with the
        system-audio stream's diarized segments into one chronological
        transcript, using each segment's shared timeline offset from 3.6
                                    │
                                    ▼
        Write meeting + segments + speakers to SQLite
                                    │
                                    ▼
        Format transcript as plain text → write to system clipboard (arboard)
                                    │
                                    ▼
        UI renders final labeled transcript, speakers renamable inline
```

**Design note on diarization timing:** true real-time diarization is still an
immature, mostly cloud-only capability. This app diarizes *after* the recording
finishes (batch, on the full audio) even in Live Mode — this keeps accuracy high
without needing a cloud dependency, while the live transcript itself still streams
in near-real-time so the user isn't staring at a blank screen.

**Design note on the "You" speaker:** the mic stream never goes through
diarization — its speaker identity is known by construction (it's whichever
device the user selected as their input). This is both simpler and more
accurate than diarizing the user's own voice out of a mixed signal, and it
means diarization workload is only ever spent on distinguishing *other*
participants from each other.

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
  1. Downloads the selected Whisper model (`small` by default, size ~500MB) with a progress bar.
  2. Prompts for a free Hugging Face token (one-time, needed for pyannote's gated model) with a direct link and copy-paste field — stored locally in the app config, never transmitted anywhere else.
  3. Downloads pyannote's Community-1 pipeline weights.
- All subsequent runs are fully offline.

### 3.6 Dual-stream capture: clock sync and cross-platform loopback

**Clock synchronization (required, not optional):** both streams must be
started from the same recording-start instant and timestamped against one
shared clock. Practically: `start_recording` records a single
`recording_started_at` instant, and both the mic thread and the
system-loopback thread compute their segment timestamps as an offset from
that one shared instant — never from each stream's own "time since I
personally started," since the two threads will not start in the exact
same millisecond and that drift would misorder the merged transcript.

**Loopback capture is fundamentally OS-specific — there is no single
cross-platform API for it, unlike microphone capture:**

| OS | Mechanism | v1 status |
|---|---|---|
| Windows | WASAPI loopback mode (open the default *output* device in capture/loopback mode) via the `wasapi` crate | **Supported in v1** |
| macOS | No user-facing OS loopback API; requires either a virtual audio driver (e.g. BlackHole, user must install separately) or `ScreenCaptureKit`'s audio-capture API (macOS 13+, requires screen-recording permission even though only audio is captured) | **Deferred — mic-only on macOS until a follow-up task**; document this limitation clearly in the README so macOS users aren't surprised |
| Linux | PulseAudio/PipeWire expose a `.monitor` source for the default sink, capturable like any other input device | **Deferred — mic-only on Linux until a follow-up task**; the mechanism is well-understood, just not in v1's initial scope |

Because of this, **v1 ships full Live Mode (system-audio capture, the core
value proposition) on Windows only.** On macOS/Linux, v1 cannot capture the
call's other participants at all — only the user's own mic, if the user
enables that toggle. This is a real reduction in usefulness on those
platforms, not a cosmetic gap, since system audio is now the primary
feature. Show a clear, honest in-app notice on macOS/Linux explaining this
rather than letting the user assume Live Mode works the same everywhere.
Closing this gap for macOS/Linux is tracked in Section 7 (Roadmap) as a
near-term priority, not a someday item.

### 3.7 Background detection & floating overlay suggestion

**Goal:** the app can run minimized to the system tray, detect when the
user has joined a supported meeting app, and surface a small always-on-top
floating button suggesting "Start transcribing?" — without requiring the
user to open the main window first.

**Detection mechanism — two tiers, since there is no single reliable
signal that works for both native apps and browser-based meetings:**

| Meeting type | Detection method | Reliability |
|---|---|---|
| Native desktop apps (Teams, Zoom) | Poll running processes (via the `sysinfo` crate) every few seconds for known process names (`Teams.exe`, `ms-teams.exe`, `Zoom.exe`, etc., kept in a small configurable list — meeting-app executables change names periodically and this list will need maintenance) | High — a running process is unambiguous |
| Browser-based meetings (Google Meet, and Teams/Zoom-in-browser) | **Not reliably auto-detectable in v1.** There is no OS-level signal that a specific browser tab is a video call versus any other tab playing audio. Do not attempt to guess this from window titles or audio-session state — the false-positive rate would make the floating button annoying rather than helpful | Not supported for auto-detection; the tray icon remains available for the user to start manually |

**Floating overlay window:** a separate small, frameless, always-on-top,
transparent Tauri window (not the main app window) positioned at the right
edge of the primary display. Appears when a native meeting app is detected,
offers Start/Dismiss, and auto-dismisses after a short timeout if ignored.
Clicking Start reuses the same `start_recording`/loopback-start commands
from Phase 1 — this is a UI trigger, not a new recording pathway.

**System tray icon:** persistent, with a right-click menu (Open app,
Start/Stop recording, Quit). This is the fallback entry point for
browser-based meetings and for any case where auto-detection didn't fire.

**Hard requirements, not optional polish:**
- Process polling must be lightweight (a several-second interval, not a
  tight loop) — this runs continuously in the background and must not be
  a noticeable drain on CPU or battery.
- The background-detection feature itself must be **fully toggleable off**
  in Settings, defaulting to a state the user explicitly confirms during
  first-run setup — continuously scanning running processes is a
  meaningful privacy-adjacent behavior even though no data leaves the
  device, and users should knowingly opt into it, not discover it later.
- The overlay window must never steal keyboard focus from the meeting
  app — it's a suggestion, not an interruption.
- Both the tray icon and the overlay window must be cleanly destroyed on
  app quit — no orphaned background windows/processes left running.

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
├── SKILLS.md
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

**Execution order matters in this phase and must be followed exactly —
running 0.2 before 0.1's placeholder files are cleared causes the Tauri
scaffolder to collide with hand-authored root files:**

`0.2 (scaffold) → 0.1 (docs, finalized against the scaffolded structure) → 0.3 → 0.4 → 0.5 → 0.6`

| # | Task | Agent | Mode | Skill required |
|---|---|---|---|---|
| 0.1 | Finalize `LICENSE`, `README.md`, `.gitignore` at the repo root, **after** task 0.2's scaffold exists — merge with (not overwrite) any `.gitignore`/`README.md` the scaffolder generated | Documentation | Build | — |
| 0.2 | Scaffold Tauri 2 project with React + TypeScript template. **The scaffolder must never run directly against a directory containing existing hand-authored files** (`README.md`, `.gitignore`, `LICENSE`, `PLAN.md`, `SKILLS.md`, `.agents/`). Scaffold into a clean temp directory, then move only the scaffold-generated project files (`src-tauri/`, `src/`, `package.json`, `tsconfig.json`, etc.) into the project root. Verification must be headless (`cargo check`, `npx tsc --noEmit`) — do not rely on launching `npm run tauri dev`'s GUI window as a pass/fail signal, since agent environments may have no display server | Frontend | Build | — |
| 0.3 | Set up `sidecar/` Python project (venv, `requirements.txt`: faster-whisper, pyannote.audio, ffmpeg-python) | ML/Sidecar | Build | — |
| 0.4 | Configure GitHub Actions CI with two separate concerns, kept explicitly distinct: (a) lint (`clippy`, `eslint`, `ruff`) and (b) a **compile-only** check (`cargo check --all-targets` + `npm run build`). Do **not** use `npx tauri build` for this — that produces a full installer bundle, belongs to Phase 6/`release.yml`, and can't even validate Windows/macOS packaging when run on an `ubuntu-latest` runner | DevOps | Build | — |
| 0.5 | Write `CONTRIBUTING.md` and issue/PR templates, including a note pointing contributors to `.agents/skills/*/SKILLS.md` before touching covered areas | Documentation | Build | — |
| 0.6 | Break Phase 1–3 into atomic per-agent tasks, confirm no task spans two agents' responsibilities | Orchestrator | Plan | — |

### Phase 1 — Core Audio Pipeline (Weeks 2–3)

| # | Task | Agent | Mode | Skill required |
|---|---|---|---|---|
| 1.1 | Implement mic capture via `cpal`; write raw PCM to a temp WAV file, resampled/downmixed to 16kHz mono regardless of device native format. Timestamps must be offsets from a single shared `recording_started_at` instant (see PLAN.md §3.6), not from this thread's own start time, so it can later be merged against the loopback stream from task 1.1b | Backend | Build | **`skills/audio-capture-lifecycle.md`** — read in full before writing the capture stream setup or the WAV write path |
| 1.1b | **(New — Windows v1 only)** Implement system-audio loopback capture via the `wasapi` crate, opening the default output device in loopback mode. Same 16kHz mono target and same shared `recording_started_at` clock as task 1.1 — both streams must be startable/stoppable together via one pair of Tauri commands, not two independently-timed recordings. Gate this feature behind a platform check; on macOS/Linux, surface a clear "system audio capture not yet available on this OS" notice instead of silently doing nothing (see PLAN.md §3.6) | Backend | Build | **`skills/audio-capture-lifecycle.md`** — this skill's rules (never let the stream drop out of scope, always resample to the fixed target format) apply identically to the loopback stream |
| 1.2 | Implement file-upload handling (accept mp3/wav/mp4/m4a); normalize via ffmpeg to 16kHz mono WAV before passing to sidecar | Backend | Build | — (ffmpeg normalization only, no cpal involved; skill not required, but must match the same 16kHz mono target defined in 1.1) |
| 1.3 | Python: `transcribe.py` — load faster-whisper model once at sidecar startup, expose stdio/HTTP interface accepting an audio path + mode (`live-chunk` \| `full-file`), return JSON segments with timestamps | ML/Sidecar | Build | **`skills/sidecar-model-lifecycle.md`** — model must be loaded once at startup per this skill's checklist, not per request |
| 1.4 | Rust ⇄ Python: implement sidecar process management (spawn on app start, communicate via local HTTP on `127.0.0.1:<random-port>`); the request contract must support submitting either stream (mic or system-audio) independently, tagged by source, since they're transcribed separately per PLAN.md §3.2 | Backend | Build | **`skills/tauri-ipc-contract.md`** applies to the Rust-side command wrapping this call; define and document the exact JSON request/response shape before implementation |
| 1.5 | Wire the recording start/stop Tauri commands to the frontend (one start/stop action controls both streams together on Windows; mic-only on macOS/Linux per §3.6) | Frontend | Build | **`skills/tauri-ipc-contract.md`** — command names and payload types must be defined jointly with task 1.4's output before this task starts |
| 1.6 | **Milestone check:** can record from mic (+ system audio on Windows) OR upload a file and get raw (undiarized) transcripts printed to the UI, correctly tagged by source stream | QA | Build | **`skills/test-fixture-scoping.md`** — any automated test written here must use session-scoped model fixtures per this skill |

### Phase 2 — Speaker Diarization (Weeks 4–5)

| # | Task | Agent | Mode | Skill required |
|---|---|---|---|---|
| 2.1 | Python: `diarize.py` — load pyannote.audio Community-1 pipeline once at sidecar startup, run on a completed recording, return speaker turn segments (start, end, speaker_id). For Live Mode meetings, this runs ONLY on the system-audio loopback recording — never on the mic recording, whose speaker is already known (see PLAN.md §3.2) | ML/Sidecar | Build | **`skills/sidecar-model-lifecycle.md`** — same load-once rule applies to the diarization pipeline as to Whisper |
| 2.2 | Python: `reconcile.py` — merge Whisper segment timestamps with diarization turns into final `[Speaker N] text` structure, using overlap-duration matching (not start-time containment). This applies only to the system-audio stream's segments | ML/Sidecar | Build | **`skills/diarization-reconciliation.md`** — read in full; this is explicitly flagged as the highest-risk logic in the project |
| 2.2b | **(New)** Python or Rust (implementer's choice, but must live in the same module family as reconcile.py for discoverability): merge the mic stream's "You"-labeled segments (task 1.1's output, transcribed directly with no diarization) with the system-audio stream's diarized-and-reconciled segments (task 2.2's output) into one chronological list, ordered by each segment's shared timeline offset from §3.6. Applies to Live Mode meetings only — File Mode has a single stream and skips this step entirely | ML/Sidecar | Build | **`skills/diarization-reconciliation.md`** — the overlap-duration principle doesn't directly apply here (these two streams don't overlap in speaker identity, only in time), but the skill's guidance on treating timestamp units consistently and never assuming two independently-produced timestamp sets already agree still applies directly |
| 2.3 | Handle the Hugging Face token setup flow (config file, validation, clear error messaging if missing/invalid) | ML/Sidecar | Build | **`skills/sidecar-model-lifecycle.md`** — HF token handling section specifically |
| 2.4 | **Milestone check:** a finished recording produces a fully speaker-labeled transcript, with the user's own turns correctly attributed and interleaved chronologically against other participants' diarized turns | QA | Build | **`skills/test-fixture-scoping.md`** and **`skills/diarization-reconciliation.md`** — reconciliation tests must use hand-constructed timestamp fixtures per the latter, not real audio |

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

### Phase 6 — Background Meeting Detection & Floating Overlay (Weeks 10–11)

| # | Task | Agent | Mode | Skill required |
|---|---|---|---|---|
| 6.1 | Configure Tauri so closing the main window minimizes to system tray instead of quitting; add a persistent tray icon with a context menu (Open app, Start/Stop recording, Quit) | Backend | Build | — (Tauri's built-in tray API; no dedicated skill file yet — flag to Orchestrator if issues recur across this phase, a `background-detection-lifecycle` skill file should be authored following the same pattern as the other 7) |
| 6.2 | Implement process-polling meeting detection via the `sysinfo` crate: check the running process list every few seconds against a configurable list of known meeting-app executable names (Teams, Zoom); emit a Tauri event on detected/lost | Backend | Build | — |
| 6.3 | Build the floating overlay window: a separate small, frameless, always-on-top, transparent `WebviewWindow` positioned at the right edge of the primary display, shown on the detected event from 6.2, with Start/Dismiss actions and an auto-dismiss timeout | Frontend | Build | **`skills/tauri-ipc-contract.md`** — the overlay's Start action must call the exact same `start_recording`/loopback-start commands from Phase 1, not a duplicate pathway |
| 6.4 | Add a Settings toggle to fully disable background detection, defaulting to a state explicitly confirmed during first-run setup (see task 6.3 of the renumbered Phase 7 first-run wizard) — this must be an honest, visible opt-in, not a silent default-on background scan | Frontend | Build | **`skills/tauri-ipc-contract.md`** |
| 6.5 | Add launch-at-system-startup option (so the tray presence persists across reboots without the user manually reopening the app) | Backend | Build | — |
| 6.6 | **Milestone check:** opening Teams or Zoom triggers the floating overlay within a few seconds; dismissing it doesn't start recording; the tray icon's manual Start works for browser-based meetings where auto-detection doesn't apply (per §3.7); disabling the feature in Settings stops all process polling entirely; app quit leaves no orphaned tray icon or overlay window | QA | Build | — |

### Phase 7 — Packaging & First-Run Experience (Weeks 12–13)

| # | Task | Agent | Mode | Skill required |
|---|---|---|---|---|
| 7.1 | Build PyInstaller spec for the Python sidecar (one-file binary per OS), with explicit hidden imports for ctranslate2/faster_whisper/torch/torchaudio/pyannote.audio/huggingface_hub | DevOps | Build | **`skills/sidecar-packaging.md`** — hidden-imports checklist must be followed exactly, this is the task it exists for |
| 7.2 | Configure `tauri.conf.json` to bundle the sidecar binary and ffmpeg as external resources | DevOps | Build | **`skills/sidecar-packaging.md`** — ffmpeg-as-data-file requirement |
| 7.3 | Build the first-run setup wizard (model download with progress bar, HF token entry, AND the explicit background-detection opt-in from task 6.4) | Frontend + Backend | Build | **`skills/tauri-ipc-contract.md`** (Frontend side); **`skills/sidecar-model-lifecycle.md`** HF token section (Backend/ML side) |
| 7.4 | Configure `tauri-action` GitHub workflow to build signed installers for Windows (MSI/NSIS), macOS (DMG, notarization if available), and Linux (AppImage + .deb) on tagged releases | DevOps | Build | **`skills/sidecar-packaging.md`** — per-OS build requirement; do not assume one PyInstaller build is portable across platforms |
| 7.5 | **Milestone check:** run the packaged binary's `--self-test` flag (real transcription + real diarization call against the actual built binary, not the dev environment) on all 3 target OSes before marking packaging complete | QA | Build | **`skills/sidecar-packaging.md`** — "Validating a build before it ships" section specifically; build success alone is explicitly insufficient per this skill |

### Phase 8 — Open Source Readiness & Launch (Week 14)

| # | Task | Agent | Mode | Skill required |
|---|---|---|---|---|
| 8.1 | Finalize `README.md`: clear install instructions per OS, screenshots/GIF demo, feature list, privacy statement — including an honest note about the background-detection feature and the Windows-only scope of full Live Mode (§3.6, §3.7) | Documentation | Build | — |
| 8.2 | Write `docs/ARCHITECTURE.md` (expanded version of Section 3 here) for contributors, including a pointer to `skills/*.md` for anyone touching a covered area | Documentation | Build | — |
| 8.3 | Add a `CODE_OF_CONDUCT.md` | Documentation | Build | — |
| 8.4 | Tag `v1.0.0`, publish GitHub Release with built installers attached | DevOps | Build | — |
| 8.5 | Optional: submit to relevant open-source directories (awesome-lists, Product Hunt, Hacker News "Show HN") | Documentation | Build | — |

---

## 7. Post-v1 Roadmap Ideas
- **macOS system-audio capture** via `ScreenCaptureKit`'s audio API (macOS 13+) or documented BlackHole setup, to bring dual-stream Live Mode to parity with Windows.
- **Linux system-audio capture** via the PulseAudio/PipeWire `.monitor` source, same goal.
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
| Mic and system-audio streams drift apart in time, misordering the merged transcript | Both streams timestamp from one shared `recording_started_at` instant, never from their own thread's start time — see §3.6. Verified explicitly in task 2.2b and the Phase 1 milestone check (1.6) |
| macOS/Linux users get no call-audio capture at all in v1 (system audio is Windows-only) | Honest in-app notice per §3.6; tracked as a near-term roadmap priority, not deferred indefinitely |
| Background process polling perceived as invasive, or drains battery if implemented carelessly | Fully toggleable off, explicit first-run opt-in (not silent default-on), lightweight polling interval — all specified as hard requirements in §3.7 |
| Browser-based meetings (Google Meet, browser Teams/Zoom) can't be auto-detected, users may expect the floating overlay to appear and be confused when it doesn't | Documented explicitly in §3.7 as a known v1 limitation; tray icon remains as the manual fallback entry point |
| Diarization is slow on CPU — measured ~2.2x real-time in testing (a 94.5s recording took ~3m24s to diarize) | Live Mode already diarizes post-meeting in the background per §3.2, not blocking the live transcript — surface a clear "Diarizing your meeting… this may take a few minutes" progress state in the UI (Phase 4) rather than a spinner with no context, so the user doesn't assume the app hung. GPU acceleration for pyannote is a Post-v1 roadmap item (§7) for users with compatible hardware |
| Diarization turn count and transcription accuracy degrade sharply on chaotic/overlapping audio (background music, crosstalk, sound effects) — measured 89 diarization turns on a 94.5s noisy test clip, versus clean, turn-taking speech performing well | Confirmed via isolated testing to be a model limitation, not a pipeline bug — clean File Mode audio with a single clear speaker transcribed correctly with the same code path. Document as a known limitation of CPU-tier open models; typical meeting audio (turn-taking, minimal background noise/music) is the primary supported use case, not heavily produced/edited content |