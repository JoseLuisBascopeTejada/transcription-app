---
name: test-fixture-scoping
description: Use this skill whenever writing or editing tests that touch the Python sidecar (transcribe.py, diarize.py, reconcile.py) or the Rust database layer. Trigger this even for a single new test case — this skill exists to prevent two recurring QA mistakes in this project: pytest fixtures that reload an ML model per test instead of per session, and Rust database tests that leak state between tests because they share a single on-disk database file.
---

# Test Fixture Scoping — Session-Scoped Models, Isolated Databases

## The critical rule

**ML model fixtures (Whisper, pyannote) must be `scope="session"` in
pytest — never `scope="function"` (the default) — or the test suite
inherits the exact per-request reload cost that `sidecar-model-lifecycle`
exists to prevent, just in the test suite instead of production. Database
tests must each get a fresh, isolated SQLite instance — never share one
file across tests, or test order starts silently affecting results.**

### Wrong (do not do this — Python)

```python
def test_transcription_accuracy():
    model = WhisperModel("distil-large-v3")   # ❌ reloaded in every test function
    result = model.transcribe("fixtures/sample.wav")
    assert "hello" in result

def test_transcription_empty_audio():
    model = WhisperModel("distil-large-v3")   # ❌ reloaded again
    result = model.transcribe("fixtures/silence.wav")
    assert result == []
```

### Right (do not do this — Python)

```python
# conftest.py
import pytest
from faster_whisper import WhisperModel
from pyannote.audio import Pipeline

@pytest.fixture(scope="session")
def whisper_model():
    return WhisperModel("distil-large-v3", device="cpu", compute_type="int8")

@pytest.fixture(scope="session")
def diarization_pipeline():
    return Pipeline.from_pretrained(
        "pyannote/speaker-diarization-community-1",
        use_auth_token=TEST_HF_TOKEN,
    )

# test_transcribe.py
def test_transcription_accuracy(whisper_model):
    result = whisper_model.transcribe("fixtures/sample.wav")
    assert "hello" in result

def test_transcription_empty_audio(whisper_model):
    result = whisper_model.transcribe("fixtures/silence.wav")
    assert result == []
```

### Wrong (do not do this — Rust)

```rust
#[test]
fn test_insert_meeting() {
    let conn = Connection::open("meetings.db").unwrap(); // ❌ shared file —
    // ...                                                //    other tests' data
}                                                          //    leaks in

#[test]
fn test_delete_meeting_cascades() {
    let conn = Connection::open("meetings.db").unwrap(); // ❌ same file again —
    // ...                                                //    test order now matters
}
```

### Right (do not do this — Rust)

```rust
fn test_connection() -> Connection {
    let conn = Connection::open_in_memory().unwrap(); // fresh, isolated, per test
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    init_schema(&conn).unwrap();
    conn
}

#[test]
fn test_insert_meeting() {
    let conn = test_connection();
    // ...
}

#[test]
fn test_delete_meeting_cascades() {
    let conn = test_connection();
    // ...
}
```

## Checklist before finishing any testing task

- [ ] Are ML model fixtures declared `scope="session"`, confirmed by checking the fixture decorator explicitly rather than assuming the default is correct?
- [ ] Does every Rust database test use `Connection::open_in_memory()` (or an equivalent fresh temp file per test), never the app's real `meetings.db` path or a file shared across tests?
- [ ] Does the test use a named fixture audio file that actually exists in the repo's `fixtures/` directory — not an assumed filename?
- [ ] For reconciliation tests specifically, are fixtures hand-constructed timestamp objects (per the `diarization-reconciliation` skill), not real audio — this logic should be testable without any model at all?

## Failure signs to watch for

- Test suite runtime growing linearly with test count in a way that tracks
  suspiciously closely with the number of tests touching the sidecar —
  check fixture scope first.
- A database test that passes alone but fails when run as part of the full
  suite (or vice versa) — near-certain sign of a shared on-disk DB file
  instead of per-test isolation.