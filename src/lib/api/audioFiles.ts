import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { exists, remove } from "@tauri-apps/plugin-fs";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import type { Chapter, ManuscriptChunk } from "../project/projectTypes";
import { isTauriRuntime } from "./fileAccess";

export type AudioMergeInput =
  | { type: "file"; path: string }
  | { type: "silence"; durationMs: number };

export type AudioMergeResult = {
  outputPath: string;
  sampleRate: number;
  channels: number;
  bitsPerSample: number;
  durationMs: number;
};

type RawAudioMergeResult = {
  output_path?: string;
  outputPath?: string;
  sample_rate?: number;
  sampleRate?: number;
  channels: number;
  bits_per_sample?: number;
  bitsPerSample?: number;
  duration_ms?: number;
  durationMs?: number;
};

export type AudioFileRecord = {
  id: string;
  label: string;
  kind: "chunk" | "chapter" | "export";
  path: string;
  chapterId?: string;
};

export function getChunkOutputPath(
  chunk: Pick<ManuscriptChunk, "id">,
  projectDir?: string,
): string {
  const fileName = `${sanitizeAudioFileName(chunk.id)}.wav`;
  if (!projectDir) return `generated_audio/${fileName}`;
  return `${trimTrailingSlash(projectDir)}/audio/chunks/${fileName}`;
}

export function getChapterOutputPath(
  chapter: Pick<Chapter, "order" | "title" | "id">,
  projectDir?: string,
): string {
  const fileName = `${String(chapter.order).padStart(2, "0")}-${sanitizeAudioFileName(chapter.title || chapter.id)}.wav`;
  if (!projectDir) return `generated_audio/chapters/${fileName}`;
  return `${trimTrailingSlash(projectDir)}/audio/chapters/${fileName}`;
}

export function getExportOutputPath(title: string, projectDir?: string): string {
  const fileName = `${sanitizeAudioFileName(title || "audiobook")}.wav`;
  if (!projectDir) return `generated_audio/exports/${fileName}`;
  return `${trimTrailingSlash(projectDir)}/audio/exports/${fileName}`;
}

export function toAudioSrc(path: string): string {
  return isTauriRuntime() ? convertFileSrc(path) : path;
}

export async function mergeWavFiles(
  inputs: AudioMergeInput[],
  outputPath: string,
): Promise<AudioMergeResult> {
  if (!isTauriRuntime()) {
    throw new Error("WAV結合は Tauri アプリ上で利用できます。");
  }
  const raw = await invoke<RawAudioMergeResult>("merge_wav_files", {
    inputs: inputs.map((input) =>
      input.type === "file"
        ? { type: "file", path: input.path }
        : { type: "silence", durationMs: Math.max(0, Math.round(input.durationMs)) },
    ),
    outputPath,
  });
  return {
    outputPath: raw.outputPath ?? raw.output_path ?? outputPath,
    sampleRate: raw.sampleRate ?? raw.sample_rate ?? 0,
    channels: raw.channels,
    bitsPerSample: raw.bitsPerSample ?? raw.bits_per_sample ?? 0,
    durationMs: raw.durationMs ?? raw.duration_ms ?? 0,
  };
}

export function buildChapterMergeInputs(
  chapter: Chapter,
  chunkStates: Record<string, ManuscriptChunk>,
): AudioMergeInput[] {
  return chapter.chunks.map((chunk) => {
    const current = chunkStates[chunk.id] ?? chunk;
    if (chunk.type === "pause") {
      return { type: "silence", durationMs: chunk.pauseMs ?? 0 };
    }
    if (current.status !== "done" || !current.audioPath) {
      throw new Error(`${chunk.id} の音声がまだ生成されていません。`);
    }
    return { type: "file", path: current.audioPath };
  });
}

export async function selectExportPath(defaultPath: string): Promise<string | undefined> {
  if (!isTauriRuntime()) return undefined;
  const selected = await save({
    title: "WAVを書き出し",
    filters: [{ name: "WAV Audio", extensions: ["wav"] }],
    defaultPath,
  });
  return selected ?? undefined;
}

export async function revealAudioFile(path: string): Promise<void> {
  if (!isTauriRuntime()) return;
  await revealItemInDir(path);
}

export async function openAudioFile(path: string): Promise<void> {
  if (!isTauriRuntime()) return;
  await openPath(path);
}

export async function deleteAudioFile(path: string): Promise<void> {
  if (!isTauriRuntime()) return;
  if (await exists(path)) await remove(path);
}

function sanitizeAudioFileName(value: string): string {
  return value
    .trim()
    .replace(/[<>:"/\\|?*\u0000-\u001F]/g, "_")
    .replace(/\s+/g, "_")
    .slice(0, 96);
}

function trimTrailingSlash(path: string): string {
  return path.replace(/[\\/]+$/, "");
}
