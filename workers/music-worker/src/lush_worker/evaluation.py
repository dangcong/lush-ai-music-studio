"""Job `eval_lora`: listening renders for each checkpoint of a LoRA, mp3 uploads and scores.

Order (docs/WORKER_PROTOCOL.md, "Job eval_lora"): check every input on disk, render (base without
LoRA, then per checkpoint the test prompt and each held-out prompt), score, restart the inference
server when at least one adapter was loaded (R11), return the result the Control Plane expects.
Scores are for reference only; the memorization thresholds in `scoring` are not calibrated yet.
"""
from __future__ import annotations

import copy
import logging
import math
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable, Optional

from .audio import ffmpeg_path, make_mp3_preview, sha256_file
from .engines.base import STAGE_RUNNING, EngineError
from .lora_files import TARGET_KEY_RE, checkpoint_dir
from .scoring import ScorerError, best_match, chroma, cosine, melody_similarity, style_score

JOB_TYPE = "eval_lora"
PHASE_RENDERING = "rendering"
PHASE_SCORING = "scoring"
RENDER_SHARE = 0.9  # rendering 0..0.9 of the job progress, scoring the rest
MAX_RENDERS = 100  # artifact kinds eval_00..eval_99
DATASET_FILE_RE = re.compile(r"^s[0-9]{3}\.wav$")
LORA_CONFIG_FILE = "adapter_config.json"
SCORE_KEYS = ("mert_style", "mert_train", "train_match", "melody", "melody_match", "quality")

log = logging.getLogger(__name__)


@dataclass
class _Render:
    index: int
    kind: str  # base | prompt | held_out
    checkpoint_id: Optional[str]
    sample_filename: Optional[str]
    prompt: str
    lora: Optional[dict[str, Any]]
    audio: Optional[Path] = None
    embedding: Optional[list[float]] = None
    scores: dict[str, Any] = field(default_factory=lambda: {k: None for k in SCORE_KEYS})

    @property
    def artifact_kind(self) -> str:
        return f"eval_{self.index:02d}"


class _RenderContext:
    """JobContext for one render: its own workdir; engine progress maps into this render's slice."""

    def __init__(self, parent: Any, workdir: Path, index: int, total: int):
        self._parent = parent
        self.workdir = workdir
        self._index = index
        self._total = total

    job_id = property(lambda self: self._parent.job_id)
    lease_token = property(lambda self: self._parent.lease_token)
    cancel_requested = property(lambda self: self._parent.cancel_requested)
    lease_lost = property(lambda self: self._parent.lease_lost)
    shutdown_requested = property(lambda self: self._parent.shutdown_requested)

    def raise_if_aborted(self) -> None:
        self._parent.raise_if_aborted()

    def progress(self, value: float, message: Optional[str] = None, stage: str = STAGE_RUNNING) -> None:
        share = (self._index + max(0.0, min(1.0, value))) / self._total * RENDER_SHARE
        self._parent.report(share, message, STAGE_RUNNING, phase=PHASE_RENDERING)


def _finite(value: Any) -> Optional[float]:
    try:
        v = float(value)
    except (TypeError, ValueError):
        return None
    return v if math.isfinite(v) else None


def _finite_embedding(value: Any) -> Optional[list[float]]:
    if not isinstance(value, list) or not value:
        return None
    out = [_finite(x) for x in value]
    return None if any(x is None for x in out) else out  # type: ignore[return-value]


# ---------- plan / checks ----------
def _plan(payload: dict[str, Any], dataset_dir: Path, checkpoint_root: Optional[Path]
          ) -> tuple[list[_Render], list[tuple[str, Path]]]:
    """Validate the payload and every file it names; nothing is rendered before this passes."""
    dataset_key = str(payload.get("dataset_key") or "")
    if not TARGET_KEY_RE.match(dataset_key):
        raise EngineError(f"dataset_key không hợp lệ: {dataset_key!r}")
    test_prompt = payload.get("test_prompt")
    if not isinstance(test_prompt, str) or not test_prompt.strip():
        raise EngineError("payload eval_lora thiếu test_prompt")
    if not isinstance(payload.get("render_template"), dict):
        raise EngineError("payload eval_lora thiếu render_template")
    try:
        scale = min(1.0, max(0.0, float(payload.get("scale", 0.7))))
    except (TypeError, ValueError) as e:
        raise EngineError(f"scale không hợp lệ: {payload.get('scale')!r}") from e

    train: list[tuple[str, Path]] = []
    audio_dir = dataset_dir / dataset_key / "audio"
    for item in payload.get("train") or []:
        name = str((item or {}).get("dataset_file") or "")
        filename = str((item or {}).get("filename") or name)
        if not DATASET_FILE_RE.match(name):
            raise EngineError(f"dataset_file không hợp lệ: {name!r}")
        path = audio_dir / name
        if not path.is_file():
            raise EngineError(f"Bài train {filename} ({dataset_key}/{name}) không có trên máy này")
        train.append((filename, path))

    held_out = payload.get("held_out") or []
    for h in held_out:
        if not isinstance(h, dict) or not isinstance(h.get("prompt"), str) or not h["prompt"].strip():
            raise EngineError("held_out thiếu prompt")

    checkpoints = payload.get("checkpoints")
    if not isinstance(checkpoints, list) or not checkpoints:
        raise EngineError("payload eval_lora không có bản lưu nào")
    if checkpoint_root is None:
        raise EngineError("node này không có thư mục train")
    renders = [_Render(0, "base", None, None, test_prompt, None)]
    for ck in checkpoints:
        ck_id, key = str((ck or {}).get("id") or ""), str((ck or {}).get("key") or "")
        if not ck_id or not TARGET_KEY_RE.match(key):
            raise EngineError(f"bản lưu không hợp lệ: {ck!r}")
        path = checkpoint_dir(ck.get("path"), checkpoint_root)  # EngineError when outside the root
        if not (path / LORA_CONFIG_FILE).is_file():
            raise EngineError(f"Bản lưu epoch {ck.get('epoch')} không có trên máy này")
        lora = {"key": key, "scale": scale, "path": str(ck.get("path"))}
        renders.append(_Render(len(renders), "prompt", ck_id, None, test_prompt, lora))
        for h in held_out:
            renders.append(_Render(len(renders), "held_out", ck_id, str(h.get("filename") or ""), h["prompt"], lora))
    if len(renders) > MAX_RENDERS:
        raise EngineError(f"quá nhiều render ({len(renders)} > {MAX_RENDERS})")
    return renders, train


