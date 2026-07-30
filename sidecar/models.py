"""Model lifecycle management — load Whisper and diarization pipeline once, reuse for every request."""

from __future__ import annotations

import logging
from pathlib import Path

import numpy as np
from faster_whisper import WhisperModel
from pyannote.audio import Pipeline

logger = logging.getLogger(__name__)

MODEL_NAME = "small"
COMPUTE_TYPE = "int8"

_whisper_model: WhisperModel | None = None
_diarization_pipeline: Pipeline | None = None

_SIDECAR_DIR = Path(__file__).resolve().parent
_HF_TOKEN_PATH = _SIDECAR_DIR / ".hf_token"


def _verify_model(model: WhisperModel) -> bool:
    """Quick test inference to confirm the model can actually run."""
    try:
        silence = np.zeros(16000, dtype=np.float32)
        segments, _ = model.transcribe(silence, language=None)
        _ = list(segments)
        return True
    except Exception:
        return False


def get_whisper_model() -> WhisperModel:
    """Return the cached WhisperModel, loading it on first call."""
    global _whisper_model
    if _whisper_model is None:
        logger.info("Loading Whisper model %s (compute_type=%s)...", MODEL_NAME, COMPUTE_TYPE)
        try:
            model = WhisperModel(MODEL_NAME, device="auto", compute_type=COMPUTE_TYPE)
            if _verify_model(model):
                _whisper_model = model
                logger.info("Whisper model loaded on GPU.")
            else:
                raise RuntimeError("GPU verification failed")
        except Exception as exc:
            logger.warning("GPU unavailable (%s), falling back to CPU.", exc)
            _whisper_model = WhisperModel(MODEL_NAME, device="cpu", compute_type=COMPUTE_TYPE)
            logger.info("Whisper model loaded on CPU.")
    return _whisper_model


def get_hf_token() -> str:
    """Read the Hugging Face token from sidecar/.hf_token.

    Raises RuntimeError if the file is missing or empty — never silently
    falls back to an unauthenticated request.
    """
    if not _HF_TOKEN_PATH.exists():
        raise RuntimeError(
            "No Hugging Face token found. Create sidecar/.hf_token with your "
            "token — see README for instructions."
        )
    token = _HF_TOKEN_PATH.read_text(encoding="utf-8").strip()
    if not token:
        raise RuntimeError(
            "Hugging Face token file sidecar/.hf_token is empty. "
            "Paste your token into that file and retry."
        )
    return token


def get_diarization_pipeline() -> Pipeline:
    """Return the cached pyannote diarization pipeline, loading it on first call."""
    global _diarization_pipeline
    if _diarization_pipeline is None:
        logger.info("Loading pyannote speaker-diarization-community-1 pipeline...")
        _diarization_pipeline = Pipeline.from_pretrained(
            "pyannote/speaker-diarization-community-1",
            token=get_hf_token(),
        )
        logger.info("Diarization pipeline loaded.")
    return _diarization_pipeline


def warm_up() -> None:
    """Run dummy inference on both models so the first real request isn't slow."""
    import tempfile

    import soundfile as sf
    import torch

    model = get_whisper_model()
    # 1 second of silence at 16 kHz
    silence = np.zeros(16000, dtype=np.float32)
    tmp_path = Path(tempfile.mktemp(suffix=".wav"))
    try:
        sf.write(str(tmp_path), silence, 16000)
        segments, _ = model.transcribe(str(tmp_path), language=None)
        _ = list(segments)
        logger.info("Warm-up transcription complete.")
        waveform = torch.from_numpy(silence).unsqueeze(0)
        get_diarization_pipeline()({"waveform": waveform, "sample_rate": 16000})
        logger.info("Warm-up diarization complete.")
    except Exception as exc:
        logger.warning("Diarization warm-up failed (%s) — diarization will be attempted on first request.", exc)
    finally:
        tmp_path.unlink(missing_ok=True)
