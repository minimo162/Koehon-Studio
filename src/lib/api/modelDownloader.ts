import { appDataDir, join } from "@tauri-apps/api/path";
import { create, mkdir } from "@tauri-apps/plugin-fs";

import { isTauriRuntime } from "./fileAccess";

export type HuggingFaceFile = {
  path: string;
  size: number;
};

export type DownloadPlan = {
  repo: string;
  baseUrl: string;
  destinationDir: string;
  files: HuggingFaceFile[];
  totalBytes: number;
};

export type DownloadProgress = {
  stage: "listing" | "downloading" | "complete";
  repo: string;
  currentFile?: string;
  fileIndex: number;
  fileCount: number;
  fileBytes: number;
  fileTotalBytes: number;
  overallBytes: number;
  overallTotalBytes: number;
};

type DownloadOptions = {
  signal?: AbortSignal;
  onProgress?: (progress: DownloadProgress) => void;
};

export class ModelDownloadError extends Error {
  constructor(message: string, public readonly cause?: unknown) {
    super(message);
  }
}

export async function listHuggingFaceFiles(repo: string): Promise<HuggingFaceFile[]> {
  const url = `https://huggingface.co/api/models/${repo}/tree/main`;
  let response: Response;
  try {
    response = await fetch(url, { headers: { Accept: "application/json" } });
  } catch (error) {
    throw new ModelDownloadError(`Hugging Face APIへ接続できませんでした: ${repo}`, error);
  }
  if (!response.ok) {
    throw new ModelDownloadError(`Hugging Face APIが ${response.status} を返しました (${repo})`);
  }
  const raw = (await response.json()) as Array<{
    type: string;
    path: string;
    size?: number;
    lfs?: { size?: number };
  }>;
  return raw
    .filter((entry) => entry.type === "file")
    .map((entry) => ({
      path: entry.path,
      size: entry.lfs?.size ?? entry.size ?? 0
    }));
}

export async function planDownload(repo: string, destinationDir: string): Promise<DownloadPlan> {
  const files = await listHuggingFaceFiles(repo);
  const totalBytes = files.reduce((sum, file) => sum + file.size, 0);
  return {
    repo,
    baseUrl: `https://huggingface.co/${repo}/resolve/main/`,
    destinationDir,
    files,
    totalBytes
  };
}

export async function downloadHuggingFaceRepo(
  plan: DownloadPlan,
  options: DownloadOptions = {}
): Promise<void> {
  if (!isTauriRuntime()) {
    throw new ModelDownloadError("モデルダウンロードは Tauri アプリ上でのみ利用できます。");
  }

  await mkdir(plan.destinationDir, { recursive: true });

  let overallBytes = 0;
  options.onProgress?.({
    stage: "listing",
    repo: plan.repo,
    fileIndex: 0,
    fileCount: plan.files.length,
    fileBytes: 0,
    fileTotalBytes: 0,
    overallBytes,
    overallTotalBytes: plan.totalBytes
  });

  for (let index = 0; index < plan.files.length; index += 1) {
    if (options.signal?.aborted) throw new ModelDownloadError("ダウンロードが中断されました。");

    const file = plan.files[index];
    const sourceUrl = `${plan.baseUrl}${encodeURI(file.path)}`;
    const destination = joinPath(plan.destinationDir, file.path);
    const parent = destination.slice(0, destination.lastIndexOf(separator(destination)));
    if (parent && parent !== plan.destinationDir) {
      await mkdir(parent, { recursive: true });
    }

    let response: Response;
    try {
      response = await fetch(sourceUrl, { signal: options.signal });
    } catch (error) {
      throw new ModelDownloadError(`${file.path} を取得できませんでした`, error);
    }
    if (!response.ok) {
      throw new ModelDownloadError(`${file.path}: HTTP ${response.status}`);
    }
    if (!response.body) {
      throw new ModelDownloadError(`${file.path}: 応答ボディが空でした`);
    }

    const totalForFile =
      Number(response.headers.get("Content-Length")) || file.size || 0;

    const handle = await create(destination);
    try {
      const reader = response.body.getReader();
      let fileBytes = 0;
      while (true) {
        if (options.signal?.aborted) {
          throw new ModelDownloadError("ダウンロードが中断されました。");
        }
        const { value, done } = await reader.read();
        if (done) break;
        if (value && value.byteLength > 0) {
          await handle.write(value);
          fileBytes += value.byteLength;
          overallBytes += value.byteLength;
          options.onProgress?.({
            stage: "downloading",
            repo: plan.repo,
            currentFile: file.path,
            fileIndex: index,
            fileCount: plan.files.length,
            fileBytes,
            fileTotalBytes: totalForFile,
            overallBytes,
            overallTotalBytes: plan.totalBytes
          });
        }
      }
    } finally {
      await handle.close();
    }
  }

  options.onProgress?.({
    stage: "complete",
    repo: plan.repo,
    fileIndex: plan.files.length,
    fileCount: plan.files.length,
    fileBytes: 0,
    fileTotalBytes: 0,
    overallBytes,
    overallTotalBytes: plan.totalBytes
  });
}

