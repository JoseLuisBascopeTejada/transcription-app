"""Reconciliation module — merge Whisper transcript segments with pyannote diarization turns.

The two models produce independently-computed boundaries that never line up
exactly. This module assigns each Whisper segment to the diarization speaker
with the *greatest time overlap* — never assumes a segment falls entirely
inside a single turn.
"""

from __future__ import annotations


def overlap_duration(a_start: float, a_end: float, b_start: float, b_end: float) -> float:
    """Return the duration (seconds) of overlap between two intervals."""
    return max(0.0, min(a_end, b_end) - max(a_start, b_start))


def assign_speaker(
    whisper_segment: dict, diarization_turns: list[dict]
) -> str:
    """Assign the diarization speaker with the greatest time overlap.

    Returns "UNKNOWN" if the segment has zero overlap with every
    diarization turn (e.g. a segment during a diarization gap).
    """
    best_speaker = "UNKNOWN"
    best_overlap = 0.0
    for turn in diarization_turns:
        overlap = overlap_duration(
            whisper_segment["start"], whisper_segment["end"],
            turn["start"], turn["end"],
        )
        if overlap > best_overlap:
            best_overlap = overlap
            best_speaker = turn["speaker"]
    return best_speaker


def reconcile(
    whisper_segments: list[dict], diarization_turns: list[dict]
) -> list[dict]:
    """Merge Whisper segments with diarization turns into a speaker-labeled transcript.

    Each Whisper segment is assigned the speaker with the greatest time
    overlap (Strategy 1 — simple whole-segment assignment, v1 default).

    Args:
        whisper_segments: [{"start": float, "end": float, "text": str}, ...]
        diarization_turns: [{"start": float, "end": float, "speaker": str}, ...]

    Returns:
        [{"start": float, "end": float, "text": str, "speaker": str}, ...]
    """
    return [
        {
            "start": seg["start"],
            "end": seg["end"],
            "text": seg["text"],
            "speaker": assign_speaker(seg, diarization_turns),
        }
        for seg in whisper_segments
    ]
