import { derived } from "svelte/store";
import { defaultGenerationState, type Project } from "../project/projectTypes";
import { appSettingsStore } from "./appSettings";
import { manuscriptStore } from "./manuscriptStore";

export const projectStore = derived([manuscriptStore, appSettingsStore], ([$manuscript, $settings]): Project => {
  const now = new Date().toISOString();
  const title = $manuscript.parsed?.metadata.title ?? "無題";
  const chapters = ($manuscript.parsed?.chapters ?? []).map((chapter) => {
    const override = $manuscript.chapterInclusion[chapter.id];
    if (override === undefined) return chapter;
    return {
      ...chapter,
      includeInNarration: override
    };
  });
  return {
    id: "local-preview",
    title,
    createdAt: now,
    updatedAt: now,
    projectDir: $manuscript.projectDir,
    manuscriptPath: $manuscript.filePath ?? $manuscript.fileName,
    metadata: $manuscript.parsed?.metadata ?? { title },
    chapters,
    settings: $settings,
    generation: defaultGenerationState
  };
});
