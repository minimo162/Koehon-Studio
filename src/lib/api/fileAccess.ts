import { open, save } from "@tauri-apps/plugin-dialog";
import { readTextFile, writeTextFile } from "@tauri-apps/plugin-fs";

export type ManuscriptFile = {
  path: string;
  name: string;
  contents: string;
};

const manuscriptFilters = [
  {
    name: "Manuscript",
    extensions: ["md", "txt"]
  }
];

export function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export async function openManuscriptWithDialog(): Promise<ManuscriptFile | undefined> {
  if (!isTauriRuntime()) return undefined;
  const selected = await open({
    multiple: false,
    filters: manuscriptFilters
  });
  if (!selected || Array.isArray(selected)) return undefined;
  const contents = await readTextFile(selected);
  return {
    path: selected,
    name: basename(selected),
    contents
  };
}

export async function openManuscriptPath(path: string): Promise<ManuscriptFile | undefined> {
  if (!isTauriRuntime()) return undefined;
  const contents = await readTextFile(path);
  return {
    path,
    name: basename(path),
    contents
  };
}

export async function saveManuscriptFile(contents: string, currentPath?: string): Promise<{ path: string; name: string } | undefined> {
  if (!isTauriRuntime()) return undefined;
  const path =
    currentPath ??
    (await save({
      filters: manuscriptFilters,
      defaultPath: "manuscript.md"
    }));
  if (!path) return undefined;
  await writeTextFile(path, contents);
  return {
    path,
    name: basename(path)
  };
}

function basename(path: string): string {
  return path.split(/[\\/]/).at(-1) ?? path;
}
