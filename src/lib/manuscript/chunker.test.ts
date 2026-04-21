import { describe, expect, it } from "vitest";
import { createChunks, splitText } from "./chunker";

describe("chunker", () => {
  it("creates pause chunks", () => {
    const chunks = createChunks("chapter-001", "前です。[pause:medium]後です。");
    expect(chunks.map((chunk) => chunk.type)).toEqual(["text", "pause", "text"]);
    expect(chunks[1].pauseMs).toBe(1000);
  });

  it("splits long text at sentence boundaries", () => {
    const text = "これは長い文章です。".repeat(80);
    const chunks = splitText(text, 120);
    expect(chunks.length).toBeGreaterThan(1);
    expect(chunks.every((chunk) => chunk.length <= 121)).toBe(true);
  });
});
