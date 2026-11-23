"""Caption drafts from a captioner model (or a folder label). Drafts only: a human approves.

Every draft records which captioner produced it. Text that mentions a voice (vocals, singer,
vocal-like, humming, spoken word…) is set to `needs_listening` so a person checks the track;
text over the word limit is kept whole and also sent to `needs_listening`, never silently cut.
"""
from __future__ import annotations

from pathlib import Path
from typing import Any, Protocol

from pydantic import BaseModel

from . import captions
from .dataset import load_manifest
from .util import LushTrainingError, atomic_write_json, now_iso, read_json


class Captioner(Protocol):
    name: str
    revision: str

    def describe(self, audio: Path) -> str: ...


class CaptionDraft(BaseModel):
    caption_en: str
    voice_terms: list[str]
    words: int
    source: str

    @classmethod
    def from_text(cls, text: str, *, source: str) -> "CaptionDraft":
        clean = " ".join(text.split())
        stripped = captions.NEGATED_VOCAL.sub(" ", clean)
        terms = sorted({m.group(0).lower() for m in captions.VOCAL_TERMS.finditer(stripped)})
        return cls(caption_en=clean, voice_terms=terms, words=len(clean.split()), source=source)

    @property
    def too_long(self) -> bool:
        return self.words > captions.MAX_CAPTION_WORDS


class FolderLabelCaptioner:
    """Fallback when no captioner model is approved: the same template text for every sample."""
    name = "folder-label"
    revision = "1"

    def __init__(self, template: str):
        if not template.strip():
            raise LushTrainingError("folder_template rỗng")
        self.template = template.strip()

    def describe(self, audio: Path) -> str:
        return self.template


class AceStepCaptioner:
    """`ACE-Step/acestep-captioner` via transformers. The loading code is written from the model card
    once the user approves the download (plan Task 7); until then it refuses to run."""
    name = "acestep-captioner"

    def __init__(self, model_dir: Path, revision: str):
        self.model_dir = Path(model_dir)
        self.revision = revision

    def describe(self, audio: Path) -> str:
        raise LushTrainingError("AceStepCaptioner chưa được nối: cần tải model đã duyệt và đọc model card (Task 7). "
                                "Dùng FolderLabelCaptioner trong lúc chờ.")


def draft_all(vdir: Path, captioner: Captioner, *, by: str) -> dict[str, int]:
    manifest, _ = load_manifest(vdir)
    source = f"{captioner.name}@{captioner.revision}"
    counts = {"drafted": 0, "skipped_approved": 0, "needs_listening": 0}
    for seg in manifest["segments"]:
        if seg["split"] not in captions.TRAINING_SPLITS:
            continue
        path = captions.work_path(vdir, seg["segment_id"])
        if not path.is_file():
            raise LushTrainingError(f"chưa có caption nháp cho {seg['segment_id']} (chạy `validate_captions.py init`)")
        if read_json(path)["status"] == "approved":
            counts["skipped_approved"] += 1
            continue
        draft = CaptionDraft.from_text(captioner.describe(Path(vdir) / seg["file"]), source=source)
        reasons = []
        if draft.voice_terms:
            reasons.append("caption nhắc tới giọng người: " + ", ".join(draft.voice_terms))
        if draft.too_long:
            reasons.append(f"caption dài {draft.words} từ (> {captions.MAX_CAPTION_WORDS})")
        status = "needs_listening" if reasons else "draft"
        captions.set_fields(vdir, seg["segment_id"], by=by, human=False, caption_text=draft.caption_en, status=status,
                            note="; ".join(reasons))
        cap: dict[str, Any] = read_json(path)
        # Provenance of the text lives outside `evidence`, which only holds metadata-field evidence.
        cap["drafted_by"] = {"tool": source, "at": now_iso(), "voice_terms": draft.voice_terms,
                             "words": draft.words, "reasons": reasons}
        atomic_write_json(path, cap)
        counts["drafted"] += 1
        if reasons:
            counts["needs_listening"] += 1
    return counts
