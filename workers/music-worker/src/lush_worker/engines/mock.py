"""MockEngine: produces a real, audible WAV without any GPU (dev machines, UI/pipeline testing).

Deterministic: the same seed + duration + bpm + key + time signature always yields the same bytes.
"""
from __future__ import annotations

import random
import time
from typing import Any, Callable

from .base import CAP_GENERATE, Engine, EngineError, GenerateResult, JobContext
from .music_synth import SAMPLE_RATE, synthesize, write_wav

MODEL_KEY = "mock-v1"
STEPS = 8
MAX_DURATION = 600.0
FAKE_KEYS = ["C major", "A minor", "G major", "E minor", "D major", "F major", "D minor"]


class MockEngine(Engine):
    name = "mock"
    version = "1"
    capabilities = (CAP_GENERATE,)

    def __init__(self, step_delay: float = 0.3, sleep: Callable[[float], None] = time.sleep):
        self.step_delay = step_delay
        self._sleep = sleep

    def list_models(self) -> list[dict[str, Any]]:
        return [{"engine": self.name, "key": MODEL_KEY, "name": "Mock Engine v1",
                 "capabilities": list(self.capabilities)}]

    def generate(self, ctx: JobContext, payload: dict[str, Any]) -> GenerateResult:
        try:
            seed = int(payload["seed"])
            duration = float(payload["duration"])
        except (KeyError, TypeError, ValueError) as e:
            raise EngineError(f"invalid generate payload (seed/duration): {e!r}") from e
        if not 0 < duration <= MAX_DURATION:
            raise EngineError(f"duration must be in (0, {MAX_DURATION:.0f}] seconds, got {duration}")

        # Missing musical parameters are derived from the seed so the result stays reproducible.
        rng = random.Random(seed)
        fallback_bpm, fallback_key = rng.randint(70, 120), rng.choice(FAKE_KEYS)
        try:
            bpm = int(payload.get("bpm") or fallback_bpm)
        except (TypeError, ValueError) as e:
            raise EngineError(f"invalid bpm: {payload.get('bpm')!r}") from e
        if not 30 <= bpm <= 300:
            raise EngineError(f"bpm must be between 30 and 300, got {bpm}")
        key = str(payload.get("key") or fallback_key)
        time_signature = str(payload.get("time_signature") or "4/4")

        ctx.progress(0.02, f"[mock] seed={seed} duration={duration:g}s bpm={bpm} key={key} ts={time_signature}")
        if payload.get("lora"):
            ctx.progress(0.03, f"[mock] LoRA '{payload['lora'].get('key')}' requested: ignored by the mock engine")
        for i in range(STEPS):
            ctx.raise_if_aborted()
            self._sleep(self.step_delay)
            ctx.progress(0.05 + 0.8 * (i + 1) / STEPS, f"[mock] diffusion {i + 1}/{STEPS}")
        ctx.raise_if_aborted()

        out = ctx.workdir / "master.wav"
        samples = synthesize(duration, bpm, key, time_signature, seed)
        write_wav(out, samples)
        ctx.progress(1.0, "[mock] audio synthesized")
        return GenerateResult(
            audio_path=out, seed=seed, duration=len(samples) / SAMPLE_RATE, sample_rate=SAMPLE_RATE,
            engine_info={"bpm": bpm, "key": key, "time_signature": time_signature, "steps": STEPS},
        )
