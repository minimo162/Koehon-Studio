import { get, writable } from "svelte/store";
import type {
  Chapter,
  GenerationState,
  ManuscriptChunk,
} from "../project/projectTypes";
import { defaultGenerationState } from "../project/projectTypes";
import { getChunkOutputPath } from "../api/audioFiles";
import { ttsClient } from "../api/ttsClient";
import { projectStore } from "./projectStore";

export type GenerationLog = {
  at: string;
  level: "info" | "error";
  message: string;
};

export const generationStateStore = writable<GenerationState>(
  defaultGenerationState,
);
export const chunkStateStore = writable<Record<string, ManuscriptChunk>>({});
export const generationLogsStore = writable<GenerationLog[]>([]);

let stopRequested = false;

type QueueOptions = {
  regenerateDone?: boolean;
  onlyFailed?: boolean;
};

export function logGeneration(
  level: GenerationLog["level"],
  message: string,
): void {
  generationLogsStore.update((logs) =>
    [{ at: new Date().toLocaleTimeString(), level, message }, ...logs].slice(
      0,
      200,
    ),
  );
}

export function clearGenerationLogs(): void {
  generationLogsStore.set([]);
}

export function formatGenerationLogs(logs: GenerationLog[]): string {
  return logs
    .slice()
    .reverse()
    .map((log) => `[${log.at}] ${log.level.toUpperCase()}\t${log.message}`)
    .join("\n");
}

export function restoreChunkStates(
  chunks: Record<string, Partial<ManuscriptChunk>>,
  missingAudioPaths: string[] = [],
): void {
  const project = get(projectStore);
  const missingAudioPathSet = new Set(missingAudioPaths);
  const baseChunks = Object.fromEntries(
    project.chapters
      .flatMap((chapter) => chapter.chunks)
      .map((chunk) => [chunk.id, chunk]),
  );
  chunkStateStore.set(
    Object.fromEntries(
      Object.entries(baseChunks).map(([id, chunk]) => {
        const restored = chunks[id];
        const audioPath = restored?.audioPath;
        const missingAudio = Boolean(audioPath && missingAudioPathSet.has(audioPath));
        return [
          id,
          {
            ...chunk,
            status: missingAudio ? "pending" : (restored?.status ?? chunk.status),
            audioPath: missingAudio ? undefined : audioPath,
            error: missingAudio ? "音声ファイルが見つかりません。" : restored?.error,
            retryCount: restored?.retryCount ?? chunk.retryCount,
          },
        ];
      }),
    ),
  );
}

export function resetGenerationState(
  state: GenerationState = defaultGenerationState,
): void {
  generationStateStore.set(state);
}

export async function generateAll(): Promise<void> {
  const project = get(projectStore);
  await runQueue(
    project.chapters.filter((chapter) => chapter.includeInNarration),
  );
}

export async function generateChapter(chapterId: string): Promise<void> {
  const project = get(projectStore);
  const chapter = project.chapters.find((item) => item.id === chapterId);
  if (chapter) await runQueue([chapter]);
}

export async function regenerateFailedChunks(): Promise<void> {
  const project = get(projectStore);
  await runQueue(
    project.chapters.filter((chapter) => chapter.includeInNarration),
    { onlyFailed: true, regenerateDone: true },
  );
}

export async function regenerateChunk(chunkId: string): Promise<void> {
  const project = get(projectStore);
  const chapter = project.chapters.find((item) =>
    item.chunks.some((chunk) => chunk.id === chunkId),
  );
  if (!chapter) return;
  const chunk = chapter.chunks.find((item) => item.id === chunkId);
  if (!chunk) return;
  await runQueue([{ ...chapter, chunks: [chunk] }], { regenerateDone: true });
}

export function stopGeneration(): void {
  stopRequested = true;
  generationStateStore.update((state) => ({ ...state, status: "stopping" }));
  logGeneration(
    "info",
    "停止要求を受け付けました。現在のチャンク完了後に停止します。",
  );
}

export async function checkSidecar(): Promise<void> {
  const health = await ttsClient.health();
  logGeneration("info", `sidecar health ok: ${health.engine}`);
}

export function clearChunkAudio(chunkId: string): void {
  chunkStateStore.update((state) => {
    const chunk = state[chunkId];
    if (!chunk) return state;
    return {
      ...state,
      [chunkId]: {
        ...chunk,
        status: "pending",
        audioPath: undefined,
        error: undefined,
      },
    };
  });
}

