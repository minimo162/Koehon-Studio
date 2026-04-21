import { writable } from "svelte/store";

export type RecentProject = {
  path: string;
  name: string;
  openedAt: string;
};

const storageKey = "koehon-studio-recent-projects";
const maxRecentProjects = 8;

function loadRecentProjects(): RecentProject[] {
  if (typeof localStorage === "undefined") return [];
  const stored = localStorage.getItem(storageKey);
  if (!stored) return [];
  try {
    const parsed = JSON.parse(stored);
    return Array.isArray(parsed) ? parsed.filter(isRecentProject).slice(0, maxRecentProjects) : [];
  } catch {
    return [];
  }
}

export const recentProjectsStore = writable<RecentProject[]>(loadRecentProjects());

recentProjectsStore.subscribe((projects) => {
  if (typeof localStorage !== "undefined") {
    localStorage.setItem(storageKey, JSON.stringify(projects.slice(0, maxRecentProjects)));
  }
});

export function rememberRecentProject(path: string, name: string): void {
  recentProjectsStore.update((projects) => [
    { path, name, openedAt: new Date().toISOString() },
    ...projects.filter((project) => project.path !== path)
  ].slice(0, maxRecentProjects));
}

function isRecentProject(value: unknown): value is RecentProject {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Record<string, unknown>;
  return typeof candidate.path === "string" && typeof candidate.name === "string" && typeof candidate.openedAt === "string";
}