# ---------- scoring ----------
def _score_all(ctx: Any, renders: list[_Render], train: list[tuple[str, Path]],
               scorers: dict[str, Any]) -> tuple[dict[str, bool], list[list[float]]]:
    """Fill `r.scores` / `r.embedding`; returns (scorers that ran, MERT training embeddings)."""
    names = [n for n, _ in train]
    total = len(renders)

    def report(done: float, message: Optional[str] = None) -> None:
        ctx.report(RENDER_SHARE + (1 - RENDER_SHARE) * 0.98 * done, message, STAGE_RUNNING, phase=PHASE_SCORING)

    # Melody (librosa): always attempted. Each training track's chroma is computed once per job.
    report(0.0, "Chấm giai điệu")
    train_chroma: list[tuple[str, Any]] = []
    for filename, path in train:
        try:
            train_chroma.append((filename, chroma(path)))
        except Exception as e:
            log.warning("eval: no chroma for training track %s: %s", filename, e)
    for i, r in enumerate(renders):
        ctx.raise_if_aborted()
        if not train_chroma or r.audio is None:
            continue
        try:
            c = chroma(r.audio)
            best: Optional[tuple[float, str]] = None
            for filename, tc in train_chroma:
                v = _finite(melody_similarity(c, tc))
                if v is not None and (best is None or v > best[0]):
                    best = (v, filename)
            if best is not None:
                r.scores["melody"], r.scores["melody_match"] = best
        except Exception as e:
            log.warning("eval: melody score failed for %s: %s", r.artifact_kind, e)
        report(0.4 * (i + 1) / total)

    used = {"mert": False, "audiobox": False}

    # MERT embeddings: train tracks once, then every render. Any scorer error: MERT unavailable.
    mert = scorers.get("mert")
    train_embs: list[list[float]] = []
    if mert is not None and train:
        report(0.4, "Chấm style (MERT)")
        try:
            mert.start()
            for _, path in train:
                ctx.raise_if_aborted()
                emb = _finite_embedding(mert.score(path))
                if emb is None:
                    raise ScorerError("non-finite embedding")
                train_embs.append(emb)
            for i, r in enumerate(renders):
                ctx.raise_if_aborted()
                if r.audio is not None:
                    r.embedding = _finite_embedding(mert.score(r.audio))
                report(0.4 + 0.4 * (i + 1) / total)
            for r in renders:
                if r.embedding is None:
                    continue
                r.scores["mert_style"] = _finite(style_score([r.embedding], train_embs))
                value, match = best_match(r.embedding, train_embs, names)
                value = _finite(value)
                if value is not None:
                    r.scores["mert_train"], r.scores["train_match"] = value, match
            used["mert"] = True
        except (ScorerError, OSError, ValueError) as e:
            log.warning("eval: MERT scorer unavailable for this job: %s", e)
            for r in renders:
                r.embedding = None
                r.scores["mert_style"] = r.scores["mert_train"] = r.scores["train_match"] = None
        finally:
            _close(mert)
    mert_train_embs = train_embs if used["mert"] else []

    # Audiobox quality per render.
    quality = scorers.get("audiobox")
    if quality is not None:
        report(0.8, "Chấm chất lượng (Audiobox)")
        try:
            quality.start()
            for i, r in enumerate(renders):
                ctx.raise_if_aborted()
                if r.audio is not None:
                    r.scores["quality"] = _finite(quality.score(r.audio))
                report(0.8 + 0.2 * (i + 1) / total)
            used["audiobox"] = True
        except (ScorerError, OSError, ValueError) as e:
            log.warning("eval: Audiobox scorer unavailable for this job: %s", e)
            for r in renders:
                r.scores["quality"] = None
        finally:
            _close(quality)
    return used, mert_train_embs


