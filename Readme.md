# Meeting Transcriber

Open-source, cross-platform desktop app for live and file-based meeting transcription with speaker diarization, local-first storage, and one-click installation.

## Features

- **Live microphone transcription** — record from your mic and watch the transcript stream in real time.
- **File-based transcription** — drag-and-drop or pick an mp3, wav, mp4, or m4a file and get a full transcript.
- **Automatic speaker labeling** — pyannote.audio identifies different speakers (Speaker 1, Speaker 2…); rename them inline.
- **100% local by default** — no account, no telemetry, no cloud dependency required. Cloud STT is an opt-in toggle only.
- **Clipboard auto-copy** — transcript is automatically copied to your clipboard on completion.
- **Persistent history** — all transcripts stored in a local SQLite database, searchable and browsable.
- **Cross-platform** — Windows, macOS, and Linux from a single repo.

## Install

| OS | Installer | Notes |
|---|---|---|
| Windows | `.msi` or `.exe` from [Releases](https://github.com/meeting-transcriber/meeting-transcriber/releases) | Windows 10+ |
| macOS | `.dmg` from [Releases](https://github.com/meeting-transcriber/meeting-transcriber/releases) | macOS 11+ (unsigned until notarization is set up) |
| Linux | `.AppImage` or `.deb` from [Releases](https://github.com/meeting-transcriber/meeting-transcriber/releases) | Ubuntu 20.04+ / most distros |

Download the installer for your OS from the GitHub Releases page. No Python or other dependencies needed — everything is bundled.

### First-launch setup

On first launch a setup wizard will:

1. Download the Whisper model (~750 MB) with a progress bar.
2. Prompt for a free [Hugging Face](https://huggingface.co/) token (one-time, needed for pyannote's gated model). Stored locally, never transmitted.
3. Download the pyannote diarization pipeline weights.

All subsequent runs are fully offline.

## Privacy

- All audio processing happens on your machine.
- No data is sent to any server unless you explicitly enable the cloud STT toggle in Settings.
- The Hugging Face token is stored in your local app config only.
- Audio is discarded after transcription by default (optional Opus retention available in Settings).

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      Tauri Shell (Rust)                      │
│  React UI · Audio Capture (cpal) · SQLite Store (rusqlite)  │
└──────────────────────┬──────────────────────────────────────┘
                       │  local IPC (stdio/HTTP over localhost)
                       ▼
       ┌───────────────────────────────────┐
       │  Python Sidecar (PyInstaller bin)  │
       │  faster-whisper · pyannote.audio   │
       │  ffmpeg (normalize audio)          │
       └───────────────────────────────────┘
```

The Tauri (Rust + React) shell handles the GUI, audio capture, file I/O, and SQLite storage. A Python sidecar binary — built with PyInstaller so end users never install Python — runs faster-whisper for transcription and pyannote.audio for speaker diarization. The two communicate over a local HTTP interface.

## Development

### Prerequisites

- [Node.js](https://nodejs.org/) ≥ 18
- [Rust](https://www.rust-lang.org/tools/install) stable toolchain
- Python 3.11
- [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/) (system-level dependencies per OS)

### Frontend / Rust shell

```bash
npm install
npm run tauri dev
```

### Python sidecar (for local development without the bundled binary)

```bash
cd sidecar
python -m venv .venv
# Windows
.venv\Scripts\activate
# macOS / Linux
source .venv/bin/activate

pip install -r requirements.txt
```

### Running tests

```bash
# Frontend lint
npx eslint src/

# Rust checks
cargo clippy --manifest-path src-tauri/Cargo.toml

# Python lint
ruff check sidecar/
```

See [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md) for full contribution guidelines, including the `skills/*.md` bug-prevention files that agents must read before touching covered areas.

## Roadmap

See [PLAN.md](PLAN.md) for the full project plan. Highlights for post-v1:

- Cross-meeting speaker voice matching using stored embeddings
- whisper.cpp backend for Apple Silicon
- Meeting summarization via a local LLM (Ollama)
- Export formats: PDF, DOCX, SRT/VTT subtitles
- Optional cloud sync plugin

## License

[MIT](LICENSE) — Copyright (c) 2026 Meeting Transcriber Contributors

## Contributing

Contributions welcome! Please read [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md) before submitting a PR. If you're touching audio capture, SQLite, Tauri IPC, the Python sidecar, or diarization logic, please read the relevant `skills/*.md` file first — these exist to prevent entire categories of bugs.
