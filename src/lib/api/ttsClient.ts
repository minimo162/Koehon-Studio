export type TtsHealth = {
  ok: boolean;
  engine: string;
  voices: Array<{ id: string; name: string }>;
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

export class TtsClientError extends Error {
  constructor(
    message: string,
    public readonly cause?: unknown
  ) {
    super(message);
  }
}

export class TtsHttpClient {
  constructor(private readonly baseUrl = "http://127.0.0.1:18083") {}

  async health(): Promise<TtsHealth> {
    return this.request<TtsHealth>("/health", { method: "GET" });
  }

  async synthesize(req: SynthesizeRequest): Promise<SynthesizeResult> {
    return this.request<SynthesizeResult>("/synthesize", {
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
  }

  private async request<T>(path: string, init: RequestInit): Promise<T> {
    const controller = new AbortController();
    const timeout = window.setTimeout(() => controller.abort(), 10000);
    try {
      const response = await fetch(`${this.baseUrl}${path}`, { ...init, signal: controller.signal });
      if (!response.ok) throw new TtsClientError(`TTS sidecar returned ${response.status}`);
      return (await response.json()) as T;
    } catch (error) {
      if (error instanceof TtsClientError) throw error;
      throw new TtsClientError("TTS sidecar に接続できません。ネイティブ sidecar の起動状態を確認してください。", error);
    } finally {
      window.clearTimeout(timeout);
    }
  }
}

export const ttsClient = new TtsHttpClient();