def _close(scorer: Any) -> None:
    try:
        scorer.close()
    except Exception as e:
        log.warning("eval: could not close a scorer: %s", e)


def _checkpoint_scores(ck_id: str, renders: list[_Render], train_embs: list[list[float]]) -> dict[str, Any]:
    mine = [r for r in renders if r.checkpoint_id == ck_id]
    out: dict[str, Any] = {"id": ck_id, "style": None, "train": None, "train_match": None,
                           "melody": None, "melody_match": None, "quality": None}
    embs = [r.embedding for r in mine if r.embedding is not None]
    if embs and train_embs:
        out["style"] = _finite(style_score(embs, train_embs))
    for score, match, key, key_match in (("mert_train", "train_match", "train", "train_match"),
                                         ("melody", "melody_match", "melody", "melody_match")):
        best = max((r for r in mine if r.scores[score] is not None), key=lambda r: r.scores[score], default=None)
        if best is not None:
            out[key], out[key_match] = best.scores[score], best.scores[match]
    qualities = [r.scores["quality"] for r in mine if r.scores["quality"] is not None]
    if qualities:
        out["quality"] = _finite(sum(qualities) / len(qualities))
    return out


# ---------- job ----------
def run_eval(ctx: Any, payload: dict[str, Any], *, engine: Any, api: Any, dataset_dir: Path,
             scorers: dict[str, Any], service: Any, checkpoint_root: Optional[Path] = None,
             upload: Optional[Callable[[str, Path], None]] = None,
             to_mp3: Callable[[Path, Path], Optional[Path]] = make_mp3_preview) -> dict[str, Any]:
    """Render, upload and score; returns the `complete` result for `eval_lora`.

    `scorers` maps "mert" (embedding) and/or "audiobox" (quality) to scorer clients that are not
    started yet (`start` / `score` / `close`). `upload(kind, mp3_path)` defaults to a single
    `api.upload_artifact` call; the runner passes one with retries."""
    if checkpoint_root is None:
        checkpoint_root = getattr(engine, "checkpoint_root", None)
    renders, train = _plan(payload, dataset_dir, checkpoint_root)
    if to_mp3 is make_mp3_preview and not ffmpeg_path():
        raise EngineError("Không có ffmpeg trên máy này: không tạo được MP3 nghe thử")
    if upload is None:
        def upload(kind: str, path: Path) -> None:
            api.upload_artifact(ctx.job_id, ctx.lease_token, kind, path, sha256_file(path), "audio/mpeg")

    template = payload["render_template"]
    total = len(renders)
    adapters_loaded = False
    try:
        for r in renders:
            ctx.raise_if_aborted()
            ctx.report(r.index / total * RENDER_SHARE, f"Tạo nhạc nghe thử {r.index + 1}/{total}",
                       STAGE_RUNNING, phase=PHASE_RENDERING)
            body = copy.deepcopy(template)
            body["prompt"] = r.prompt
            body["seed"] = payload.get("seed", template.get("seed"))
            body["duration"] = payload.get("duration", template.get("duration"))
            if r.lora is None:
                body.pop("lora", None)
            else:
                body["lora"] = dict(r.lora)
                adapters_loaded = True  # set before the call: a failed load may still leave state behind
            workdir = Path(ctx.workdir) / f"render_{r.index:02d}"
            workdir.mkdir(parents=True, exist_ok=True)
            result = engine.generate(_RenderContext(ctx, workdir, r.index, total), body)
            ctx.raise_if_aborted()
            mp3 = to_mp3(result.audio_path, workdir / f"{r.artifact_kind}.mp3")
            if mp3 is None:
                raise EngineError("Không tạo được MP3 nghe thử (ffmpeg lỗi)")
            upload(r.artifact_kind, mp3)
            r.audio = result.audio_path
        ctx.raise_if_aborted()
        used, train_embs = _score_all(ctx, renders, train, scorers)
    finally:
        if adapters_loaded:
            _restart(ctx, service)

    checkpoint_ids = list(dict.fromkeys(r.checkpoint_id for r in renders if r.checkpoint_id))
    return {
        "renders": [{"artifact_kind": r.artifact_kind, "kind": r.kind, "checkpoint_id": r.checkpoint_id,
                     "sample_filename": r.sample_filename, "prompt": r.prompt, "scores": dict(r.scores)}
                    for r in renders],
        "checkpoints": [_checkpoint_scores(c, renders, train_embs) for c in checkpoint_ids],
        "scorers": used,
    }


def _restart(ctx: Any, service: Any) -> None:
    """R11: drop the evaluation adapters from VRAM. Never raises (it runs in a finally)."""
    unit = getattr(service, "unit", "inference")
    log.info("job %s: restarting %s to drop the evaluation adapters", getattr(ctx, "job_id", "?"), unit)
    try:
        service.stop()
    except Exception as e:
        log.error("eval: could not stop %s: %s", unit, e)
    try:
        if not service.start():
            log.error("eval: %s did not become ready after the evaluation", unit)
    except Exception as e:
        log.error("eval: could not start %s: %s", unit, e)