async function runQueue(
  chapters: Chapter[],
  options: QueueOptions = {},
): Promise<void> {
  stopRequested = false;
  pruneMissingChunks();
  const allChunks = collectQueueChunks(chapters, options);
  if (allChunks.length === 0) {
    resetGenerationState({
      ...defaultGenerationState,
      status: "completed",
      finishedAt: new Date().toISOString(),
    });
    logGeneration(
      "info",
      options.onlyFailed
        ? "再生成対象の失敗チャンクはありません。"
        : "生成対象チャンクはありません。",
    );
    return;
  }
  prepareQueuedChunks(allChunks, {
    resetStatuses: options.regenerateDone || options.onlyFailed,
  });
  generationStateStore.set({
    status: "running",
    totalChunks: allChunks.length,
    completedChunks: 0,
    failedChunks: 0,
    startedAt: new Date().toISOString(),
  });

  for (const chunk of allChunks) {
    if (stopRequested) break;
    const latestChunk = get(chunkStateStore)[chunk.id] ?? chunk;
    if (
      !options.regenerateDone &&
      latestChunk.status === "done" &&
      latestChunk.audioPath
    ) {
      updateChunk(latestChunk);
      incrementCompleted();
      logGeneration("info", `${chunk.id} は生成済みのためスキップしました。`);
      continue;
    }
    if (chunk.type === "pause") {
      updateChunk({ ...chunk, status: "skipped" });
      incrementCompleted();
      continue;
    }

    updateChunk({ ...chunk, status: "generating", error: undefined });
    generationStateStore.update((state) => ({
      ...state,
      currentChapterId: chunk.chapterId,
      currentChunkId: chunk.id,
    }));
    try {
      const project = get(projectStore);
      const result = await ttsClient.synthesize({
        requestId: chunk.id,
        text: chunk.text ?? "",
        voice: project.settings.voice,
        outputPath: getChunkOutputPath(
          chunk,
          project.settings.outputDirectory || project.projectDir,
        ),
      });
      updateChunk({ ...chunk, status: "done", audioPath: result.audioPath });
      incrementCompleted();
      logGeneration("info", `${chunk.id} を生成しました。`);
    } catch (error) {
      updateChunk({
        ...chunk,
        status: "failed",
        error: error instanceof Error ? error.message : String(error),
        retryCount: chunk.retryCount + 1,
      });
      generationStateStore.update((state) => ({
        ...state,
        failedChunks: state.failedChunks + 1,
      }));
      logGeneration("error", `${chunk.id} の生成に失敗しました。`);
    }
  }

  generationStateStore.update((state) => ({
    ...state,
    status: stopRequested
      ? "idle"
      : state.failedChunks > 0
        ? "failed"
        : "completed",
    currentChapterId: undefined,
    currentChunkId: undefined,
    finishedAt: new Date().toISOString(),
  }));
}

export function collectQueueChunks(
  chapters: Chapter[],
  options: QueueOptions = {},
): ManuscriptChunk[] {
  const current = get(chunkStateStore);
  return chapters
    .flatMap((chapter) => chapter.chunks)
    .filter((chunk) => {
      if (!options.onlyFailed) return true;
      return current[chunk.id]?.status === "failed";
    });
}

export function prepareQueuedChunks(
  chunks: ManuscriptChunk[],
  options: { resetStatuses?: boolean } = {},
): void {
  chunkStateStore.update((state) => ({
    ...state,
    ...Object.fromEntries(
      chunks.map((chunk) => {
        const existing = state[chunk.id];
        if (
          existing &&
          !options.resetStatuses &&
          canReuseChunkState(existing, chunk)
        ) {
          return [
            chunk.id,
            {
              ...chunk,
              status: existing.status,
              audioPath: existing.audioPath,
              error: existing.error,
              retryCount: existing.retryCount,
            },
          ];
        }
        return [
          chunk.id,
          {
            ...chunk,
            status: "pending",
            audioPath: undefined,
            error: undefined,
          },
        ];
      }),
    ),
  }));
}

function updateChunk(chunk: ManuscriptChunk): void {
  chunkStateStore.update((state) => ({ ...state, [chunk.id]: chunk }));
}

function incrementCompleted(): void {
  generationStateStore.update((state) => ({
    ...state,
    completedChunks: state.completedChunks + 1,
  }));
}

function pruneMissingChunks(): void {
  const project = get(projectStore);
  const validIds = new Set(
    project.chapters
      .flatMap((chapter) => chapter.chunks)
      .map((chunk) => chunk.id),
  );
  chunkStateStore.update((state) =>
    Object.fromEntries(
      Object.entries(state).filter(([id]) => validIds.has(id)),
    ),
  );
}

function canReuseChunkState(
  existing: ManuscriptChunk,
  current: ManuscriptChunk,
): boolean {
  if (existing.type !== current.type) return false;
  if (existing.type === "text") return existing.text === current.text;
  return existing.pauseMs === current.pauseMs;
}
