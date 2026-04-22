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
  return args;
}

// MOSS preset layout: moss-tts-nano and moss-audio-tokenizer are siblings.
export function deriveCodecDir(modelDir: string): string {
  if (!modelDir) return "";
  const sep = modelDir.includes("\\") && !modelDir.includes("/") ? "\\" : "/";
  const trimmed = modelDir.endsWith(sep) ? modelDir.slice(0, -1) : modelDir;
  const lastSep = trimmed.lastIndexOf(sep);
  if (lastSep <= 0) return "";
  return `${trimmed.slice(0, lastSep)}${sep}moss-audio-tokenizer`;
}

export async function ensureSidecar(events: SidecarEvents = {}): Promise<void> {
  if (await isHealthy()) {
    events.onStatus?.("running");
    return;
  }
  if (starting) return starting;
  starting = startSidecar(events).finally(() => {
    starting = undefined;
  });
  return starting;
}

export async function startSidecar(events: SidecarEvents = {}): Promise<void> {
  if (!isTauriRuntime()) {
    events.onLog?.("info", "ブラウザ実行中のため sidecar 自動起動はスキップしました。");
    return;
  }
  if (child && (await isHealthy())) {
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
  if (!child) return;
  await child.kill();
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
    if (await isHealthy()) {
      events.onStatus?.("running");
      return;
    }
    await delay(300);
  }
  events.onStatus?.("failed");
  throw new Error("TTS sidecar の /health 確認がタイムアウトしました。");
}

async function isHealthy(): Promise<boolean> {
  try {
    const health = await ttsClient.health();
    return health.ok;
  } catch {
    return false;
  }
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => globalThis.setTimeout(resolve, ms));
}
