---
name: sidecar-model-lifecycle
description: Use this skill whenever writing or editing code that loads faster-whisper or pyannote.audio models, runs transcription/diarization inference, or handles sidecar startup/warm-up in the Python ML sidecar. Trigger this even for small changes to sidecar/transcribe.py, sidecar/diarize.py, or the sidecar's HTTP server entry point — this skill exists to prevent the single most damaging performance bug in this class of project, which is reloading a Whisper or pyannote model on every request instead of once at sidecar startup.
---

# Sidecar Model Lifecycle — Load Once, Never Per-Request

## The critical rule

**`WhisperModel(...)` and `Pipeline.from_pretrained(...)` must each run
exactly once, when the sidecar process starts, and the loaded objects must
be kept in module-level variables and reused for every transcription or
diarization call.**

Loading `distil-large-v3` or the pyannote Community-1 pipeline from disk
takes several seconds each. If either gets rebuilt inside a request
handler, every live-mode chunk and every file-mode job pays that cost
again — the app will feel completely broken even though the underlying
transcription itself is fast.

### Wrong (do not do this)

```python
@app.post("/transcribe")
def transcribe(req: TranscribeRequest):
    model = WhisperModel("distil-large-v3", device="auto")  # ❌ reloaded every call
    segments, _ = model.transcribe(req.audio_path)
    return {"segments": list(segments)}
```

### Right

```python
# sidecar/models.py
from faster_whisper import WhisperModel
from pyannote.audio import Pipeline

_whisper_model = None
_diarization_pipeline = None

def get_whisper_model():
    global _whisper_model
    if _whisper_model is None:
        _whisper_model = WhisperModel("distil-large-v3", device="auto", compute_type="int8")
    return _whisper_model

def get_diarization_pipeline():
    global _diarization_pipeline
    if _diarization_pipeline is None:
        _diarization_pipeline = Pipeline.from_pretrained(
            "pyannote/speaker-diarization-community-1",
            use_auth_token=get_hf_token(),  # from local config, see hf token handling below
        )
    return _diarization_pipeline

def warm_up():
    """Run a dummy inference on both models so the first real request isn't slow."""
    import numpy as np
    import soundfile as sf
    import tempfile
    silence = np.zeros(16000, dtype=np.float32)  # 1 second at 16kHz
    with tempfile.NamedTemporaryFile(suffix=".wav") as f:
        sf.write(f.name, silence, 16000)
        list(get_whisper_model().transcribe(f.name))
        get_diarization_pipeline()(f.name)
```

```python
# sidecar/main.py (FastAPI entry point)
from models import warm_up

@app.on_event("startup")
def startup_event():
    warm_up()   # ✅ runs once, before the sidecar accepts real requests from the Rust shell
```

## Checklist before finishing any task touching transcribe.py, diarize.py, or the sidecar entry point

- [ ] Is each model held in a module-level variable, not created inside a function that runs per-request?
- [ ] Is `warm_up()` wired into the sidecar's startup event, not called lazily on the first real transcription?
- [ ] Does `warm_up()` use a tiny synthetic audio clip (1 second of silence) rather than requiring a real file?
- [ ] Do `transcribe()`/`diarize()` request handlers call `get_whisper_model()`/`get_diarization_pipeline()` rather than constructing their own instance?
- [ ] Is the exact model name pinned literally (`distil-large-v3`, `pyannote/speaker-diarization-community-1`) rather than left as a variable that could silently drift between calls?

## HF token handling — don't hardcode, don't skip validation

The pyannote pipeline requires a Hugging Face token (see PLAN.md §3.5 setup
wizard). Read it from local app config at startup, not from an environment
variable set ad hoc, and fail loudly with a clear error if it's missing —
never silently fall back to an unauthenticated request that will hang or
403.

```python
def get_hf_token() -> str:
    token = load_local_config().get("hf_token")
    if not token:
        raise RuntimeError(
            "No Hugging Face token configured. Run the first-run setup wizard "
            "or set it in Settings before using diarization."
        )
    return token
```

## Testing this layer without slow model loads

In `test_sidecar.py`, load models once in a `conftest.py` fixture scoped at
`session` level, not `function` level — otherwise every test that touches
either model reloads it and the suite becomes as slow as the bug this skill
prevents.

```python
@pytest.fixture(scope="session")
def whisper_model():
    from models import get_whisper_model
    return get_whisper_model()
```