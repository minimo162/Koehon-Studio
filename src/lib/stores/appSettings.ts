import { writable } from "svelte/store";
import { defaultProjectSettings, type ProjectSettings } from "../project/projectTypes";

const storageKey = "koehon-studio-settings";

function loadSettings(): ProjectSettings {
  if (typeof localStorage === "undefined") return defaultProjectSettings;
  const stored = localStorage.getItem(storageKey);
  if (!stored) return defaultProjectSettings;
  try {
    return { ...defaultProjectSettings, ...JSON.parse(stored) };
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