function separator(path: string): string {
  return path.includes("\\") && !path.includes("/") ? "\\" : "/";
}

function joinPath(base: string, child: string): string {
  const sep = separator(base);
  const trimmedBase = base.endsWith(sep) ? base.slice(0, -1) : base;
  const trimmedChild = child.startsWith("/") || child.startsWith("\\") ? child.slice(1) : child;
  return `${trimmedBase}${sep}${trimmedChild.replaceAll("/", sep)}`;
}

/** Known model presets. Each entry is a Hugging Face repo and its recommended destination subdir. */
export const MODEL_PRESETS = [
  {
    id: "moss-tts-nano",
    label: "MOSS-TTS-Nano 100M (ONNX)",
    repo: "OpenMOSS-Team/MOSS-TTS-Nano-100M-ONNX",
    subdir: "moss-tts-nano",
    description: "TTS本体。約672MB。global/local transformer 5段構成の ONNX と tokenizer.model を含みます。"
  },
  {
    id: "moss-audio-tokenizer",
    label: "MOSS Audio Tokenizer (ONNX)",
    repo: "OpenMOSS-Team/MOSS-Audio-Tokenizer-Nano-ONNX",
    subdir: "moss-audio-tokenizer",
    description: "波形 ↔ トークン変換用の ONNX。MOSS-TTS-Nano の前後段で必要。"
  }
] as const;
export type ModelPresetId = (typeof MODEL_PRESETS)[number]["id"];

export type AutoSetupProgress = {
  presetId: ModelPresetId;
  presetLabel: string;
  stepIndex: number;
  stepCount: number;
  detail: DownloadProgress;
};

export type AutoSetupResult = {
  modelDirectory: string;
  codecDirectory: string;
};

/**
 * Resolve the default models root under the app data directory, creating
 * parents as needed. Returns an absolute path like
 * `<appData>/models`.
 */
export async function defaultModelsRoot(): Promise<string> {
  const base = await appDataDir();
  return join(base, "models");
}

/**
 * Download every MODEL_PRESET into `<appData>/models/<subdir>` and report
 * combined progress to the caller. Returns the paths that should be written
 * into user settings (modelDirectory + codecDirectory) so the sidecar can
 * pick them up on its next restart.
 */
export async function autoSetupModels(options: {
  signal?: AbortSignal;
  onProgress?: (progress: AutoSetupProgress) => void;
} = {}): Promise<AutoSetupResult> {
  if (!isTauriRuntime()) {
    throw new ModelDownloadError("自動セットアップは Tauri アプリ上でのみ利用できます。");
  }

  const root = await defaultModelsRoot();
  await mkdir(root, { recursive: true });

  const resolved: Record<ModelPresetId, string> = {
    "moss-tts-nano": "",
    "moss-audio-tokenizer": ""
  };

  for (let index = 0; index < MODEL_PRESETS.length; index += 1) {
    const preset = MODEL_PRESETS[index];
    const targetDir = await join(root, preset.subdir);
    resolved[preset.id] = targetDir;

    const plan = await planDownload(preset.repo, targetDir);
    await downloadHuggingFaceRepo(plan, {
      signal: options.signal,
      onProgress: (detail) =>
        options.onProgress?.({
          presetId: preset.id,
          presetLabel: preset.label,
          stepIndex: index,
          stepCount: MODEL_PRESETS.length,
          detail
        })
    });
  }

  return {
    modelDirectory: resolved["moss-tts-nano"],
    codecDirectory: resolved["moss-audio-tokenizer"]
  };
}
