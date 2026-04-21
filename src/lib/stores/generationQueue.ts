import { get, writable } from "svelte/store";
import type { Chapter, GenerationState, ManuscriptChunk } from "../project/projectTypes";
import { defaultGenerationState } from "../project/projectTypes";
import { ttsClient } from "../api/ttsClient";
import { projectStore } from "./projectStore";

export type GenerationLog = {
  at: string;
  level: "info" | "error";
  message: string;
};

export const generationStateStore = writable<GenerationState>(defaultGenerationState);
export const chunkStateStore = writable<Record<string, ManuscriptChunk>>({});
export const generationLogsStore = writable<GenerationLog[]>([]);

let stopRequested = false;

export function logGeneration(level: GenerationLog["level"], message: string): void {
  generationLogsStore.update((logs) => [{ at: new Date().toLocaleTimeString(), level, message }, ...logs].slice(0, 200));
}

export async function generateAll(): Promise<void> {
  const project = get(projectStore);
  await runQueue(project.chapters.filter((chapter) => chapter.includeInNarration));
}

export async function generateChapter(chapterId: string): Promise<void> {
  const project = get(projectStore);
  const chapter = project.chapters.find((item) => item.id === chapterId);
  if (chapter) await runQueue([chapter]);
}

export function stopGeneration(): void {
  stopRequested = true;
  generationStateStore.update((state) => ({ ...state, status: "stopping" }));
  logGeneration("info", "停止要求を受け付けました。現在のチャンク完了後に停止します。");
}

export async function checkSidecar(): Promise<void> {
  const health = await ttsClient.health();
  logGeneration("info", `sidecar health ok: ${health.engine}`);
}

async function runQueue(chapters: Chapter[]): Promise<void> {
  stopRequested = false;
  const allChunks = collectQueueChunks(chapters);
  chunkStateStore.set(Object.fromEntries(allChunks.map((chunk) => [chunk.id, chunk])));
  generationStateStore.set({
    status: "running",
    totalChunks: allChunks.length,
    completedChunks: 0,
    failedChunks: 0,
    startedAt: new Date().toISOString()
  });

  for (const chunk of allChunks) {
    if (stopRequested) break;
    if (chunk.type === "pause") {
      updateChunk({ ...chunk, status: "skipped" });
      incrementCompleted();
      continue;
    }

    updateChunk({ ...chunk, status: "generating" });
    generationStateStore.update((state) => ({ ...state, currentChapterId: chunk.chapterId, currentChunkId: chunk.id }));
    try {
      const result = await ttsClient.synthesize({
        requestId: chunk.id,
        text: chunk.text ?? "",
        voice: get(projectStore).settings.voice,
        outputPath: `generated_audio/${chunk.id}.wav`
      });
      updateChunk({ ...chunk, status: "done", audioPath: result.audioPath });
      incrementCompleted();
      logGeneration("info", `${chunk.id} を生成しました。`);
    } catch (error) {
      updateChunk({ ...chunk, status: "failed", error: error instanceof Error ? error.message : String(error), retryCount: chunk.retryCount + 1 });
      generationStateStore.update((state) => ({ ...state, failedChunks: state.failedChunks + 1 }));
      logGeneration("error", `${chunk.id} の生成に失敗しました。`);
    }
  }

  generationStateStore.update((state) => ({
    ...state,
    status: stopRequested ? "idle" : state.failedChunks > 0 ? "failed" : "completed",
    currentChapterId: undefined,
    currentChunkId: undefined,
    finishedAt: new Date().toISOString()
  }));
}

export function collectQueueChunks(chapters: Chapter[]): ManuscriptChunk[] {
  return chapters.flatMap((chapter) => chapter.chunks);
}

function updateChunk(chunk: ManuscriptChunk): void {
  chunkStateStore.update((state) => ({ ...state, [chunk.id]: chunk }));
}

function incrementCompleted(): void {
  generationStateStore.update((state) => ({ ...state, completedChunks: state.completedChunks + 1 }));
}
