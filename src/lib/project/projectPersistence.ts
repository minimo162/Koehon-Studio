import { dirname, join } from "@tauri-apps/api/path";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  exists,
  mkdir,
  readTextFile,
  writeTextFile,
} from "@tauri-apps/plugin-fs";
import type {
  ChunkStatus,
  GenerationState,
  ManuscriptChunk,
  Project,
  ProjectSettings,
} from "./projectTypes";
import { normalizeProjectSettings } from "../stores/appSettings";

export type ProjectSnapshot = {
  schemaVersion: 1;
  title: string;
  createdAt: string;
  updatedAt: string;
  manuscriptFile: string;
  metadata: Project["metadata"];
  settings: ProjectSettings;
  chapterInclusion: Record<string, boolean>;
  generation: GenerationState;
  chunks: Record<string, PersistedChunkState>;
};

export type PersistedChunkState = {
  status: ChunkStatus;
  audioPath?: string;
  error?: string;
  retryCount: number;
};

export type SavedProject = {
  projectFilePath: string;
  projectDir: string;
  manuscriptPath: string;
  snapshot: ProjectSnapshot;
};

export type LoadedProject = SavedProject & {
  rawManuscript: string;
  missingAudioPaths: string[];
};

const projectFileName = "project.json";
const manuscriptFileName = "manuscript.md";
const projectFilters = [{ name: "Koehon Project", extensions: ["json"] }];

export function createProjectSnapshot(
  project: Project,
  rawManuscript: string,
  chapterInclusion: Record<string, boolean>,
  chunks: Record<string, ManuscriptChunk>,
): ProjectSnapshot {
  const now = new Date().toISOString();
  return {
    schemaVersion: 1,
    title: project.title,
    createdAt: project.createdAt,
    updatedAt: now,
    manuscriptFile: manuscriptFileName,
    metadata: project.metadata,
    settings: project.settings,
    chapterInclusion,
    generation: normalizeGenerationState(project.generation),
    chunks: Object.fromEntries(
      Object.entries(chunks).map(([id, chunk]) => [
        id,
        {
          status: normalizeChunkStatus(chunk.status),
          audioPath: chunk.audioPath,
          error: chunk.error,
          retryCount: chunk.retryCount,
        },
      ]),
    ),
  };
}

export async function saveProjectWithDialog(
  project: Project,
  rawManuscript: string,
  chapterInclusion: Record<string, boolean>,
  chunks: Record<string, ManuscriptChunk>,
  currentProjectFilePath?: string,
): Promise<SavedProject | undefined> {
  const projectFilePath =
    currentProjectFilePath ??
    (await save({
      title: "プロジェクトを保存",
      filters: projectFilters,
      defaultPath: projectFileName,
    }));
  if (!projectFilePath) return undefined;
  return writeProject(
    projectFilePath,
    project,
    rawManuscript,
    chapterInclusion,
    chunks,
  );
}

export async function openProjectWithDialog(): Promise<
  LoadedProject | undefined
> {
  const selected = await open({
    title: "プロジェクトを開く",
    multiple: false,
    filters: projectFilters,
  });
  if (!selected || Array.isArray(selected)) return undefined;
  return readProject(selected);
}

export async function openProjectPath(path: string): Promise<LoadedProject> {
  return readProject(path);
}

async function writeProject(
  projectFilePath: string,
  project: Project,
  rawManuscript: string,
  chapterInclusion: Record<string, boolean>,
  chunks: Record<string, ManuscriptChunk>,
): Promise<SavedProject> {
  const projectDir = await dirname(projectFilePath);
  const manuscriptPath = await join(projectDir, manuscriptFileName);
  if (!(await exists(projectDir))) {
    await mkdir(projectDir, { recursive: true });
  }
  const snapshot = createProjectSnapshot(
    project,
    rawManuscript,
    chapterInclusion,
    chunks,
  );
  await writeTextFile(manuscriptPath, rawManuscript);
  await writeTextFile(
    projectFilePath,
    `${JSON.stringify(snapshot, null, 2)}\n`,
  );
  return { projectFilePath, projectDir, manuscriptPath, snapshot };
}

async function readProject(projectFilePath: string): Promise<LoadedProject> {
  const projectDir = await dirname(projectFilePath);
  const rawSnapshot = await readTextFile(projectFilePath);
  const snapshot = parseProjectSnapshot(rawSnapshot);
  const manuscriptPath = await join(
    projectDir,
    snapshot.manuscriptFile || manuscriptFileName,
  );
  if (!(await exists(manuscriptPath))) {
    throw new Error(
      `${snapshot.manuscriptFile || manuscriptFileName} が見つかりません。project.json と同じフォルダに配置してください。`,
    );
  }
  const rawManuscript = await readTextFile(manuscriptPath);
  const missingAudioPaths = await findMissingAudioPaths(snapshot);
  return {
    projectFilePath,
    projectDir,
    manuscriptPath,
    snapshot,
    rawManuscript,
    missingAudioPaths,
  };
}

async function findMissingAudioPaths(
  snapshot: ProjectSnapshot,
): Promise<string[]> {
  const paths = new Set(
    Object.values(snapshot.chunks)
      .map((chunk) => chunk.audioPath)
      .filter((path): path is string => Boolean(path)),
  );
  const missing: string[] = [];
  for (const path of paths) {
    if (!(await exists(path))) missing.push(path);
  }
  return missing;
}

function parseProjectSnapshot(raw: string): ProjectSnapshot {
  const parsed = JSON.parse(raw) as Partial<ProjectSnapshot>;
  if (parsed.schemaVersion !== 1) {
    throw new Error("未対応の project.json 形式です。");
  }
  if (!parsed.settings || !parsed.metadata || !parsed.manuscriptFile) {
    throw new Error("project.json に必要な情報がありません。");
  }
  return {
    schemaVersion: 1,
    title: parsed.title ?? "無題",
    createdAt: parsed.createdAt ?? new Date().toISOString(),
    updatedAt: parsed.updatedAt ?? new Date().toISOString(),
    manuscriptFile: parsed.manuscriptFile,
    metadata: parsed.metadata,
    settings: normalizeProjectSettings(parsed.settings),
    chapterInclusion: parsed.chapterInclusion ?? {},
    generation: normalizeGenerationState(
      parsed.generation ?? {
        status: "idle",
        totalChunks: 0,
        completedChunks: 0,
        failedChunks: 0,
      },
    ),
    chunks: Object.fromEntries(
      Object.entries(parsed.chunks ?? {}).map(([id, chunk]) => [
        id,
        {
          ...chunk,
          status: normalizeChunkStatus(chunk.status),
          retryCount: chunk.retryCount ?? 0,
        },
      ]),
    ),
  };
}

export function normalizeGenerationState(
  state: GenerationState,
): GenerationState {
  const inactiveStatus =
    state.status === "running" ||
    state.status === "stopping" ||
    state.status === "paused"
      ? "idle"
      : state.status;
  return {
    status: inactiveStatus,
    totalChunks: state.totalChunks,
    completedChunks: state.completedChunks,
    failedChunks: state.failedChunks,
    startedAt: inactiveStatus === state.status ? state.startedAt : undefined,
    finishedAt: inactiveStatus === state.status ? state.finishedAt : undefined,
  };
}

function normalizeChunkStatus(status: ChunkStatus | undefined): ChunkStatus {
  return status === "generating" ? "pending" : (status ?? "pending");
}
