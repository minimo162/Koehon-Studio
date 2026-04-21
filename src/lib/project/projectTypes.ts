export type ManuscriptMetadata = {
  title?: string;
  source_type?: string;
  source?: string;
  audience?: string;
  language?: string;
  style?: string;
  version?: number | string;
  [key: string]: string | number | undefined;
};

export type ManuscriptTag = {
  raw: string;
  name: "pause" | "voice" | "speed" | "chapter" | "unknown";
  value?: string;
};

export type ChunkStatus = "pending" | "generating" | "done" | "failed" | "skipped";

export type ManuscriptChunk = {
  id: string;
  chapterId: string;
  order: number;
  type: "text" | "pause";
  text?: string;
  pauseMs?: number;
  tags: ManuscriptTag[];
  status: ChunkStatus;
  audioPath?: string;
  error?: string;
  retryCount: number;
};

export type Chapter = {
  id: string;
  title: string;
  level: number;
  order: number;
  rawMarkdown: string;
  plainText: string;
  includeInNarration: boolean;
  chunks: ManuscriptChunk[];
};

export type ProjectSettings = {
  ttsEngine: "moss-tts-nano-onnx";
  voice?: string;
  modelDirectory?: string;
  outputDirectory?: string;
  cpuThreads: number;
  maxChunkChars: number;
  pauseShortMs: number;
  pauseMediumMs: number;
  pauseLongMs: number;
  outputSampleRate: number;
  exportFormat: "wav" | "mp3" | "m4b";
  includeManuscriptMemo: boolean;
};

export type GenerationState = {
  status: "idle" | "running" | "paused" | "stopping" | "completed" | "failed";
  currentChapterId?: string;
  currentChunkId?: string;
  totalChunks: number;
  completedChunks: number;
  failedChunks: number;
  startedAt?: string;
  finishedAt?: string;
};

export type Project = {
  id: string;
  title: string;
  createdAt: string;
  updatedAt: string;
  projectDir?: string;
  manuscriptPath?: string;
  metadata: ManuscriptMetadata;
  chapters: Chapter[];
  settings: ProjectSettings;
  generation: GenerationState;
};

export const defaultProjectSettings: ProjectSettings = {
  ttsEngine: "moss-tts-nano-onnx",
  voice: "default",
  modelDirectory: "",
  outputDirectory: "",
  cpuThreads: 4,
  maxChunkChars: 450,
  pauseShortMs: 500,
  pauseMediumMs: 1000,
  pauseLongMs: 2000,
  outputSampleRate: 48000,
  exportFormat: "wav",
  includeManuscriptMemo: false
};

export const defaultGenerationState: GenerationState = {
  status: "idle",
  totalChunks: 0,
  completedChunks: 0,
  failedChunks: 0
};
