import { beforeEach, describe, expect, it } from "vitest";
import { formatError, reportError } from "./errors";
import { generationLogsStore } from "../stores/generationQueue";

describe("formatError", () => {
  it("extracts message from Error instances", () => {
    expect(formatError(new Error("boom"))).toBe("boom");
  });

  it("returns strings as-is", () => {
    expect(formatError("raw message")).toBe("raw message");
  });

  it("serialises plain objects", () => {
    expect(formatError({ code: 42, reason: "nope" })).toBe(
      '{"code":42,"reason":"nope"}',
    );
  });

  it("falls back for null/undefined", () => {
    expect(formatError(null)).toBe("不明なエラーが発生しました。");
    expect(formatError(undefined)).toBe("不明なエラーが発生しました。");
  });
});

describe("reportError", () => {
  beforeEach(() => generationLogsStore.set([]));

  it("logs the contextualised message and returns it", () => {
    const message = reportError("保存", new Error("disk full"));
    expect(message).toBe("disk full");
    const logs = [] as Array<{ level: string; message: string }>;
    generationLogsStore.subscribe((l) => logs.push(...l))();
    expect(logs[0].level).toBe("error");
    expect(logs[0].message).toBe("保存: disk full");
  });
});
