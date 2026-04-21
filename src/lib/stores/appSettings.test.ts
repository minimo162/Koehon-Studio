import { describe, expect, it } from "vitest";
import { defaultProjectSettings } from "../project/projectTypes";
import {
  normalizeProjectSettings,
  validateProjectSettings,
} from "./appSettings";

describe("appSettings", () => {
  it("normalizes persisted partial settings", () => {
    expect(
      normalizeProjectSettings({
        cpuThreads: 64,
        maxChunkChars: 10,
        pauseShortMs: -1,
        voice: "  narrator  ",
      }),
    ).toMatchObject({
      cpuThreads: 32,
      maxChunkChars: 100,
      pauseShortMs: 0,
      voice: "narrator",
    });
  });

  it("reports invalid editable settings", () => {
    const errors = validateProjectSettings({
      ...defaultProjectSettings,
      cpuThreads: 0,
      maxChunkChars: 2000,
      exportFormat: "mp3",
    });

    expect(errors).toContain("CPUスレッド数は 1 から 32 の範囲で指定してください。");
    expect(errors).toContain("最大チャンク文字数は 100 から 1200 の範囲で指定してください。");
    expect(errors).toContain("初期版の書き出し形式は wav のみ利用できます。");
  });
});
