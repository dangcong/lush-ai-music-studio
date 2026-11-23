"""Caption review page: a person listens, edits, approves or excludes. Local only (127.0.0.1).

Open it from another machine through an SSH tunnel: `ssh -L 8765:127.0.0.1:8765 <host>`.
Every decision is applied atomically per item (captions.set_fields writes the whole file at once)
and appended to `<version>/review/log.jsonl` for the pilot report.
"""
from __future__ import annotations

import copy
import json
import re
import threading
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Literal, Optional
from urllib.parse import unquote

from pydantic import BaseModel, Field, ValidationError, field_validator

from . import captions
from .config import Project
from .dataset import excluded_tracks, load_manifest, record_review
from .util import LushTrainingError, append_jsonl, atomic_write_json, now_iso, read_json

STATIC = Path(__file__).parent / "review_static"
SEGMENT_ID_RE = re.compile(r"^[A-Za-z0-9_-]{1,80}$")


class ReviewDecision(BaseModel):
    segment_id: str
    verdict: Literal["correct", "edited", "wrong", "exclude"]
    caption_text: Optional[str] = None
    metadata: dict[str, Any] = Field(default_factory=dict)
    hallucinations: list[Literal["instrument", "mood", "genre", "tempo", "key", "other"]] = Field(default_factory=list)
    has_voice: bool = False
    seconds_spent: float = Field(ge=0)
    reviewer: str = Field(min_length=1)

    @field_validator("metadata")
    @classmethod
    def _known_fields(cls, v: dict[str, Any]) -> dict[str, Any]:
        unknown = set(v) - set(captions.METADATA_FIELDS)
        if unknown:
            raise ValueError(f"trường metadata không hợp lệ: {sorted(unknown)}")
        return v


def _segment(vdir: Path, segment_id: str) -> tuple[dict[str, Any], dict[str, Any]]:
    manifest, _ = load_manifest(vdir)
    for seg in manifest["segments"]:
        if seg["segment_id"] == segment_id and seg["split"] in captions.TRAINING_SPLITS:
            track = next(t for t in manifest["tracks"] if t["track_id"] == seg["track_id"])
            return seg, track
    raise LushTrainingError(f"segment không có trong version này: {segment_id}")


# One writer at a time: every decision is read-modify-write on the caption file, the dataset's
# vocal_review.json and the log. The page server is threaded, so two tabs must not interleave.
_WRITE_LOCK = threading.Lock()


def _approval_errors(vdir: Path, seg: dict[str, Any], d: ReviewDecision, who: str) -> list[str]:
    """What `lock` would reject if this decision were saved as approved; checked before writing."""
    _, digest = load_manifest(vdir)
    cap = copy.deepcopy(read_json(captions.work_path(vdir, d.segment_id)))
    cap["metadata"].update(d.metadata or {})
    if d.caption_text is not None:
        cap["caption_text"] = d.caption_text
    cap["status"] = "approved"
    cap["human_edits"].append({"field": "status", "new": "approved", "by": who})
    cap["evidence"] += [{"field": f, "source": "human"} for f, v in (d.metadata or {}).items() if v is not None]
    errors, _ = captions.validate_caption(cap, seg, digest)
    return errors


def apply_decision(project: Project, dataset_name: str, vdir: Path, d: ReviewDecision) -> dict[str, Any]:
    with _WRITE_LOCK:
        seg, track = _segment(vdir, d.segment_id)
        who = d.reviewer.strip()
        if d.has_voice or d.verdict == "exclude":
            record_review(project, dataset_name, track["sha256"], "vocal" if d.has_voice else "excluded", by=who,
                          note="có giọng người" if d.has_voice else "loại khi duyệt caption")
            status = "needs_listening"
        elif d.verdict in ("correct", "edited"):
            if track["sha256"] in excluded_tracks(vdir):
                raise LushTrainingError("bài này đã bị loại khi duyệt; không duyệt được caption của nó. Nếu loại nhầm, "
                                        "ghi lại bằng `prepare_dataset.py review --decision instrumental`.")
            errors = _approval_errors(vdir, seg, d, who)
            if errors:
                raise LushTrainingError("không duyệt được: " + "; ".join(errors))
            status = "approved"
        else:
            status = "draft"
        cap = captions.set_fields(vdir, d.segment_id, by=who, human=True, metadata=d.metadata or None,
                                  caption_text=d.caption_text, status=status, note=f"review: {d.verdict}")
        if d.metadata:
            cap["evidence"] += [{"field": f, "source": "human", "by": who, "at": now_iso(), "value": v}
                                for f, v in d.metadata.items() if v is not None]
            atomic_write_json(captions.work_path(vdir, d.segment_id), cap)
        append_jsonl(Path(vdir) / "review" / "log.jsonl",
                     {**d.model_dump(), "track_id": seg["track_id"], "status": status, "at": now_iso()})
        return cap


