import { Command, type Child } from "@tauri-apps/plugin-shell";
import { get } from "svelte/store";
import { appSettingsStore } from "../stores/appSettings";
import { isTauriRuntime } from "./fileAccess";
import { ttsClient, type TtsHealth } from "./ttsClient";

export type SidecarStatus = "idle" | "starting" | "loading" | "running" | "failed" | "stopped";

type SidecarEvents = {
  onStatus?: (status: SidecarStatus) => void;
  onLog?: (level: "info" | "error", message: string) => void;
};

let child: Child | undefined;
let starting: Promise<void> | undefined;
let stopping = false;
const sidecarProgram = "../native-tts/sidecars/koehon-tts-sidecar";
const SIDECAR_BOOT_TIMEOUT_MS = 120000;

function buildArgs(): string[] {
  const settings = get(appSettingsStore);
  const args: string[] = [];
  const modelDir = settings.modelDirectory?.trim() ?? "";
  if (modelDir) {
    args.push("--model-dir", modelDir);
  }
  const explicitCodecDir = settings.codecDirectory?.trim() ?? "";
  const codecDir = explicitCodecDir || deriveCodecDir(modelDir);
  if (codecDir) {
    args.push("--codec-dir", codecDir);
  }
  if (settings.cpuThreads && settings.cpuThreads > 0) {
    args.push("--cpu-threads", String(settings.cpuThreads));
  }
  if (settings.inferenceSteps && settings.inferenceSteps > 0) {
    args.push("--num-steps", String(settings.inferenceSteps));
  }
  return args;
}

// Irodori preset layout: irodori-tts and semantic-dacvae live as sibling
// subdirectories under the app's models root. If the user points
// --model-dir at the Irodori folder, this yields the sibling codec
// folder automatically so they don't need to configure two paths.
export function deriveCodecDir(modelDir: string): string {
  if (!modelDir) return "";
  const sep = modelDir.includes("\\") && !modelDir.includes("/") ? "\\" : "/";
  const trimmed = modelDir.endsWith(sep) ? modelDir.slice(0, -1) : modelDir;
  const lastSep = trimmed.lastIndexOf(sep);
  if (lastSep <= 0) return "";
  return `${trimmed.slice(0, lastSep)}${sep}semantic-dacvae`;
}

export async function ensureSidecar(events: SidecarEvents = {}): Promise<void> {
  // Go through startSidecar so the whole isHealthy-then-spawn flow is tracked
  // by `starting`. Otherwise a stopSidecar that lands during the initial
  // `await isHealthy()` window sees `starting === undefined` and returns a
  // no-op — then the spawn proceeds unobserved and the caller (e.g. model
  // download) races against an alive sidecar mmapping the model files.
  return startSidecar(events);
}

export async function startSidecar(events: SidecarEvents = {}): Promise<void> {
  if (starting) return starting;
  starting = runStart(events).finally(() => {
    starting = undefined;
  });
  return starting;
}

async function runStart(events: SidecarEvents): Promise<void> {
  if (!isTauriRuntime()) {
    events.onLog?.("info", "ブラウザ実行中のため sidecar 自動起動はスキップしました。");
    return;
  }
  const existingHealth = await probeHealth(events);
  if (existingHealth) {
    events.onStatus?.(existingHealth.ok ? "running" : "loading");
    return;
  }

  events.onStatus?.("starting");
  const args = buildArgs();
  const command = Command.sidecar(sidecarProgram, args);
  let exitMessage = "";
  command.stdout.on("data", makeStreamLogger(events, "info"));
  command.stderr.on("data", makeStreamLogger(events, "error"));
  command.on("close", (data) => {
    child = undefined;
    exitMessage = `TTS sidecar が終了しました (code=${data.code ?? "null"}, signal=${data.signal ?? "null"})。`;
    events.onLog?.(stopping || data.code === 0 ? "info" : "error", exitMessage);
    stopping = false;
  });
  command.on("error", (error) => {
    exitMessage = `TTS sidecar の起動に失敗しました: ${error}`;
    events.onLog?.("error", exitMessage);
  });
  child = await command.spawn();
  events.onLog?.("info", `TTS sidecar を起動しました (${args.join(" ") || "既定設定"})。`);
  await waitForHealth(events, () => exitMessage);
}

export async function stopSidecar(events: SidecarEvents = {}): Promise<void> {
  // If a startSidecar is still in flight we must wait for it — otherwise the
  // spawn completes after our `if (!child) return` check, leaves a running
  // sidecar behind, and whoever called stopSidecar (e.g. the model-download
  // flow) proceeds against an alive sidecar still holding file mmaps.
  if (starting) {
    try {
      await starting;
    } catch {
      // startSidecar failed; nothing to kill, fall through.
    }
  }
  if (!child) return;
  try {
    stopping = true;
    await child.kill();
  } catch (error) {
    stopping = false;
    events.onLog?.("error", error instanceof Error ? error.message : String(error));
  }
  child = undefined;
  events.onStatus?.("stopped");
  events.onLog?.("info", "TTS sidecar を停止しました。");
}

export async function restartSidecar(events: SidecarEvents = {}): Promise<void> {
  await stopSidecar(events);
  await startSidecar(events);
}

async function waitForHealth(events: SidecarEvents, getExitMessage: () => string = () => ""): Promise<void> {
  const started = Date.now();
  while (Date.now() - started < SIDECAR_BOOT_TIMEOUT_MS) {
    const exitMessage = getExitMessage();
    if (exitMessage) {
      events.onStatus?.("failed");
      throw new Error(exitMessage);
    }
    const health = await probeHealth(events);
    if (health) {
      events.onStatus?.(health.ok ? "running" : "loading");
      return;
    }
    await delay(300);
  }
  events.onStatus?.("failed");
  throw new Error(`TTS sidecar の /health 確認が ${Math.round(SIDECAR_BOOT_TIMEOUT_MS / 1000)} 秒以内に完了しませんでした。`);
}

let lastLoggedEngine: string | undefined;
let lastLoggedDiagnostics: string | undefined;

async function probeHealth(events: SidecarEvents = {}): Promise<TtsHealth | undefined> {
  try {
    const health = await ttsClient.health();
    if (health.engine !== lastLoggedEngine) {
      lastLoggedEngine = health.engine;
      events.onLog?.("info", `TTSエンジン: ${health.engine_name ?? health.engine}`);
    }
    const diagnosticsKey = JSON.stringify(health.diagnostics ?? []);
    if (diagnosticsKey !== lastLoggedDiagnostics) {
      lastLoggedDiagnostics = diagnosticsKey;
      for (const diag of health.diagnostics ?? []) {
        const level = diag.severity === "error" ? "error" : "info";
        const suffix = diag.hint ? ` (${diag.hint})` : "";
        events.onLog?.(level, `${diag.message}${suffix}`);
      }
    }
    return health;
  } catch {
    return undefined;
  }
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => globalThis.setTimeout(resolve, ms));
}

function makeStreamLogger(
  events: SidecarEvents,
  level: "info" | "error"
): (chunk: string) => void {
  let buffer = "";
  return (chunk: string) => {
    buffer += String(chunk);
    const lines = buffer.split(/\r?\n/);
    buffer = lines.pop() ?? "";
    for (const line of lines) {
      const trimmed = line.trim();
      if (trimmed) events.onLog?.(level, trimmed);
    }
    const trimmedBuffer = buffer.trim();
    if (trimmedBuffer.length > 160) {
      events.onLog?.(level, trimmedBuffer);
      buffer = "";
    }
  };
}
