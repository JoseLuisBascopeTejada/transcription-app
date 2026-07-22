---
name: tauri-ipc-contract
description: Use this skill whenever writing or editing frontend code that calls a Tauri command via invoke(), or listens for a backend-emitted event via listen(). Trigger this even for small UI changes that touch data coming from the Rust shell — this skill exists to prevent the most common source of silent breakage in a Tauri app, which is the frontend and backend independently drifting on a command name, argument shape, or event payload shape with no compiler to catch the mismatch.
---

# Tauri IPC Contract — The Frontend Never Guesses a Shape

## The critical rule

**Every `invoke()` call and every `listen()` event handler must use the
exact command name and exact argument/return/payload JSON shape as
currently defined on the Rust side — never assumed, never inferred from a
similar-sounding command, and never left as `any` in TypeScript.**

Rust and TypeScript are compiled/type-checked independently in this
architecture — there is no shared type system enforcing the contract
between them. A silent mismatch here doesn't fail to compile; it fails at
runtime with a vague error or, worse, with no error and just missing data
in the UI.

### Wrong (do not do this)

```typescript
// ❌ command name guessed, return shape assumed to be `any`, no error handling
const result = await invoke("startRecording");
setTranscript(result.text);
```

### Right

```typescript
// types/ipc.ts — mirrors the Rust command signatures exactly, kept in sync manually
export interface StartRecordingResponse {
  recording_id: string;
  started_at: string; // ISO 8601
}

export interface TranscriptSegmentEvent {
  segment_id: string;
  meeting_id: number;
  text: string;
  is_final: boolean; // false = partial live transcript, true = confirmed
}

// component code
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { StartRecordingResponse, TranscriptSegmentEvent } from "../types/ipc";

async function startRecording() {
  try {
    const result = await invoke<StartRecordingResponse>("start_recording");
    setRecordingId(result.recording_id);
  } catch (err) {
    setError(`Failed to start recording: ${err}`);
  }
}

useEffect(() => {
  const unlisten = listen<TranscriptSegmentEvent>("transcript-segment", (event) => {
    appendSegment(event.payload);
  });
  return () => { unlisten.then((fn) => fn()); };
}, []);
```

## Checklist before finishing any task that calls invoke() or listen()

- [ ] Does the command name string match the Rust `#[tauri::command]` function name exactly, including snake_case (Tauri does not auto-convert casing for you to rely on)?
- [ ] Is there a TypeScript interface for the argument and return/payload shape, sourced from the current Rust struct — not typed as `any` or left unannotated?
- [ ] Is every `invoke()` call wrapped in try/catch with a user-visible error path, not just a happy-path `await`?
- [ ] For event listeners, is `unlisten()` called on component unmount to avoid duplicate handlers accumulating across re-renders?

## When the contract needs to change

If a task requires adding or changing a command's arguments or an event's
payload shape, that is a two-sided change — flag it explicitly so the
Backend agent updates the Rust signature in the same work cycle, rather
than the frontend unilaterally adapting to a shape it's guessing the
backend will eventually match.

## Failure signs to watch for

- UI silently showing `undefined` instead of a value — almost always a
  payload field name mismatch, not a logic bug in the component.
- Live transcript never updating — check the event name string first
  (`"transcript-segment"` must match the Rust `emit()` call exactly).
- An `invoke()` that "hangs" — usually means the command name doesn't exist
  on the Rust side and Tauri is waiting on a promise that will never
  resolve; check the Rust command registration list.