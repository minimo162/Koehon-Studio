import { Command, type Child } from "@tauri-apps/plugin-shell";
import { get } from "svelte/store";
import { appSettingsStore } from "../stores/appSettings";
import { isTauriRuntime } from "./fileAccess";
import { ttsClient } from "./ttsClient";

export type SidecarStatus = "idle" | "starting" | "running" | "failed" | "stopped";

type SidecarEvents = {
  onStatus?: (status: SidecarStatus) => void;
  onLog?: (level: "info" | "error", message: string) => void;
};

let child: Child | undefined;
let starting: Promise<void> | undefined;
const sidecarProgram = "../native-tts/sidecars/koehon-tts-sidecar";

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
  if (await isHealthy()) {
    events.onStatus?.("running");
    return;
  }

  events.onStatus?.("starting");
  const args = buildArgs();
  const command = Command.sidecar(sidecarProgram, args);
  command.stdout.on("data", (line) => events.onLog?.("info", String(line).trim()));
  command.stderr.on("data", (line) => events.onLog?.("error", String(line).trim()));
  child = await command.spawn();
  events.onLog?.("info", `TTS sidecar を起動しました (${args.join(" ") || "既定設定"})。`);
  await waitForHealth(events);
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
    await child.kill();
  } catch (error) {
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

async function waitForHealth(events: SidecarEvents): Promise<void> {
  const started = Date.now();
  while (Date.now() - started < 8000) {
    if (await isHealthy(events)) {
      events.onStatus?.("running");
      return;
    }
    await delay(300);
  }
  events.onStatus?.("failed");
  throw new Error("TTS sidecar の /health 確認がタイムアウトしました。");
}

let lastLoggedEngine: string | undefined;

async function isHealthy(events: SidecarEvents = {}): Promise<boolean> {
  try {
    const health = await ttsClient.health();
    if (health.ok && health.engine !== lastLoggedEngine) {
      lastLoggedEngine = health.engine;
      events.onLog?.("info", `TTSエンジン: ${health.engine_name ?? health.engine}`);
      for (const diag of health.diagnostics ?? []) {
        const level = diag.severity === "error" ? "error" : "info";
        const suffix = diag.hint ? ` (${diag.hint})` : "";
        events.onLog?.(level, `${diag.message}${suffix}`);
      }
    }
    return health.ok;
  } catch {
    return false;
  }
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => globalThis.setTimeout(resolve, ms));
}
