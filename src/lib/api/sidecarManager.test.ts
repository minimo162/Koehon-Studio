import { describe, expect, it } from "vitest";
import { deriveCodecDir } from "./sidecarManager";

describe("deriveCodecDir", () => {
  it("returns the sibling moss-audio-tokenizer path on POSIX layouts", () => {
    expect(deriveCodecDir("/home/me/models/moss-tts-nano")).toBe(
      "/home/me/models/moss-audio-tokenizer",
    );
  });

  it("strips a trailing slash before deriving", () => {
    expect(deriveCodecDir("/home/me/models/moss-tts-nano/")).toBe(
      "/home/me/models/moss-audio-tokenizer",
    );
  });

  it("handles Windows-style paths", () => {
    expect(deriveCodecDir("C:\\models\\moss-tts-nano")).toBe(
      "C:\\models\\moss-audio-tokenizer",
    );
  });

  it("returns empty for an empty input", () => {
    expect(deriveCodecDir("")).toBe("");
  });

  it("returns empty when the path has no usable parent", () => {
    expect(deriveCodecDir("/moss-tts-nano")).toBe("");
    expect(deriveCodecDir("moss-tts-nano")).toBe("");
  });
});
