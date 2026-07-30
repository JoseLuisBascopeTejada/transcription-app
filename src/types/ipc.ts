export interface StartRecordingResponse {
  recording_id: string;
  started_at: string;
  mic_enabled: boolean;
}

export interface StopRecordingResponse {
  system_audio_path: string;
  mic_path: string | null;
  duration_seconds: number;
}

export interface NormalizeResponse {
  normalized_path: string;
  duration_seconds: number;
}

export interface TranscriptSegment {
  start: number;
  end: number;
  text: string;
}

export interface ReconciledSegment {
  start: number;
  end: number;
  text: string;
  speaker: string;
}

export interface MergedSegment {
  label: string;
  start: number;
  end: number;
  text: string;
}
