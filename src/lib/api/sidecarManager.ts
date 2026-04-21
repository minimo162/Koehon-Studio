import { Command, type Child } from "@tauri-apps/plugin-shell";
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
  const command = Command.sidecar(sidecarProgram);
  command.stdout.on("data", (line) => events.onLog?.("info", String(line).trim()));
  command.stderr.on("data", (line) => events.onLog?.("error", String(line).trim()));
  child = await command.spawn();
  events.onLog?.("info", "TTS sidecar を起動しました。");
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
