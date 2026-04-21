import { beforeEach, describe, expect, it } from "vitest";
import { get } from "svelte/store";
import type { Chapter } from "../project/projectTypes";
import {
  chunkStateStore,
  collectQueueChunks,
  prepareQueuedChunks,
  restoreChunkStates,
} from "./generationQueue";
import { setManuscript } from "./manuscriptStore";

describe("collectQueueChunks", () => {
  beforeEach(() => {
    chunkStateStore.set({});
  });

  it("preserves manuscript chunk order including pauses", () => {
    const chapters = [
      {
        id: "chapter-001",
        title: "本文",
        level: 1,
        order: 1,
        rawMarkdown: "",
        plainText: "",
        includeInNarration: true,
        chunks: [
          {
            id: "text-1",
            chapterId: "chapter-001",
            order: 1,
            type: "text",
            text: "前",
            tags: [],
            status: "pending",
            retryCount: 0,
          },
          {
            id: "pause-1",
            chapterId: "chapter-001",
            order: 2,
            type: "pause",
            pauseMs: 1000,
            tags: [],
            status: "pending",
            retryCount: 0,
          },
          {
            id: "text-2",
            chapterId: "chapter-001",
            order: 3,
            type: "text",
            text: "後",
            tags: [],
            status: "pending",
            retryCount: 0,
          },
        ],
      },
    ] satisfies Chapter[];

    expect(collectQueueChunks(chapters).map((chunk) => chunk.id)).toEqual([
      "text-1",
      "pause-1",
      "text-2",
    ]);
  });

  it("keeps existing chunk states when preparing a partial queue", () => {
    chunkStateStore.set({
      chapter1: {
        id: "chapter1",
        chapterId: "chapter-001",
        order: 1,
        type: "text",
        text: "保存済み",
        tags: [],
        status: "done",
        audioPath: "/tmp/chapter1.wav",
        retryCount: 0,
      },
    });

    prepareQueuedChunks([
      {
        id: "chapter2",
        chapterId: "chapter-002",
        order: 1,
        type: "text",
        text: "生成対象",
        tags: [],
        status: "pending",
        retryCount: 0,
      },
    ]);

    expect(get(chunkStateStore).chapter1.audioPath).toBe("/tmp/chapter1.wav");
    expect(get(chunkStateStore).chapter2.status).toBe("pending");
  });

  it("resets persisted audio when chunk text changes", () => {
    chunkStateStore.set({
      chapter1: {
        id: "chapter1",
        chapterId: "chapter-001",
        order: 1,
        type: "text",
        text: "古い本文",
        tags: [],
        status: "done",
        audioPath: "/tmp/chapter1.wav",
        retryCount: 0,
      },
    });

    prepareQueuedChunks([
      {
        id: "chapter1",
        chapterId: "chapter-001",
        order: 1,
        type: "text",
        text: "新しい本文",
        tags: [],
        status: "pending",
        retryCount: 0,
      },
    ]);

    expect(get(chunkStateStore).chapter1.status).toBe("pending");
    expect(get(chunkStateStore).chapter1.audioPath).toBeUndefined();
  });

  it("resets restored chunks whose audio files are missing", () => {
    setManuscript("# 章\n\n本文");

    restoreChunkStates(
      {
        "chapter-001-chunk-001": {
          status: "done",
          audioPath: "/tmp/missing.wav",
          retryCount: 0,
        },
      },
      ["/tmp/missing.wav"],
    );

    const chunk = get(chunkStateStore)["chapter-001-chunk-001"];
    expect(chunk.status).toBe("pending");
    expect(chunk.audioPath).toBeUndefined();
    expect(chunk.error).toBe("音声ファイルが見つかりません。");
  });
});
