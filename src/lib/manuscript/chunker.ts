import type { ManuscriptChunk, ProjectSettings } from "../project/projectTypes";
import { defaultProjectSettings } from "../project/projectTypes";
import { normalizeForSpeech } from "./normalizer";
import { parseTags, pauseMsFor, splitByPauseTags } from "./tags";

export function createChunks(chapterId: string, rawMarkdown: string, settings: ProjectSettings = defaultProjectSettings): ManuscriptChunk[] {
  const chunks: ManuscriptChunk[] = [];
  let order = 1;

  for (const part of splitByPauseTags(rawMarkdown)) {
    if (part.type === "pause") {
      chunks.push({
        id: `${chapterId}-chunk-${String(order).padStart(3, "0")}`,
        chapterId,
        order,
        type: "pause",
        pauseMs: pauseMsFor(part.tag.value, settings),
        tags: [part.tag],
        status: "pending",
        retryCount: 0
      });
      order += 1;
      continue;
    }

    const normalized = normalizeForSpeech(part.value);
    for (const text of splitText(normalized, settings.maxChunkChars)) {
      chunks.push({
        id: `${chapterId}-chunk-${String(order).padStart(3, "0")}`,
        chapterId,
        order,
        type: "text",
        text,
        tags: parseTags(part.value).filter((tag) => tag.name !== "pause"),
        status: "pending",
        retryCount: 0
      });
      order += 1;
    }
  }

  return chunks;
}

export function splitText(text: string, maxChars = 450): string[] {
  const clean = text.trim();
  if (!clean) return [];

  const paragraphs = clean.split(/\n{2,}/).map((part) => part.trim()).filter(Boolean);
  const chunks: string[] = [];
  let current = "";

  for (const paragraph of paragraphs) {
    for (const segment of splitLongText(paragraph, maxChars)) {
      if (!current) {
        current = segment;
      } else if ((current + "\n\n" + segment).length <= maxChars) {
        current = `${current}\n\n${segment}`;
      } else {
        chunks.push(current);
        current = segment;
      }
    }
  }
  if (current) chunks.push(current);
  return chunks;
}

function splitLongText(text: string, maxChars: number): string[] {
  if (text.length <= maxChars) return [text];
  const result: string[] = [];
  let rest = text.trim();
  while (rest.length > maxChars) {
    const index = findSplitIndex(rest, maxChars);
    result.push(rest.slice(0, index).trim());
    rest = rest.slice(index).trim();
  }
  if (rest) result.push(rest);
  return result;
}

function findSplitIndex(text: string, maxChars: number): number {
  const window = text.slice(0, maxChars + 1);
  const boundaries = ["。", "？", "！", "\n", "、", " "];
  for (const boundary of boundaries) {
    const index = window.lastIndexOf(boundary);
    if (index >= Math.floor(maxChars * 0.45)) {
      return index + boundary.length;
    }
  }
  return maxChars;
}
