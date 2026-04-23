"""
Koehon Studio — Python TTS sidecar (Irodori-TTS RF-DiT + DACVAE).

Speaks the same loopback-only HTTP API the previous Rust/ORT sidecar did
so the Tauri frontend's transport stays unchanged:

    GET  /health       -> HealthResponse
    POST /synthesize   -> SynthesizeResponse    body: SynthesizeRequest

/synthesize writes a PCM16 WAV to `output_path` and returns metadata, same
contract as before. The frontend doesn't need to know we swapped engines.
"""

from __future__ import annotations

import argparse
import logging
import os
import sys
import threading
import time
from pathlib import Path
from typing import Any

print("koehon python sidecar bootstrap", flush=True)

from fastapi import FastAPI, HTTPException
from fastapi.responses import JSONResponse
from pydantic import BaseModel, Field
from starlette.middleware.cors import CORSMiddleware

logger = logging.getLogger("koehon.sidecar")


# ---------- protocol models (mirrors native-tts/src/main.rs) --------------


class VoiceInfo(BaseModel):
    id: str
    name: str


class EngineDiagnostic(BaseModel):
    severity: str  # info | warning | error
    code: str
    message: str
    hint: str | None = None


class HealthResponse(BaseModel):
    ok: bool
    engine: str
    engine_name: str
    sample_rate: int
    voices: list[VoiceInfo]
    diagnostics: list[EngineDiagnostic]


class SynthesizeRequest(BaseModel):
    request_id: str
    text: str
    voice: str | None = None
    seed: int | None = None
    output_path: str = Field(..., description="absolute path to write the PCM16 WAV")


class SynthesizeResponse(BaseModel):
    ok: bool
    request_id: str
    audio_path: str
    sample_rate: int
    elapsed_seconds: float


# ---------- engine wrapper ------------------------------------------------


