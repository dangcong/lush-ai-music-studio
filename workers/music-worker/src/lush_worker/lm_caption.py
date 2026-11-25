"""`acestep-lm`: the LM loaded inside ACE-Step describes audio (`/release_task` with `full_analysis_only`).

A built-in captioner (no script in CAPTIONER_DIR): registered while ACE-Step reports a loaded LM, it serves the
same `caption_tracks` and `describe_input` jobs as script captioners and returns the same result shapes
(docs/WORKER_PROTOCOL.md, "Captioner"). Spec: docs/superpowers/specs/2026-11-25-acestep-xl-only-design.md.
"""
from __future__ import annotations

import logging
from pathlib import Path
from typing import Any, Callable, Optional

from .captioning import _post, extract_fields
from .engines.base import EngineError, EngineUnavailable

log = logging.getLogger(__name__)

LM_CAPTIONER_KEY = "acestep-lm"
LM_CAPTIONER_NAME = "ACE-Step LM 4B"

# (ctx, audio file) -> ACE-Step analysis item: {prompt, bpm, keyscale, timesignature, lyrics, ...}
Analyze = Callable[[Any, Path], dict[str, Any]]


def lm_captioner_model(lm_name: str) -> dict[str, Any]:
    return {"engine": "captioner", "key": LM_CAPTIONER_KEY, "name": LM_CAPTIONER_NAME, "version": lm_name,
            "capabilities": ["caption"]}


def _fields(item: dict[str, Any]) -> Optional[dict[str, str]]:
    caption = str(item.get("prompt") or "").strip()
    return extract_fields(caption) if caption else None


def _number(value: Any) -> Optional[float]:
    try:
        return float(value) if value not in (None, "") else None
    except (TypeError, ValueError):
        return None


def run_lm_caption_job(ctx: Any, api: Any, analyze: Analyze, lm_name: str,
                       payload: dict[str, Any]) -> dict[str, Any]:
    """Caption every catalog track; a track the LM cannot describe is posted as an error and skipped.
    ACE-Step being down stops the job (retryable) instead of failing every remaining track."""
    tracks = payload.get("tracks") or []
    inputs = payload.get("inputs") or {}
    source_ref = f"{LM_CAPTIONER_KEY}@{lm_name}"
    done, failed = 0, []
    total = max(len(tracks), 1)
    for i, tr in enumerate(tracks):
        ctx.raise_if_aborted()
        track_id = str(tr["catalog_track_id"])
        audio = (inputs.get(tr.get("input")) or {}).get("path")
        fields: Optional[dict[str, str]] = None
        error: Optional[str] = None
        if not audio:
            error = "the track has no audio input"
        else:
            try:
                fields = _fields(analyze(ctx, Path(audio)))
                if fields is None:
                    error = "the LM returned an empty answer"
            except EngineUnavailable:
                raise
            except EngineError as e:
                log.warning("job %s: LM failed on %s: %s", ctx.job_id, track_id, e)
                error = str(e)
        if fields is not None:
            _post(api, ctx, track_id, fields, source_ref)
            done += 1
        else:
            _post(api, ctx, track_id, None, source_ref, error=error or "unknown error")
            failed.append(track_id)
        ctx.report((i + 1) / total, f"captioned {i + 1}/{len(tracks)}", stage="running", phase="caption")
    return {"done": done, "failed": failed}


def run_lm_describe_job(ctx: Any, analyze: Analyze, lm_name: str, payload: dict[str, Any]) -> dict[str, Any]:
    """`describe_input` with the LM: caption plus the LM's own tempo and key."""
    audio = ((payload.get("inputs") or {}).get("audio") or {}).get("path")
    if not audio:
        raise EngineError("describe_input job has no 'audio' input")
    ctx.report(0.1, f"listening with {LM_CAPTIONER_KEY}", stage="running", phase="caption")
    item = analyze(ctx, Path(audio))
    fields = _fields(item)
    if fields is None:
        raise EngineError("the LM returned an empty answer")
    key = str(item.get("keyscale") or "").strip() or None
    return {**fields, "source_ref": f"{LM_CAPTIONER_KEY}@{lm_name}", "tempo_bpm": _number(item.get("bpm")),
            "key": key}
