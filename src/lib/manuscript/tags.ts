import type { ManuscriptTag, ProjectSettings } from "../project/projectTypes";
import { defaultProjectSettings } from "../project/projectTypes";

const tagPattern = /\[([A-Za-z]+)(?::([^\]\n]+))?\]/g;

export function parseTags(text: string): ManuscriptTag[] {
  const tags: ManuscriptTag[] = [];
  for (const match of text.matchAll(tagPattern)) {
    const keyword = match[1].toLowerCase();
    const value = match[2]?.trim();
    tags.push({
      raw: match[0],
      name: knownTagName(keyword),
      value
    });
  }
  return tags;
}

export function pauseMsFor(value: string | undefined, settings: ProjectSettings = defaultProjectSettings): number {
  switch ((value ?? "").toLowerCase()) {
    case "short":
      return settings.pauseShortMs;
    case "long":
      return settings.pauseLongMs;
    case "medium":
    default:
      return settings.pauseMediumMs;
  }
}

export function stripControlTags(text: string): string {
  return text.replace(tagPattern, "");
}

export function splitByPauseTags(text: string): Array<{ type: "text"; value: string } | { type: "pause"; tag: ManuscriptTag }> {
  const parts: Array<{ type: "text"; value: string } | { type: "pause"; tag: ManuscriptTag }> = [];
  let cursor = 0;
  const pausePattern = /\[pause:(short|medium|long)\]/gi;
  for (const match of text.matchAll(pausePattern)) {
    if (match.index > cursor) {
      parts.push({ type: "text", value: text.slice(cursor, match.index) });
    }
    parts.push({ type: "pause", tag: { raw: match[0], name: "pause", value: match[1].toLowerCase() } });
    cursor = match.index + match[0].length;
  }
  if (cursor < text.length) {
    parts.push({ type: "text", value: text.slice(cursor) });
  }
  return parts;
}

function knownTagName(keyword: string): ManuscriptTag["name"] {
  switch (keyword) {
    case "pause":
    case "voice":
    case "speed":
    case "chapter":
      return keyword;
    default:
      return "unknown";
  }
}
