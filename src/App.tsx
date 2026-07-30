import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
  StartRecordingResponse,
  StopRecordingResponse,
  NormalizeResponse,
  TranscriptSegment,
  ReconciledSegment,
  MergedSegment,
} from "./types/ipc";
import "./App.css";

function App() {
  // Live Mode state
  const [includeMic, setIncludeMic] = useState(false);
  const [isRecording, setIsRecording] = useState(false);
  const [recordingInfo, setRecordingInfo] =
    useState<StartRecordingResponse | null>(null);

  // File Mode state
  const [selectedFile, setSelectedFile] = useState<string | null>(null);

  // Shared state
  const [segments, setSegments] = useState<MergedSegment[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function remapSpeakerLabels(segments: ReconciledSegment[]): Map<string, string> {
    const mapping = new Map<string, string>();
    let speakerNum = 1;
    for (const seg of segments) {
      if (mapping.has(seg.speaker)) continue;
      if (seg.speaker === "UNKNOWN") {
        mapping.set(seg.speaker, "Speaker desconocido");
      } else {
        mapping.set(seg.speaker, `Speaker ${speakerNum}`);
        speakerNum++;
      }
    }
    return mapping;
  }

  async function handleStartRecording() {
    setError(null);
    setSegments([]);
    try {
      const response = await invoke<StartRecordingResponse>(
        "start_recording",
        { includeMic }
      );
      setRecordingInfo(response);
      setIsRecording(true);
    } catch (err) {
      console.error("Start Recording failed:", err);
      setError(`Failed to start recording: ${err}`);
    }
  }

  async function handleStopRecording() {
    setError(null);
    setLoading(true);
    try {
      const stopResponse = await invoke<StopRecordingResponse>(
        "stop_recording"
      );
      setIsRecording(false);
      setRecordingInfo(null);

      // Transcribe system audio with diarization
      const systemReconciled = await invoke<ReconciledSegment[]>(
        "transcribe_with_speakers",
        { audioPath: stopResponse.system_audio_path }
      );

      const speakerMap = remapSpeakerLabels(systemReconciled);

      // Transcribe mic if present (no diarization — speaker is always "You")
      let micSegments: TranscriptSegment[] = [];
      if (stopResponse.mic_path) {
        micSegments = await invoke<TranscriptSegment[]>(
          "transcribe_audio",
          {
            audioPath: stopResponse.mic_path,
            mode: "full-file",
          }
        );
      }

      // Merge both streams into a single chronologically sorted list
      const merged: MergedSegment[] = [
        ...systemReconciled.map((s) => ({
          start: s.start,
          end: s.end,
          text: s.text,
          label: speakerMap.get(s.speaker) ?? s.speaker,
        })),
        ...micSegments.map((s) => ({
          ...s,
          label: "Micrófono",
        })),
      ].sort((a, b) => a.start - b.start);

      setSegments(merged);
    } catch (err) {
      console.error("Stop Recording failed:", err);
      setError(`Failed to stop recording: ${err}`);
      setIsRecording(false);
    } finally {
      setLoading(false);
    }
  }

  async function handleFileSelect() {
    try {
      const filePath = await open({
        multiple: false,
        filters: [
          { name: "Audio/Video", extensions: ["mp3", "wav", "mp4", "m4a"] },
        ],
      });
      if (filePath) {
        setSelectedFile(filePath as string);
      }
    } catch (err) {
      console.error("File dialog failed:", err);
      setError(`Failed to open file dialog: ${err}`);
    }
  }

  async function handleUploadAndTranscribe() {
    if (!selectedFile) return;
    setError(null);
    setSegments([]);
    setLoading(true);
    try {
      const normalizeResponse = await invoke<NormalizeResponse>(
        "upload_and_normalize",
        { filePath: selectedFile }
      );

      const reconciledSegments = await invoke<ReconciledSegment[]>(
        "transcribe_with_speakers",
        { audioPath: normalizeResponse.normalized_path }
      );

      const speakerMap = remapSpeakerLabels(reconciledSegments);
      setSegments(
        reconciledSegments.map((s) => ({
          start: s.start,
          end: s.end,
          text: s.text,
          label: speakerMap.get(s.speaker) ?? s.speaker,
        }))
      );
    } catch (err) {
      console.error("Upload/Transcribe failed:", err);
      setError(`Failed to process file: ${err}`);
    } finally {
      setLoading(false);
    }
  }

  return (
    <main className="container">
      <h1>Transcription App</h1>

      {error && <div className="error">{error}</div>}

      {/* SECTION A — Live Mode */}
      <section className="mode-section">
        <h2>Live Mode</h2>
        <div className="controls">
          <label className="checkbox-label">
            <input
              type="checkbox"
              checked={includeMic}
              onChange={(e) => setIncludeMic(e.target.checked)}
              disabled={isRecording || loading}
            />
            Also capture my microphone
          </label>

          <button
            onClick={handleStartRecording}
            disabled={isRecording || loading}
          >
            Start Recording
          </button>
          <button
            onClick={handleStopRecording}
            disabled={!isRecording || loading}
          >
            Stop Recording
          </button>

          {isRecording && recordingInfo && (
            <div className="recording-indicator">
              Recording in progress... (ID: {recordingInfo.recording_id})
            </div>
          )}

          {loading && !isRecording && (
            <div className="transcribing-indicator">
              Transcribing audio... this may take a minute for long recordings.
            </div>
          )}
        </div>
      </section>

      {/* SECTION B — File Mode */}
      <section className="mode-section">
        <h2>File Mode</h2>
        <div className="controls">
          <button onClick={handleFileSelect}>Select File</button>
          {selectedFile && <span className="file-name">{selectedFile}</span>}
          <button
            onClick={handleUploadAndTranscribe}
            disabled={!selectedFile || loading}
          >
            Upload &amp; Transcribe
          </button>
        </div>
      </section>

      {/* Transcript output */}
      {segments.length > 0 && (
        <section className="transcript-section">
          <h2>Transcript</h2>
          <ul className="transcript-list">
            {segments.map((seg, i) => (
              <li key={i} className="transcript-line">
                <span className="transcript-source">
                  {seg.label} =&gt;{" "}
                </span>
                <span className="transcript-text">{seg.text}</span>
              </li>
            ))}
          </ul>
        </section>
      )}
    </main>
  );
}

export default App;
