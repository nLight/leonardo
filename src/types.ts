export type RecordingStatus = "new" | "queued" | "processing" | "ready" | "error";

export interface TranscriptSegment {
  startMs: number;
  endMs: number;
  text: string;
}

export interface Chapter {
  startMs: number;
  title: string;
  description: string;
}

export interface AudioTrackInfo {
  number: number;
  streamIndex: number;
  title: string;
  codec: string;
  channels: number;
}

export interface Recording {
  id: string;
  path: string;
  fileName: string;
  displayTitle: string;
  extension: string;
  sizeBytes: number;
  modifiedAt: number;
  durationMs: number | null;
  status: RecordingStatus;
  progress: number;
  summary: string;
  language: string | null;
  audioTracks: AudioTrackInfo[];
  audioSource: string | null;
  transcript: TranscriptSegment[];
  chapters: Chapter[];
  error: string | null;
}

export interface AppSettings {
  mediaFolders: string[];
  ffmpegPath: string;
  ffprobePath: string;
  whisperPath: string;
  modelPath: string;
  language: string;
  audioMode: "auto" | "track" | "mix";
  microphoneTrack: number;
}

export interface LibrarySnapshot {
  recordings: Recording[];
  settings: AppSettings;
}

export interface TimeRange {
  startMs: number;
  endMs: number;
}

export interface Candidate {
  id: string;
  startMs: number;
  endMs: number;
  score: number;
  peakLufs: number;
  sceneCuts: number;
  reason: string;
}

export interface Highlights {
  version: number;
  durationMs: number;
  audioSource: string;
  clips: Candidate[];
  deadAir: TimeRange[];
  unusable: TimeRange[];
}
