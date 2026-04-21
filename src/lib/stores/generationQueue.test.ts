import { describe, expect, it } from "vitest";
import type { Chapter } from "../project/projectTypes";
import { collectQueueChunks } from "./generationQueue";

describe("collectQueueChunks", () => {
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
          { id: "text-1", chapterId: "chapter-001", order: 1, type: "text", text: "前", tags: [], status: "pending", retryCount: 0 },
          { id: "pause-1", chapterId: "chapter-001", order: 2, type: "pause", pauseMs: 1000, tags: [], status: "pending", retryCount: 0 },
          { id: "text-2", chapterId: "chapter-001", order: 3, type: "text", text: "後", tags: [], status: "pending", retryCount: 0 }
        ]
      }
    ] satisfies Chapter[];

    expect(collectQueueChunks(chapters).map((chunk) => chunk.id)).toEqual(["text-1", "pause-1", "text-2"]);
  });
});
