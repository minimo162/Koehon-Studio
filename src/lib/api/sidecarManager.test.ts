import { describe, expect, it } from "vitest";
import { deriveCodecDir } from "./sidecarManager";

// The codec dir is derived from the model dir by swapping the last path
// segment for `semantic-dacvae` — matches how auto-setup lays the two
// Hugging Face repos out as siblings.
describe("deriveCodecDir", () => {
  it("returns the sibling semantic-dacvae path on POSIX layouts", () => {
    expect(deriveCodecDir("/home/me/models/irodori-tts")).toBe(
      "/home/me/models/semantic-dacvae",
    );
  });

  it("strips a trailing slash before deriving", () => {
    expect(deriveCodecDir("/home/me/models/irodori-tts/")).toBe(
      "/home/me/models/semantic-dacvae",
    );
  });

  it("handles Windows-style paths", () => {
    expect(deriveCodecDir("C:\\models\\irodori-tts")).toBe(
      "C:\\models\\semantic-dacvae",
    );
  });

  it("returns empty for an empty input", () => {
    expect(deriveCodecDir("")).toBe("");
  });

  it("returns empty when the path has no usable parent", () => {
    expect(deriveCodecDir("/irodori-tts")).toBe("");
    expect(deriveCodecDir("irodori-tts")).toBe("");
  });
});
