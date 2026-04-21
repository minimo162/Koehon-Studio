import { appDataDir, join } from "@tauri-apps/api/path";
import { open } from "@tauri-apps/plugin-dialog";
import {
  exists,
  mkdir,
  readTextFile,
  writeTextFile,
} from "@tauri-apps/plugin-fs";
import type { ProjectSettings } from "../project/projectTypes";
import {
  normalizeProjectSettings,
  validateProjectSettings,
} from "../stores/appSettings";
import { isTauriRuntime } from "./fileAccess";

const settingsFileName = "settings.json";

export type LoadedSettingsFile = {
  path: string;
  settings: ProjectSettings;
};

export async function getSettingsFilePath(): Promise<string | undefined> {
  if (!isTauriRuntime()) return undefined;
  return join(await appDataDir(), settingsFileName);
}

export async function loadSettingsFile(): Promise<LoadedSettingsFile | undefined> {
  const path = await getSettingsFilePath();
  if (!path || !(await exists(path))) return undefined;
  const raw = await readTextFile(path);
  return {
    path,
    settings: normalizeProjectSettings(JSON.parse(raw)),
  };
}

export async function saveSettingsFile(
  settings: ProjectSettings,
): Promise<string | undefined> {
  const path = await getSettingsFilePath();
  if (!path) return undefined;
  const errors = validateProjectSettings(settings);
  if (errors.length > 0) {
    throw new Error(errors.join("\n"));
  }
  const dir = await appDataDir();
  if (!(await exists(dir))) {
    await mkdir(dir, { recursive: true });
  }
  await writeTextFile(path, `${JSON.stringify(settings, null, 2)}\n`);
  return path;
}

export async function selectDirectory(
  title: string,
): Promise<string | undefined> {
  if (!isTauriRuntime()) return undefined;
  const selected = await open({
    title,
    directory: true,
    multiple: false,
  });
  if (!selected || Array.isArray(selected)) return undefined;
  return selected;
}