def list_items(vdir: Path) -> list[dict[str, Any]]:
    manifest, _ = load_manifest(vdir)
    names = {t["track_id"]: Path(t["source_path"]).name for t in manifest["tracks"]}
    items = []
    for seg in manifest["segments"]:
        if seg["split"] not in captions.TRAINING_SPLITS:
            continue
        wp = captions.work_path(vdir, seg["segment_id"])
        cap = read_json(wp) if wp.is_file() else None
        ap = captions.analysis_path(vdir, seg["segment_id"])
        estimates = read_json(ap).get("estimates") if ap.is_file() else None
        items.append({"segment_id": seg["segment_id"], "track_id": seg["track_id"], "source_name": names.get(seg["track_id"]),
                      "split": seg["split"], "start": seg["start"], "end": seg["end"], "duration": seg["duration"],
                      "status": cap["status"] if cap else None, "caption_text": cap["caption_text"] if cap else None,
                      "metadata": cap["metadata"] if cap else None, "estimates": estimates,
                      "reasons": ((cap or {}).get("drafted_by") or {}).get("reasons") or []})
    return items


def make_server(project: Project, dataset_name: str, vdir: Path, host: str = "127.0.0.1",
                port: int = 8765) -> ThreadingHTTPServer:
    if host not in ("127.0.0.1", "localhost", "::1"):
        raise LushTrainingError("trang review chỉ nghe trên localhost; dùng SSH tunnel để mở từ máy khác")
    vdir = Path(vdir)

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, fmt: str, *args: Any) -> None:  # quiet
            pass

        def _send(self, code: int, body: bytes, ctype: str) -> None:
            self.send_response(code)
            self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(body)

        def _send_audio(self, f: Path) -> None:
            """Supports `Range: bytes=a-b` so the browser can seek inside a 4-minute segment."""
            data = f.read_bytes()
            size = len(data)
            m = re.match(r"^bytes=(\d*)-(\d*)$", self.headers.get("Range") or "")
            if not m or (not m.group(1) and not m.group(2)):
                self.send_response(200)
                start, end = 0, size - 1
            else:
                if m.group(1):
                    start = int(m.group(1))
                    end = min(int(m.group(2)) if m.group(2) else size - 1, size - 1)
                else:
                    start, end = max(0, size - int(m.group(2))), size - 1
                if start > end or start >= size:
                    self.send_response(416)
                    self.send_header("Content-Range", f"bytes */{size}")
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                self.send_response(206)
                self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
            body = data[start:end + 1]
            self.send_header("Content-Type", "audio/wav")
            self.send_header("Accept-Ranges", "bytes")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def _json(self, code: int, obj: Any) -> None:
            self._send(code, json.dumps(obj, ensure_ascii=False).encode("utf-8"), "application/json; charset=utf-8")

        def _audio_file(self, raw: str) -> Optional[Path]:
            sid = unquote(raw)
            if not SEGMENT_ID_RE.match(sid):
                return None
            try:
                seg, _ = _segment(vdir, sid)
            except LushTrainingError:
                return None
            return vdir / seg["file"]

        def do_GET(self) -> None:  # noqa: N802
            path = self.path.split("?", 1)[0]
            if path in ("/", "/index.html"):
                self._send(200, (STATIC / "index.html").read_bytes(), "text/html; charset=utf-8")
            elif path == "/api/items":
                self._json(200, list_items(vdir))
            elif path.startswith("/audio/"):
                f = self._audio_file(path[len("/audio/"):])
                if f is None or not f.is_file():
                    self._json(404, {"error": "not found"})
                else:
                    self._send_audio(f)
            else:
                self._json(404, {"error": "not found"})

        def do_POST(self) -> None:  # noqa: N802
            path = self.path.split("?", 1)[0]
            if not path.startswith("/api/items/"):
                return self._json(404, {"error": "not found"})
            sid = unquote(path[len("/api/items/"):])
            try:
                raw = json.loads(self.rfile.read(int(self.headers.get("Content-Length") or 0)) or b"{}")
                d = ReviewDecision(**{**raw, "segment_id": sid})
            except (ValueError, ValidationError, TypeError) as e:
                return self._json(422, {"error": str(e)})
            try:
                cap = apply_decision(project, dataset_name, vdir, d)
            except LushTrainingError as e:
                return self._json(404 if "không có trong version" in str(e) else 409, {"error": str(e)})
            except Exception as e:  # noqa: BLE001 - answer instead of dropping the connection
                return self._json(500, {"error": f"{type(e).__name__}: {e}"})
            self._json(200, {"status": cap["status"], "segment_id": sid})

    return ThreadingHTTPServer((host, port), Handler)


def serve(project: Project, dataset_name: str, vdir: Path, host: str = "127.0.0.1", port: int = 8765) -> None:
    srv = make_server(project, dataset_name, vdir, host, port)
    print(f"Trang review: http://{host}:{srv.server_address[1]}/  (Ctrl+C để dừng)")
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        srv.server_close()
