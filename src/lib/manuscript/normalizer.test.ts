import { describe, expect, it } from "vitest";
import { normalizeForSpeech } from "./normalizer";

describe("normalizeForSpeech", () => {
  it("removes common markdown decoration and control tags", () => {
    expect(normalizeForSpeech("# 見出し\n\n- **本文**です。[pause:short]\nhttps://example.com")).toBe("見出し\n本文です。");
  });

  it("keeps link labels and inline code content", () => {
    expect(normalizeForSpeech("[資料](https://example.com) と `設定値` を確認します。")).toBe("資料 と 設定値 を確認します。");
  });

  it("keeps bare URLs when enabled", () => {
    expect(normalizeForSpeech("詳細は https://example.com/path を確認します。", { readUrls: true })).toBe("詳細は https://example.com/path を確認します。");
  });
});
