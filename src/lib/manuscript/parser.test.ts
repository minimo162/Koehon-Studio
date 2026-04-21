import { describe, expect, it } from "vitest";
import { parseManuscript } from "./parser";

describe("parseManuscript", () => {
  it("extracts front matter and top-level chapters", () => {
    const parsed = parseManuscript(`---
title: 研修資料
source_type: PowerPoint
version: 1
---

# はじめに
本文です。

# 原稿作成メモ
確認事項です。
`);

    expect(parsed.metadata.title).toBe("研修資料");
    expect(parsed.metadata.source_type).toBe("PowerPoint");
    expect(parsed.chapters).toHaveLength(2);
    expect(parsed.chapters[0].title).toBe("はじめに");
    expect(parsed.chapters[1].includeInNarration).toBe(false);
  });

  it("uses a single chapter when no heading exists", () => {
    const parsed = parseManuscript("見出しのない本文です。");
    expect(parsed.metadata.title).toBe("無題");
    expect(parsed.chapters).toHaveLength(1);
    expect(parsed.chapters[0].title).toBe("本文");
  });

  it("keeps text before the first heading as a narratable preface chapter", () => {
    const parsed = parseManuscript(`導入文です。

# 第1章
本文です。
`);

    expect(parsed.metadata.title).toBe("第1章");
    expect(parsed.chapters).toHaveLength(2);
    expect(parsed.chapters[0].title).toBe("本文");
    expect(parsed.chapters[0].plainText).toBe("導入文です。");
    expect(parsed.chapters[1].id).toBe("chapter-002");
  });
});
