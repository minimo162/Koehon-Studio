import { describe, expect, it } from "vitest";
import { parseTags, pauseMsFor, splitByPauseTags } from "./tags";

describe("tags", () => {
  it("parses known tags", () => {
    expect(parseTags("[voice:narrator]本文[speed:slow][chapter:end]")).toEqual([
      { raw: "[voice:narrator]", name: "voice", value: "narrator" },
      { raw: "[speed:slow]", name: "speed", value: "slow" },
      { raw: "[chapter:end]", name: "chapter", value: "end" }
    ]);
  });

  it("keeps unknown tags as syntax without treating them as speech text", () => {
    expect(parseTags("[emotion:calm]本文")).toEqual([{ raw: "[emotion:calm]", name: "unknown", value: "calm" }]);
  });

  it("splits pause tags and maps durations", () => {
    const parts = splitByPauseTags("前[pause:medium]後");
    expect(parts).toHaveLength(3);
    expect(pauseMsFor("short")).toBe(500);
    expect(pauseMsFor("medium")).toBe(1000);
    expect(pauseMsFor("long")).toBe(2000);
  });
});