class IrodoriEngine:
    """
    Thin adapter around `irodori_tts.inference_runtime.InferenceRuntime`.

    Heavy imports (torch, irodori) are deferred to `load()` so the sidecar
    can come up and answer /health immediately — the frontend depends on
    that to drive the first-run "model setup needed" UI before weights
    are on disk.
    """

    ENGINE_ID = "irodori-tts-500m-v2"
    ENGINE_NAME = "Irodori-TTS 500M v2 (RF-DiT, Japanese)"

    def __init__(
        self,
        *,
        checkpoint: Path,
        codec_repo: str,
        num_steps: int,
        cpu_threads: int,
        device: str,
        precision: str,
        max_ref_seconds: float,
    ) -> None:
        self._checkpoint = checkpoint
        self._codec_repo = codec_repo
        self._num_steps = num_steps
        self._cpu_threads = cpu_threads
        self._device = device
        self._precision = precision
        self._max_ref_seconds = max_ref_seconds

        self._runtime: Any = None
        self._sampling_cls: Any = None

        self._diagnostics: list[EngineDiagnostic] = []
        self._load_error: str | None = None
        self._sample_rate: int = 48_000  # DACVAE Japanese 32dim reconstructs 48kHz
        self._ready = False
        self._lock = threading.Lock()

    # -- lifecycle ---------------------------------------------------

    def load(self) -> None:
        with self._lock:
            if self._ready:
                return
            try:
                self._load_inner()
                self._ready = True
                self._load_error = None
            except Exception as exc:  # noqa: BLE001
                self._load_error = str(exc)
                logger.exception("engine load failed")
                self._diagnostics = [
                    EngineDiagnostic(
                        severity="error",
                        code="engine.load_failed",
                        message=f"Irodori-TTS のロードに失敗しました: {exc}",
                        hint=(
                            "モデルディレクトリに model.safetensors、"
                            "コーデックとして Aratako/Semantic-DACVAE-Japanese-32dim が配置されているか確認してください。"
                        ),
                    )
                ]

    def _load_inner(self) -> None:
        if not self._checkpoint.is_file():
            raise FileNotFoundError(
                f"checkpoint not found: {self._checkpoint}. "
                "モデルディレクトリに model.safetensors を配置してください。"
            )

        if self._cpu_threads > 0:
            try:
                import torch
                torch.set_num_threads(self._cpu_threads)
            except Exception:  # noqa: BLE001
                logger.exception("failed to configure torch cpu threads")

        from irodori_tts.inference_runtime import (  # type: ignore
            InferenceRuntime,
            RuntimeKey,
            SamplingRequest,
        )

        codec_location = _resolve_codec_location(self._codec_repo)
        key = RuntimeKey(
            checkpoint=str(self._checkpoint),
            model_device=self._device,
            codec_repo=codec_location,
            model_precision=self._precision,
            codec_device=self._device,
            codec_precision=self._precision,
            enable_watermark=False,
            compile_model=False,
            compile_dynamic=False,
        )

        logger.info("loading Irodori runtime: %s", self._checkpoint)
        started = time.perf_counter()
        runtime = InferenceRuntime.from_key(key)
        logger.info("Irodori runtime ready in %.1fs", time.perf_counter() - started)

        self._runtime = runtime
        self._sampling_cls = SamplingRequest

        # Sample rate comes from the codec. A 0-step dry-run of synthesize
        # would be expensive; the Japanese DACVAE the checkpoint loads is
        # documented to output 48 kHz so we keep that as the advertised
        # rate. If the checkpoint ever ships a different codec the first
        # /synthesize response will still carry the authoritative rate.
        self._sample_rate = 48_000

        self._diagnostics = [
            EngineDiagnostic(
                severity="info",
                code="engine.ready",
                message=(
                    f"Irodori-TTS 読込済 · checkpoint={self._checkpoint.name} · "
                    f"codec={codec_location} · steps={self._num_steps} · "
                    f"device={self._device}/{self._precision}"
                ),
            )
        ]

    # -- /health -----------------------------------------------------

    def health(self) -> HealthResponse:
        if not self._ready and self._load_error is None:
            diagnostics = [
                EngineDiagnostic(
                    severity="info",
                    code="engine.loading",
                    message="Irodori-TTS を初期化中です…",
                )
            ]
            return HealthResponse(
                ok=False,
                engine=self.ENGINE_ID,
                engine_name=self.ENGINE_NAME,
                sample_rate=self._sample_rate,
                voices=[],
                diagnostics=diagnostics,
            )
        voices: list[VoiceInfo] = [
            VoiceInfo(id="default", name="Irodori 既定話者"),
        ]
        return HealthResponse(
            ok=self._ready,
            engine=self.ENGINE_ID,
            engine_name=self.ENGINE_NAME,
            sample_rate=self._sample_rate,
            voices=voices,
            diagnostics=list(self._diagnostics),
        )

    # -- /synthesize -------------------------------------------------

    def synthesize(self, req: SynthesizeRequest) -> SynthesizeResponse:
        if not self._ready:
            # Try once more in case the user just finished placing model files.
            self.load()
            if not self._ready:
                raise HTTPException(
                    status_code=503,
                    detail={
                        "ok": False,
                        "error": self._load_error or "Irodori-TTS がまだ読み込まれていません。",
                        "code": "engine.not_ready",
                    },
                )

        started = time.perf_counter()
        with self._lock:
            samples, sample_rate = self._run_inference(req)
        elapsed = time.perf_counter() - started

        output_path = Path(req.output_path)
        output_path.parent.mkdir(parents=True, exist_ok=True)
        # Match the old Rust sidecar: mono PCM16 WAV at the codec's native rate.
        import soundfile as sf
        sf.write(str(output_path), samples, sample_rate, subtype="PCM_16")

        return SynthesizeResponse(
            ok=True,
            request_id=req.request_id,
            audio_path=str(output_path),
            sample_rate=sample_rate,
            elapsed_seconds=float(elapsed),
        )

    def _run_inference(self, req: SynthesizeRequest) -> tuple[np.ndarray, int]:
        import numpy as np
        import torch

        # Let Irodori pick its own seed if none supplied — its sampler uses
        # logit-normal noise schedules that benefit from fresh randomness
        # across requests.
        seed = int(req.seed) if req.seed is not None else int(torch.seed() % (2**31 - 1))

        sampling_req = self._sampling_cls(
            text=req.text,
            caption=None,
            ref_wav=None,
            ref_latent=None,
            no_ref=True,  # standard TTS (no voice cloning)
            num_candidates=1,
            num_steps=self._num_steps,
            seed=seed,
            trim_tail=True,
        )
        result = self._runtime.synthesize(sampling_req)

        # SamplingResult.audio is a torch.Tensor; shape can be one of
        #   (channels, samples)         — single candidate
        #   (num_candidates, channels, samples)
        # We only asked for num_candidates=1 so collapse to mono.
        audio = result.audio
        if hasattr(audio, "detach"):
            audio = audio.detach().to("cpu").to(torch.float32).numpy()

        arr = np.asarray(audio, dtype=np.float32)
        if arr.ndim == 3:
            arr = arr[0]
        if arr.ndim == 2 and arr.shape[0] > 1:
            # Downmix multichannel to mono (preserve loudness roughly).
            arr = arr.mean(axis=0)
        elif arr.ndim == 2:
            arr = arr[0]

        arr = np.clip(arr, -1.0, 1.0)
        sample_rate = int(getattr(result, "sample_rate", self._sample_rate))
        return arr, sample_rate


# ---------- FastAPI app ---------------------------------------------------


