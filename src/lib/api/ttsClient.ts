export type TtsHealth = {
  ok: boolean;
  engine: string;
  engine_name?: string;
  sample_rate?: number;
  voices: Array<{ id: string; name: string }>;
  diagnostics?: Array<{
    severity: "info" | "warning" | "error";
    code: string;
    message: string;
    hint?: string | null;
  }>;
};

export type SynthesizeRequest = {
  requestId: string;
  text: string;
  voice?: string;
  seed?: number;
  outputPath: string;
};

export type SynthesizeResult = {
  ok: boolean;
  requestId: string;
  audioPath: string;
  sampleRate: number;
  elapsedSeconds: number;
};

type SidecarErrorResponse = {
  ok: false;
  error?: string;
};

type RawSynthesizeResult = {
  ok: boolean;
  request_id?: string;
  requestId?: string;
  audio_path?: string;
  audioPath?: string;
  sample_rate?: number;
  sampleRate?: number;
  elapsed_seconds?: number;
  elapsedSeconds?: number;
};

export class TtsClientError extends Error {
  constructor(
    message: string,
    public readonly cause?: unknown
  ) {
    super(message);
  }
}

export class TtsHttpClient {
  constructor(
    private readonly baseUrl = "http://127.0.0.1:18083",
    private readonly timeoutMs = 30000
  ) {}

  async health(): Promise<TtsHealth> {
    return this.request<TtsHealth>("/health", { method: "GET" });
  }

  async synthesize(req: SynthesizeRequest): Promise<SynthesizeResult> {
    const raw = await this.request<RawSynthesizeResult>("/synthesize", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        request_id: req.requestId,
        text: req.text,
        voice: req.voice,
        seed: req.seed,
        output_path: req.outputPath
      })
    });
    return {
      ok: raw.ok,
      requestId: raw.requestId ?? raw.request_id ?? req.requestId,
      audioPath: raw.audioPath ?? raw.audio_path ?? req.outputPath,
      sampleRate: raw.sampleRate ?? raw.sample_rate ?? 0,
      elapsedSeconds: raw.elapsedSeconds ?? raw.elapsed_seconds ?? 0
    };
  }

  private async request<T>(path: string, init: RequestInit): Promise<T> {
    const controller = new AbortController();
    const timeout = globalThis.setTimeout(() => controller.abort(), this.timeoutMs);
    try {
      const response = await fetch(`${this.baseUrl}${path}`, { ...init, signal: controller.signal });
      if (!response.ok) {
        let detail = "";
        try {
          const error = (await response.json()) as SidecarErrorResponse;
          detail = error.error ? `: ${error.error}` : "";
        } catch {
          detail = "";
        }
        throw new TtsClientError(`TTS sidecar returned ${response.status}${detail}`);
      }
      return (await response.json()) as T;
    } catch (error) {
      if (error instanceof TtsClientError) throw error;
      if (error instanceof DOMException && error.name === "AbortError") {
        throw new TtsClientError(`TTS sidecar の応答が ${Math.round(this.timeoutMs / 1000)} 秒以内に返りませんでした。`, error);
      }
      throw new TtsClientError("TTS sidecar に接続できません。ネイティブ sidecar の起動状態を確認してください。", error);
    } finally {
      globalThis.clearTimeout(timeout);
    }
  }
}

export const ttsClient = new TtsHttpClient();
