# Meeting Transcriber

A free, open-source, cross-platform desktop app for transcribing meetings —
live from your microphone or from an uploaded recording — with automatic
speaker labeling. Everything runs **locally on your machine**. No account,
no cloud upload, no subscription.

![platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-blue)
![license](https://img.shields.io/badge/license-MIT-green)

<!-- ![demo](docs/demo.gif) -->

## Features

- 🎙️ **Live Mode** — transcribe your microphone in near-real-time.
- 📁 **File Mode** — drop in an mp3, wav, mp4, or m4a and get a transcript.
- 🗣️ **Speaker Diarization** — automatically distinguishes speakers
  (`Speaker 1`, `Speaker 2`, …), with one-click renaming.
- 🔒 **Local-first** — transcription and diarization run on-device by
  default. Nothing leaves your computer unless you explicitly enable the
  optional cloud fallback.
- 💾 **Lightweight storage** — transcripts are stored in a single local
  SQLite file. Audio is discarded after transcription by default (optional
  compressed retention available).
- 📋 **Auto-copy to clipboard** — the finished transcript is copied
  automatically, ready to paste anywhere.

## Install

Download the latest installer for your OS from the
[Releases page](../../releases):

| OS | File |
|---|---|
| Windows | `.msi` installer |
| macOS | `.dmg` |
| Linux | `.AppImage` or `.deb` |

No Python, no pip, no manual dependency setup required — everything the app
needs is bundled in the installer.

### First launch

On first run, a short setup wizard will:
1. Download the local speech-to-text model (~750MB, one-time).
2. Ask for a free Hugging Face token (used only locally, to download the
   open-source diarization model — never transmitted anywhere else). Get one
   at [huggingface.co/settings/tokens](https://huggingface.co/settings/tokens).

After that, the app works fully offline.

## Privacy

By default, **all audio processing happens on your device**. Recordings and
transcripts are never sent anywhere. An optional cloud transcription toggle
exists in Settings for users who want it, but it is off by default and
clearly labeled when enabled.

## Architecture

See [PLAN.md](PLAN.md) and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for
the full technical breakdown. In short:

- **Shell:** Tauri (Rust) + React frontend
- **Transcription:** faster-whisper (local)
- **Diarization:** pyannote.audio (local)
- **Storage:** SQLite (local, single file)

## Development

```bash
# prerequisites: Rust, Node.js, Python 3.11
git clone https://github.com/<your-org>/meeting-transcriber.git
cd meeting-transcriber

# frontend + tauri shell
npm install
npm run tauri dev

# python sidecar (separate terminal, for sidecar development)
cd sidecar
python -m venv .venv && source .venv/bin/activate
pip install -r requirements.txt
python transcribe.py
```

See [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md) for the full contributor
guide, coding conventions, and how to build release installers locally.

## Roadmap

- Cross-meeting speaker voice matching
- whisper.cpp backend for Apple Silicon
- Optional local LLM meeting summaries
- Export to PDF / DOCX / SRT

See [PLAN.md](PLAN.md#6-post-v1-roadmap-ideas) for details.

## License

[MIT](LICENSE) — free to use, modify, and distribute.

## Contributing

Contributions are welcome! Please read
[CONTRIBUTING.md](docs/CONTRIBUTING.md) and check open issues before
submitting a PR.