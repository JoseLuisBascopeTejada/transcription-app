---
name: sidecar-packaging
description: Use this skill whenever writing or editing the PyInstaller build spec for the Python ML sidecar, or the GitHub Actions workflow step that builds it. Trigger this even for small changes to build_sidecar.spec — this skill exists to prevent the most damaging packaging bug in this class of project, which is a sidecar binary that builds successfully and runs fine in development but crashes at runtime on end-user machines because faster-whisper or pyannote's non-obvious dependencies weren't bundled.
---

# Sidecar Packaging — faster-whisper and pyannote Have Hidden Dependencies

## The critical rule

**faster-whisper (via CTranslate2) and pyannote.audio (via PyTorch, torchaudio,
and various HF Hub/transformers internals) both import modules dynamically
at runtime that PyInstaller's static analysis cannot always detect. Every
build must explicitly list these as `hiddenimports`, and every build must
be validated by actually running the packaged binary standalone — a
successful `pyinstaller` build with no errors does NOT mean the binary
works.**

A build that "succeeds" but is missing a hidden import fails only when a
specific code path is hit at runtime (e.g. the first real diarization
call) — which means it can pass a quick smoke test and still crash for a
real user.

### Wrong (do not do this)

```
# build_sidecar.spec
# ❌ relying entirely on PyInstaller's auto-detection with no explicit hiddenimports
a = Analysis(['sidecar/main.py'], pathex=['.'], binaries=[], datas=[])
```

### Right

```python
# build_sidecar.spec
hiddenimports = [
    # CTranslate2 / faster-whisper
    "ctranslate2",
    "faster_whisper",
    "faster_whisper.tokenizer",

    # pyannote.audio and its torch/torchaudio backend
    "pyannote.audio",
    "pyannote.audio.pipelines",
    "torchaudio",
    "torchaudio.backend",
    "torch",

    # HF Hub download machinery used at first-run model download
    "huggingface_hub",
    "huggingface_hub.file_download",
]

a = Analysis(
    ['sidecar/main.py'],
    pathex=['.'],
    binaries=[],
    datas=[
        # ffmpeg binary must be bundled alongside, not assumed to be on the
        # end-user's PATH
        ('bin/ffmpeg', '.'),
    ],
    hiddenimports=hiddenimports,
)
```

## Checklist before finishing any packaging task

- [ ] Does the spec explicitly list hidden imports for `ctranslate2`, `faster_whisper`, `torch`, `torchaudio`, `pyannote.audio`, and `huggingface_hub` — not relying on auto-detection alone?
- [ ] Is the `ffmpeg` binary bundled as a data file rather than assumed to exist on the end-user's system PATH?
- [ ] Was the packaged binary actually executed standalone (not just `pyinstaller` exiting with code 0) — specifically, was a real transcription AND a real diarization call exercised against the built binary, not just against the dev environment?
- [ ] Does the GitHub Actions workflow build the sidecar separately per target OS (win/mac/linux) rather than assuming one build is portable across platforms — PyInstaller output is not cross-platform?

## Validating a build before it ships

Never mark a packaging task complete based on build success alone. Run this
minimal smoke test against the actual packaged binary on each target OS:

```bash
./dist/sidecar --self-test
# should: load both models, run a 1-second dummy transcription AND
# diarization, print "OK", exit 0 — this is the same warm_up() path
# described in the sidecar-model-lifecycle skill, reused here as a
# packaging validation gate
```

If `--self-test` isn't wired up yet, that's a prerequisite task — flag it
to the Orchestrator rather than shipping a build with no way to verify it.

## Failure signs to watch for

- Sidecar works when run via `python main.py` in development but the
  PyInstaller-built binary crashes immediately or on first inference call —
  missing hidden import, almost always in the torch/torchaudio/pyannote
  chain.
- Works on the build machine, fails on a clean test machine — usually a
  bundled data file (ffmpeg, model cache path) that isn't actually included
  in `datas`.
- Build succeeds on one OS's CI runner but not another — each OS needs its
  own PyInstaller run; a Linux-built binary will not run on Windows.