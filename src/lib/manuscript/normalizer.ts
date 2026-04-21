import { stripControlTags } from "./tags";

export type NormalizeOptions = {
  readUrls?: boolean;
};

export function normalizeForSpeech(markdown: string, options: NormalizeOptions = {}): string {
  let text = markdown.replace(/^#{1,6}\s+/gm, "");
  text = text.replace(/```[\s\S]*?```/g, "");
  text = text.replace(/`([^`]+)`/g, "$1");
  text = text.replace(/!\[([^\]]*)\]\([^)]+\)/g, "$1");
  text = text.replace(/\[([^\]]+)\]\([^)]+\)/g, "$1");
  text = text.replace(/^>\s?/gm, "");
  text = text.replace(/^\s*[-*+]\s+/gm, "");
  text = text.replace(/^\s*\d+\.\s+/gm, "");
  text = text.replace(/[*_~]{1,3}/g, "");
  text = stripControlTags(text);
  if (!options.readUrls) {
    text = text.replace(/https?:\/\/\S+/g, "");
  }
  text = text.replace(/[ \t]+/g, " ");
  text = text.replace(/\n{3,}/g, "\n\n");
  return text.trim();
}
