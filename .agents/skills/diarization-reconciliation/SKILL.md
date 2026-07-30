---
name: diarization-reconciliation
description: Use this skill whenever writing or editing code in sidecar/reconcile.py, or any function that merges faster-whisper transcript segments with pyannote.audio diarization turns into a final speaker-labeled transcript. Trigger this even for small changes to the merge/alignment logic — this skill exists because timestamp reconciliation between two independent models is the highest-risk logic in the whole pipeline, and subtle boundary bugs here silently produce a wrong transcript rather than a visible crash.
---

# Diarization Reconciliation — Aligning Two Independent Timestamp Sets

## The critical rule

**Whisper segments and pyannote diarization turns come from two separate
models with independently-computed boundaries that will never line up
exactly. Reconciliation must assign each Whisper segment to the diarization
speaker with the greatest time overlap — never assume a Whisper segment
falls entirely inside a single diarization turn.**

Treating this as a simple "does segment start time fall within a turn"
lookup is the most common bug here, and it silently misattributes speech at
every speaker change, which is exactly where accuracy matters most to the
user.

### Wrong (do not do this)

```python
def assign_speaker(whisper_segment, diarization_turns):
    for turn in diarization_turns:
        if turn.start <= whisper_segment.start:   # ❌ only checks segment start,
            speaker = turn.speaker                #    ignores overlap duration entirely
    return speaker
```

### Right

```python
def overlap_duration(a_start, a_end, b_start, b_end) -> float:
    return max(0.0, min(a_end, b_end) - max(a_start, b_start))

def assign_speaker(whisper_segment, diarization_turns):
    """Assign the diarization speaker with the greatest time overlap
    with this Whisper segment. Returns 'UNKNOWN' if there is no overlap
    at all (e.g. a Whisper segment during a diarization gap)."""
    best_speaker = "UNKNOWN"
    best_overlap = 0.0
    for turn in diarization_turns:
        overlap = overlap_duration(
            whisper_segment.start, whisper_segment.end,
            turn.start, turn.end,
        )
        if overlap > best_overlap:
            best_overlap = overlap
            best_speaker = turn.speaker
    return best_speaker
```

## Handling a Whisper segment that spans a speaker change

A single Whisper segment can legitimately span two diarization turns (e.g.
one speaker finishes mid-segment and another starts). Two acceptable
strategies — pick one and apply it consistently, don't mix them silently:

1. **Simple (default for v1):** assign the whole segment to the
   speaker with the greatest overlap, per the function above. Accept the
   minor inaccuracy at boundaries.
2. **Split (stretch goal):** split the Whisper segment's text at the word
   level (if word-level timestamps are enabled in faster-whisper) and
   assign each word-group to its own overlapping speaker.

Do not implement strategy 2 without explicit instruction — it requires
word-level timestamps to be enabled on the Whisper call, which has its own
performance cost, and should be a deliberate decision by the Orchestrator,
not something introduced silently inside reconciliation.

## Checklist before finishing any task touching reconcile.py

- [ ] Does speaker assignment use overlap *duration*, not just start-time containment?
- [ ] Is there an explicit fallback (`UNKNOWN` or similar) for a Whisper segment with zero overlap against any diarization turn, rather than an unhandled exception or silently assigning the first turn?
- [ ] Are timestamps compared in the same unit throughout (seconds as floats, or milliseconds as ints) — check both Whisper's and pyannote's native output units before merging, since a unit mismatch will silently produce nonsense assignments rather than an error?
- [ ] Is the final output shape exactly what the Backend agent's SQLite write path expects (see the `segments` table schema in PLAN.md §3.3): `speaker_label`, `start_ms`, `end_ms`, `text`?

## Testing this layer

Use fixed, hand-constructed timestamp fixtures rather than real audio —
this logic should be tested as pure interval math, independent of model
accuracy:

```python
def test_segment_spanning_speaker_change():
    turns = [
        DiarizationTurn(start=0.0, end=5.0, speaker="SPEAKER_00"),
        DiarizationTurn(start=5.0, end=10.0, speaker="SPEAKER_01"),
    ]
    segment = WhisperSegment(start=4.0, end=6.0, text="...")
    # Segment overlaps SPEAKER_00 for 1.0s and SPEAKER_01 for 1.0s — a tie.
    # Document and test the tie-break rule explicitly (e.g. "first turn wins")
    # rather than leaving tie behavior undefined.
    assert assign_speaker(segment, turns) in ("SPEAKER_00", "SPEAKER_01")
```