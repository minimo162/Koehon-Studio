import type { Chapter, ManuscriptMetadata, ProjectSettings } from "../project/projectTypes";
import { defaultProjectSettings } from "../project/projectTypes";
import { createChunks } from "./chunker";
import { normalizeForSpeech } from "./normalizer";

export type ParsedManuscript = {
  metadata: ManuscriptMetadata;
  body: string;
  chapters: Chapter[];
  warnings: string[];
};

const knownMetadataKeys = new Set(["title", "source_type", "source", "audience", "language", "style", "version"]);

export function parseManuscript(raw: string, settings: ProjectSettings = defaultProjectSettings): ParsedManuscript {
  const { metadata, body, warnings } = extractFrontMatter(raw);
  const chapters = splitChapters(body, settings);
  if (!metadata.title) {
    metadata.title = chapters.find((chapter) => chapter.title !== "本文")?.title ?? "無題";
  }
  return { metadata, body, chapters, warnings };
}

export function extractFrontMatter(raw: string): { metadata: ManuscriptMetadata; body: string; warnings: string[] } {
  const warnings: string[] = [];
  if (!raw.startsWith("---\n") && !raw.startsWith("---\r\n")) {
    return { metadata: {}, body: raw, warnings };
  }

  const normalized = raw.replace(/\r\n/g, "\n");
  const end = normalized.indexOf("\n---", 4);
  if (end < 0) {
    warnings.push("front matter の終了区切りが見つかりませんでした。");
    return { metadata: {}, body: raw, warnings };
  }

  const block = normalized.slice(4, end).trim();
  const body = normalized.slice(end + 4).replace(/^\n/, "");
  const metadata: ManuscriptMetadata = {};

  for (const line of block.split("\n")) {
    const match = /^([A-Za-z0-9_-]+):\s*(.*)$/.exec(line);
    if (!match) {
      warnings.push(`front matter の行を読み飛ばしました: ${line}`);
      continue;
    }
    const key = match[1];
    if (!knownMetadataKeys.has(key)) continue;
    metadata[key] = coerceValue(match[2].trim().replace(/^["']|["']$/g, ""));
  }

  return { metadata, body, warnings };
}

function coerceValue(value: string): string | number {
  if (/^\d+$/.test(value)) return Number(value);
  return value;
}

export function splitChapters(body: string, settings: ProjectSettings = defaultProjectSettings): Chapter[] {
  const lines = body.replace(/\r\n/g, "\n").split("\n");
  const headingIndexes: Array<{ index: number; title: string }> = [];

  lines.forEach((line, index) => {
    const match = /^#\s+(.+?)\s*$/.exec(line);
    if (match) headingIndexes.push({ index, title: match[1].trim() });
  });

  if (headingIndexes.length === 0) {
    const rawMarkdown = body.trim();
    const chapter: Chapter = {
      id: "chapter-001",
      title: "本文",
      level: 1,
      order: 1,
      rawMarkdown,
      plainText: normalizeForSpeech(rawMarkdown, { readUrls: settings.readUrls }),
      includeInNarration: true,
      chunks: []
    };
    chapter.chunks = createChunks(chapter.id, chapter.rawMarkdown, settings);
    return [chapter];
  }

  const chapters: Chapter[] = [];
  const preface = lines.slice(0, headingIndexes[0].index).join("\n").trim();
  if (preface) {
    chapters.push(buildChapter(chapters.length + 1, "本文", preface, true, settings));
  }

  for (const [index, heading] of headingIndexes.entries()) {
    const next = headingIndexes[index + 1]?.index ?? lines.length;
    const rawMarkdown = lines.slice(heading.index, next).join("\n").trim();
    const title = heading.title;
    const includeInNarration = settings.includeManuscriptMemo || !/^原稿作成メモ/.test(title);
    chapters.push(buildChapter(chapters.length + 1, title, rawMarkdown, includeInNarration, settings));
  }

  return chapters;
}

function buildChapter(order: number, title: string, rawMarkdown: string, includeInNarration: boolean, settings: ProjectSettings): Chapter {
  const chapter: Chapter = {
    id: `chapter-${String(order).padStart(3, "0")}`,
    title,
    level: 1,
    order,
    rawMarkdown,
    plainText: normalizeForSpeech(rawMarkdown, { readUrls: settings.readUrls }),
    includeInNarration,
    chunks: []
  };
  chapter.chunks = createChunks(chapter.id, rawMarkdown, settings);
  return chapter;
}
