import { writable } from "svelte/store";

export type RecentFile = {
  path: string;
  name: string;
  openedAt: string;
};

const storageKey = "koehon-studio-recent-files";
const maxRecentFiles = 8;

function loadRecentFiles(): RecentFile[] {
  if (typeof localStorage === "undefined") return [];
  const stored = localStorage.getItem(storageKey);
  if (!stored) return [];
  try {
    const parsed = JSON.parse(stored);
    return Array.isArray(parsed) ? parsed.filter(isRecentFile).slice(0, maxRecentFiles) : [];
  } catch {
    return [];
  }
}

export const recentFilesStore = writable<RecentFile[]>(loadRecentFiles());

recentFilesStore.subscribe((files) => {
  if (typeof localStorage !== "undefined") {
    localStorage.setItem(storageKey, JSON.stringify(files.slice(0, maxRecentFiles)));
  }
});

export function rememberRecentFile(path: string, name: string): void {
  recentFilesStore.update((files) => [
    { path, name, openedAt: new Date().toISOString() },
    ...files.filter((file) => file.path !== path)
  ].slice(0, maxRecentFiles));
}

function isRecentFile(value: unknown): value is RecentFile {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Record<string, unknown>;
  return typeof candidate.path === "string" && typeof candidate.name === "string" && typeof candidate.openedAt === "string";
}
