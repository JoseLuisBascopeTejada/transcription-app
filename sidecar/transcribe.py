"""Transcription module — faster-whisper wrappers for full-file and live-chunk modes."""

from __future__ import annotations

import threading

from models import get_whisper_model

# Serializes every call to model.transcribe() so two overlapping requests
# never corrupt the shared WhisperModel's internal state.
_transcribe_lock = threading.Lock()


def _run_transcription(audio_path: str, language: str | None = None) -> list[dict]:
    """Core transcription logic shared by both modes.

    Args:
        audio_path: Path to the audio file.
        language: ISO 639-1 language code (e.g. "es", "en") or None for
            auto-detect.
    """
    model = get_whisper_model()
    with _transcribe_lock:
        segments_iter, _ = model.transcribe(audio_path, language=language, task="transcribe")
        results: list[dict] = []
        for seg in segments_iter:
            results.append({
                "start": round(seg.start, 3),
                "end": round(seg.end, 3),
                "text": seg.text.strip(),
            })
    return results


def transcribe_full(audio_path: str, language: str | None = None) -> list[dict]:
    """Batch mode — single pass over the whole file."""
    return _run_transcription(audio_path, language=language)


def transcribe_chunk(audio_path: str, language: str | None = None) -> list[dict]:
    """Live mode — single chunk (~5-10s). Same call, caller controls chunk size."""
    return _run_transcription(audio_path, language=language)
