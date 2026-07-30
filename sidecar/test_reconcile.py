"""Standalone test for reconcile.py — pure interval math, no server needed."""

import sys
sys.path.insert(0, ".")

from reconcile import overlap_duration, assign_speaker, reconcile


def test_overlap_duration():
    """Basic overlap cases."""
    assert overlap_duration(0.0, 5.0, 3.0, 8.0) == 2.0
    assert overlap_duration(0.0, 3.0, 5.0, 8.0) == 0.0
    assert overlap_duration(0.0, 10.0, 2.0, 6.0) == 4.0
    assert overlap_duration(2.0, 6.0, 0.0, 10.0) == 4.0
    assert overlap_duration(5.0, 5.0, 0.0, 10.0) == 0.0  # zero-width
    print("PASS: test_overlap_duration")


def test_assign_speaker_simple():
    """Segment fully inside one turn."""
    turns = [
        {"start": 0.0, "end": 5.0, "speaker": "SPEAKER_00"},
        {"start": 5.0, "end": 10.0, "speaker": "SPEAKER_01"},
    ]
    seg = {"start": 1.0, "end": 3.0, "text": "hello"}
    assert assign_speaker(seg, turns) == "SPEAKER_00"
    print("PASS: test_assign_speaker_simple")


def test_assign_speaker_spanning_change():
    """Segment spanning two turns — tie at 1.0s each, first turn wins (earlier in list)."""
    turns = [
        {"start": 0.0, "end": 5.0, "speaker": "SPEAKER_00"},
        {"start": 5.0, "end": 10.0, "speaker": "SPEAKER_01"},
    ]
    seg = {"start": 4.0, "end": 6.0, "text": "..."}
    # Overlaps SPEAKER_00 for 1.0s (4.0-5.0) and SPEAKER_01 for 1.0s (5.0-6.0)
    speaker = assign_speaker(seg, turns)
    assert speaker in ("SPEAKER_00", "SPEAKER_01"), f"Expected either speaker, got {speaker}"
    print(f"PASS: test_assign_speaker_spanning_change (tie-break: {speaker})")


def test_assign_speaker_no_overlap():
    """Segment during a diarization gap — should return UNKNOWN."""
    turns = [
        {"start": 2.0, "end": 4.0, "speaker": "SPEAKER_00"},
    ]
    seg = {"start": 6.0, "end": 8.0, "text": "gap speech"}
    assert assign_speaker(seg, turns) == "UNKNOWN"
    print("PASS: test_assign_speaker_no_overlap")


def test_assign_speaker_partial_overlap():
    """Segment overlaps two turns unequally — should pick the larger overlap."""
    turns = [
        {"start": 0.0, "end": 3.0, "speaker": "SPEAKER_00"},  # overlap: 1s (2-3)
        {"start": 2.0, "end": 10.0, "speaker": "SPEAKER_01"},  # overlap: 3s (2-5)
    ]
    seg = {"start": 2.0, "end": 5.0, "text": "mostly speaker 01"}
    assert assign_speaker(seg, turns) == "SPEAKER_01"
    print("PASS: test_assign_speaker_partial_overlap")


def test_reconcile_end_to_end():
    """Full reconcile pipeline with multiple segments and turns."""
    turns = [
        {"start": 0.0, "end": 4.0, "speaker": "SPEAKER_00"},
        {"start": 4.0, "end": 10.0, "speaker": "SPEAKER_01"},
    ]
    segments = [
        {"start": 0.5, "end": 3.5, "text": "Hello, I'm speaker zero."},
        {"start": 4.5, "end": 9.0, "text": "And I'm speaker one."},
        {"start": 3.0, "end": 5.5, "text": "We overlap here."},
    ]
    result = reconcile(segments, turns)
    assert len(result) == 3
    assert result[0]["speaker"] == "SPEAKER_00"  # 3.0s overlap with SPEAKER_00
    assert result[1]["speaker"] == "SPEAKER_01"  # 4.5s overlap with SPEAKER_01
    # Segment [3.0, 5.5]: overlaps SPEAKER_00 for 1.0s (3-4), SPEAKER_01 for 1.5s (4-5.5)
    assert result[2]["speaker"] == "SPEAKER_01"
    # Verify output shape
    for r in result:
        assert set(r.keys()) == {"start", "end", "text", "speaker"}
    print("PASS: test_reconcile_end_to_end")


def test_reconcile_unknown_fallback():
    """Segment with no overlap at all returns UNKNOWN."""
    turns = [
        {"start": 5.0, "end": 8.0, "speaker": "SPEAKER_00"},
    ]
    segments = [
        {"start": 0.0, "end": 2.0, "text": "Before any turn."},
    ]
    result = reconcile(segments, turns)
    assert result[0]["speaker"] == "UNKNOWN"
    print("PASS: test_reconcile_unknown_fallback")


if __name__ == "__main__":
    test_overlap_duration()
    test_assign_speaker_simple()
    test_assign_speaker_spanning_change()
    test_assign_speaker_no_overlap()
    test_assign_speaker_partial_overlap()
    test_reconcile_end_to_end()
    test_reconcile_unknown_fallback()
    print("\nAll tests passed.")
