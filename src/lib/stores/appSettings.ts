import { writable } from "svelte/store";
import { defaultProjectSettings, type ProjectSettings } from "../project/projectTypes";

const storageKey = "koehon-studio-settings";

type PersistedProjectSettings = Partial<ProjectSettings> & Record<string, unknown>;

function loadSettings(): ProjectSettings {
  if (typeof localStorage === "undefined") return defaultProjectSettings;
  const stored = localStorage.getItem(storageKey);
  if (!stored) return defaultProjectSettings;
  try {
    return normalizeProjectSettings(JSON.parse(stored) as PersistedProjectSettings);
  } catch {
    return defaultProjectSettings;
  }
}

export const appSettingsStore = writable<ProjectSettings>(loadSettings());

appSettingsStore.subscribe((settings) => {
  if (typeof localStorage !== "undefined") {
    localStorage.setItem(storageKey, JSON.stringify(settings));
  }
});

export function normalizeProjectSettings(input: PersistedProjectSettings): ProjectSettings {
  return {
    ...defaultProjectSettings,
    ttsEngine: "moss-tts-nano-onnx",
    voice: normalizeText(input.voice, defaultProjectSettings.voice),
    modelDirectory: normalizeText(input.modelDirectory, ""),
    outputDirectory: normalizeText(input.outputDirectory, ""),
    cpuThreads: clampInteger(input.cpuThreads, 1, 32, defaultProjectSettings.cpuThreads),
    maxChunkChars: clampInteger(input.maxChunkChars, 100, 1200, defaultProjectSettings.maxChunkChars),
    pauseShortMs: clampInteger(input.pauseShortMs, 0, 10_000, defaultProjectSettings.pauseShortMs),
    pauseMediumMs: clampInteger(input.pauseMediumMs, 0, 10_000, defaultProjectSettings.pauseMediumMs),
    pauseLongMs: clampInteger(input.pauseLongMs, 0, 10_000, defaultProjectSettings.pauseLongMs),
    outputSampleRate: clampInteger(input.outputSampleRate, 8_000, 192_000, defaultProjectSettings.outputSampleRate),
    exportFormat: "wav",
    includeManuscriptMemo: typeof input.includeManuscriptMemo === "boolean" ? input.includeManuscriptMemo : defaultProjectSettings.includeManuscriptMemo,
    readUrls: typeof input.readUrls === "boolean" ? input.readUrls : defaultProjectSettings.readUrls,
  };
}

export function validateProjectSettings(settings: ProjectSettings): string[] {
  const errors: string[] = [];
  if (!isIntegerInRange(settings.cpuThreads, 1, 32)) {
    errors.push("CPUスレッド数は 1 から 32 の範囲で指定してください。");
  }
  if (!isIntegerInRange(settings.maxChunkChars, 100, 1200)) {
    errors.push("最大チャンク文字数は 100 から 1200 の範囲で指定してください。");
  }
  if (!isIntegerInRange(settings.pauseShortMs, 0, 10_000)) {
    errors.push("短い無音は 0 から 10000 ms の範囲で指定してください。");
  }
  if (!isIntegerInRange(settings.pauseMediumMs, 0, 10_000)) {
    errors.push("標準無音は 0 から 10000 ms の範囲で指定してください。");
  }
  if (!isIntegerInRange(settings.pauseLongMs, 0, 10_000)) {
    errors.push("長い無音は 0 から 10000 ms の範囲で指定してください。");
  }
  if (!isIntegerInRange(settings.outputSampleRate, 8_000, 192_000)) {
    errors.push("サンプルレートは 8000 から 192000 Hz の範囲で指定してください。");
  }
  if (settings.exportFormat !== "wav") {
    errors.push("初期版の書き出し形式は wav のみ利用できます。");
  }
  return errors;
}

function normalizeText(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value.trim() : fallback;
}

function clampInteger(value: unknown, min: number, max: number, fallback: number): number {
  const numberValue = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(numberValue)) return fallback;
  return Math.min(max, Math.max(min, Math.round(numberValue)));
}

function isIntegerInRange(value: number, min: number, max: number): boolean {
  return Number.isInteger(value) && value >= min && value <= max;
}
