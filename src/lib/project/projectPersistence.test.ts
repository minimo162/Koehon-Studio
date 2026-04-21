import { describe, expect, it } from "vitest";
import type { GenerationState, Project } from "./projectTypes";
import {
  createProjectSnapshot,
  normalizeGenerationState,
} from "./projectPersistence";

const baseProject: Project = {
  id: "test",
  title: "テスト",
  createdAt: "2026-01-01T00:00:00.000Z",
  updatedAt: "2026-01-01T00:00:00.000Z",
  metadata: { title: "テスト" },
  chapters: [],
  settings: {
    ttsEngine: "moss-tts-nano-onnx",
    voice: "default",
    cpuThreads: 4,
    maxChunkChars: 450,
    pauseShortMs: 500,
    pauseMediumMs: 1000,
    pauseLongMs: 2000,
    outputSampleRate: 48000,
    exportFormat: "wav",
    includeManuscriptMemo: false,
  },
  generation: {
    status: "running",
    currentChapterId: "chapter-001",
    currentChunkId: "chapter-001-chunk-001",
    totalChunks: 2,
    completedChunks: 1,
    failedChunks: 0,
    startedAt: "2026-01-01T00:00:00.000Z",
  },
};

describe("projectPersistence", () => {
  it("normalizes active generation state before persistence", () => {
    const state: GenerationState = {
      status: "running",
      currentChapterId: "chapter-001",
      currentChunkId: "chapter-001-chunk-001",
      totalChunks: 2,
      completedChunks: 1,
      failedChunks: 0,
    };

    expect(normalizeGenerationState(state)).toEqual({
      status: "idle",
      totalChunks: 2,
      completedChunks: 1,
      failedChunks: 0,
      startedAt: undefined,
      finishedAt: undefined,
    });
  });

  it("normalizes generating chunks in snapshots", () => {
    const snapshot = createProjectSnapshot(
      baseProject,
      "# 本文",
      {},
      {
        "chapter-001-chunk-001": {
          id: "chapter-001-chunk-001",
          chapterId: "chapter-001",
          order: 1,
          type: "text",
          text: "本文",
          tags: [],
          status: "generating",
          retryCount: 0,
        },
      },
    );

    expect(snapshot.generation.status).toBe("idle");
    expect(snapshot.generation.currentChunkId).toBeUndefined();
    expect(snapshot.chunks["chapter-001-chunk-001"].status).toBe("pending");
  });
});
