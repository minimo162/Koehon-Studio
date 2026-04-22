import { describe, expect, it } from "vitest";
import { buildPrompt, sourceTypeInstructions } from "./promptTemplates";

describe("buildPrompt", () => {
  it("embeds the selected options into the prompt body", () => {
    const prompt = buildPrompt({
      sourceType: "PowerPoint / スライド資料",
      audience: "新入社員",
      style: "落ち着いた研修講師風",
      length: "15分程度",
      purpose: "通勤中の復習",
    });
    expect(prompt).toContain("元資料種別: PowerPoint / スライド資料");
    expect(prompt).toContain("対象読者: 新入社員");
    expect(prompt).toContain("文体: 落ち着いた研修講師風");
    expect(prompt).toContain("長さ: 15分程度");
    expect(prompt).toContain("目的: 通勤中の復習");
  });

  it("picks the sourceType-specific additional instruction", () => {
    const prompt = buildPrompt({
      sourceType: "Excel / 表データ",
      audience: "経営層",
      style: "簡潔",
      length: "5分",
      purpose: "レポート共有",
    });
    expect(prompt).toContain(sourceTypeInstructions["Excel / 表データ"]);
  });

  it("falls back to the default instruction for unknown source types", () => {
    const prompt = buildPrompt({
      sourceType: "未登録の種別",
      audience: "A",
      style: "B",
      length: "C",
      purpose: "D",
    });
    expect(prompt).toContain(sourceTypeInstructions["その他"]);
  });

  it("includes structural requirements for front matter, pause tags, and memo section", () => {
    const prompt = buildPrompt({
      sourceType: "その他",
      audience: "一般",
      style: "中立",
      length: "10分",
      purpose: "紹介",
    });
    expect(prompt).toContain("front matter");
    expect(prompt).toContain("[pause:short]");
    expect(prompt).toContain("[pause:medium]");
    expect(prompt).toContain("[pause:long]");
    expect(prompt).toContain("# 原稿作成メモ");
  });
});