def build_app(engine: IrodoriEngine) -> FastAPI:
    app = FastAPI(title="koehon-tts-sidecar", version="0.2.0-irodori")
    app.add_middleware(
        CORSMiddleware,
        allow_origins=["*"],
        allow_methods=["GET", "POST", "OPTIONS"],
        allow_headers=["*"],
    )

    @app.get("/health", response_model=HealthResponse)
    def health() -> HealthResponse:
        return engine.health()

    @app.post("/synthesize", response_model=SynthesizeResponse)
    def synthesize(req: SynthesizeRequest) -> SynthesizeResponse:
        return engine.synthesize(req)

    @app.exception_handler(Exception)
    def on_error(_request: Any, exc: Exception) -> JSONResponse:
        logger.exception("unhandled: %s", exc)
        return JSONResponse(
            status_code=500,
            content={"ok": False, "error": str(exc), "code": "internal"},
        )

    return app


# ---------- entrypoint ----------------------------------------------------


def _default_model_dir() -> Path:
    env_dir = os.environ.get("KOEHON_MODEL_DIR")
    if env_dir:
        return Path(env_dir)
    if sys.platform.startswith("win"):
        base = Path(os.environ.get("APPDATA", Path.home() / "AppData/Roaming"))
    elif sys.platform == "darwin":
        base = Path.home() / "Library/Application Support"
    else:
        base = Path.home() / ".local/share"
    return base / "studio.koehon.app" / "models" / "irodori-tts"


def _resolve_codec_location(value: str) -> str:
    location = str(value).strip()
    if location.startswith("hf://"):
        return location
    path = Path(location)
    if path.is_dir():
        weights = path / "weights.pth"
        if weights.is_file():
            return str(weights)
    return location


def _parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    p = argparse.ArgumentParser(description="Koehon Studio TTS sidecar (Irodori)")
    p.add_argument(
        "--model-dir",
        type=Path,
        default=None,
        help="Directory containing model.safetensors (Irodori checkpoint).",
    )
    p.add_argument(
        "--codec-dir",
        type=str,
        default="Aratako/Semantic-DACVAE-Japanese-32dim",
        help=(
            "DACVAE codec repo id or local directory. When pointing to a "
            "local path it must contain the DACVAE weights the Irodori "
            "runtime expects."
        ),
    )
    p.add_argument("--host", default="127.0.0.1")
    p.add_argument("--port", type=int, default=18083)
    p.add_argument(
        "--cpu-threads",
        type=int,
        default=0,
        help="torch.set_num_threads value (0 = library default).",
    )
    p.add_argument(
        "--num-steps",
        type=int,
        default=32,
        help="Flow-matching sampling steps. Lower = faster, higher = better.",
    )
    p.add_argument("--device", default="cpu", choices=["cpu", "cuda", "mps"])
    p.add_argument("--precision", default="fp32", choices=["fp32", "bf16"])
    p.add_argument("--max-ref-seconds", type=float, default=30.0)
    p.add_argument("--log-level", default="info")
    return p.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = _parse_args(argv)

    logging.basicConfig(
        level=getattr(logging, args.log_level.upper(), logging.INFO),
        format="[%(asctime)s] %(levelname)s %(name)s: %(message)s",
    )

    # Enforce loopback-only — mirrors the Rust sidecar's safety gate.
    if args.host not in {"127.0.0.1", "::1", "localhost"}:
        logger.error("refusing non-loopback bind host=%s", args.host)
        return 2

    model_dir = args.model_dir or _default_model_dir()
    checkpoint = model_dir / "model.safetensors"

    if args.cpu_threads > 0:
        os.environ.setdefault("OMP_NUM_THREADS", str(args.cpu_threads))
        os.environ.setdefault("MKL_NUM_THREADS", str(args.cpu_threads))

    engine = IrodoriEngine(
        checkpoint=checkpoint,
        codec_repo=args.codec_dir,
        num_steps=args.num_steps,
        cpu_threads=args.cpu_threads,
        device=args.device,
        precision=args.precision,
        max_ref_seconds=args.max_ref_seconds,
    )

    app = build_app(engine)

    import uvicorn
    print(
        f"koehon tts sidecar listening on http://{args.host}:{args.port} "
        f"engine={IrodoriEngine.ENGINE_ID}",
        flush=True,
    )

    # Load on a worker thread so /health comes online immediately — the
    # weights take 10-30s on CPU and the frontend polls /health to drive
    # the first-run UI. Keep heavyweight imports out of the main thread
    # before uvicorn starts listening.
    threading.Thread(target=engine.load, name="engine-load", daemon=True).start()

    uvicorn.run(app, host=args.host, port=args.port, log_level=args.log_level)
    return 0


if __name__ == "__main__":
    sys.exit(main())
