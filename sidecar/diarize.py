"""Diarization module — pyannote.audio speaker diarization."""

from __future__ import annotations

import logging
import subprocess
import tempfile
from pathlib import Path

import numpy as np
import soundfile as sf
import torch

from models import get_diarization_pipeline

logger = logging.getLogger(__name__)


def _load_audio_as_waveform(audio_path: str) -> dict:
    """Load any audio/video file as a pyannote-compatible waveform dict.

    Uses ffmpeg to decode to mono 16 kHz WAV, then loads via soundfile
    and wraps in a torch tensor.  Returns {"waveform": (1, samples) Tensor,
    "sample_rate": 16000}.
    """
    tmp_wav = Path(tempfile.mktemp(suffix=".wav"))
    try:
        subprocess.run(
            [
                "ffmpeg", "-y", "-i", str(audio_path),
                "-ar", "16000", "-ac", "1",
                "-f", "wav", str(tmp_wav),
            ],
            capture_output=True,
            check=True,
        )
        data, sr = sf.read(str(tmp_wav), dtype="float32")
        waveform = torch.from_numpy(data).unsqueeze(0)  # (1, samples)
        return {"waveform": waveform, "sample_rate": sr}
    finally:
        tmp_wav.unlink(missing_ok=True)


def diarize(audio_path: str) -> list[dict]:
    """Run speaker diarization on an audio file.

    Returns a list of turns, each with "start" (float seconds),
    "end" (float seconds), and "speaker" (e.g. "SPEAKER_00").
    """
    pipeline = get_diarization_pipeline()
    audio = _load_audio_as_waveform(audio_path)
    output = pipeline(audio)

    turns: list[dict] = []
    for turn, speaker in output.exclusive_speaker_diarization:
        turns.append({
            "start": round(turn.start, 3),
            "end": round(turn.end, 3),
            "speaker": speaker,
        })

    logger.info("Diarization produced %d turn(s) for %s", len(turns), audio_path)
    return turns
