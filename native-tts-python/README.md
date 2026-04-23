# Koehon Studio — Python TTS sidecar (Irodori)

Replaces the old Rust/ORT sidecar. Serves the same loopback-only HTTP API
(`GET /health`, `POST /synthesize`) but runs Irodori-TTS-500M-v2 (RF-DiT +
Semantic-DACVAE-Japanese-32dim) via PyTorch CPU.

## Layout

```
native-tts-python/
├── server.py           FastAPI server + IrodoriEngine adapter
├── requirements.txt    Pinned inference-only deps (CPU wheels)
├── README.md           (this file)
```

The Windows installer ships a bundle containing an embedded Python
interpreter plus the installed packages. In dev, run with a local venv:

```bash
uv venv
uv pip install --index-strategy unsafe-best-match \
    --extra-index-url https://download.pytorch.org/whl/cpu \
    -r requirements.txt
uv pip install \
    git+https://github.com/Aratako/Irodori-TTS.git \
    git+https://github.com/facebookresearch/dacvae.git
uv run python server.py \
    --model-dir "$HOME/.local/share/studio.koehon.app/models/irodori-tts" \
    --codec-dir Aratako/Semantic-DACVAE-Japanese-32dim \
    --cpu-threads 4 \
    --num-steps 32
```

## Protocol

Same as the previous Rust sidecar:

```http
GET /health
-> { ok, engine, engine_name, sample_rate, voices[], diagnostics[] }

POST /synthesize
{
  "request_id": "chapter-001-chunk-000",
  "text": "...",
  "voice": null,
  "seed": null,
  "output_path": "C:\\...\\chunk.wav"
}
-> { ok, request_id, audio_path, sample_rate, elapsed_seconds }
```

The response doesn't carry audio bytes — the server writes a PCM16 WAV to
`output_path` on disk, matching the old contract.

## Cold-start behaviour

`/health` answers immediately; model weights load on a background thread.
While loading, `health.ok == false` and diagnostics carry
`engine.loading`. Once loaded, `engine.ready`. If load fails,
`engine.load_failed` with a hint pointing at the model directory.

The frontend already polls `/health` to drive its setup/re-setup UI, so
no frontend change is needed to handle the slow cold start (10-30 s CPU).
