"""Render the evaluation plan against a running `acestep-api` (ACE-Step 1.5 ca1e85f HTTP API).

Base side: LoRA layers disabled (`/v1/lora/toggle false`), which gives exact base-model output.
Candidate side: the checkpoint adapter, loaded ONCE under its own name, then enabled and selected.
`/v1/lora/unload` is never used: at ca1e85f it does not remove the layers. After a session, restart
`acestep-api` to drop test adapters. Both sides use the same prompt, seed, steps, guidance, duration.

Behaviour mirrors workers/music-worker/src/lush_worker/engines/acestep.py; this workspace does not
import from workers/ (D28), so it carries its own small stdlib client.
"""
from __future__ import annotations

import hashlib
import json
import re
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any, Optional

from .util import LushTrainingError, atomic_write_json, now_iso, read_json

DEFAULT_STEPS = 50  # ACE-Step 1.5 XL SFT (D36 in the Studio repo)
DEFAULT_GUIDANCE = 7.0
STATUS_DONE, STATUS_FAILED = 1, 2


class AceStepHttp:
    def __init__(self, base_url: str, api_key: str = "", timeout: float = 60.0, poll_seconds: float = 2.0,
                 task_timeout: float = 1800.0):
        self.base_url = base_url.rstrip("/")
        self.api_key = api_key
        self.timeout = timeout
        self.poll_seconds = poll_seconds
        self.task_timeout = task_timeout

    def _open(self, method: str, path: str, body: Optional[dict[str, Any]] = None, timeout: Optional[float] = None):
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(self.base_url + path, data=data, method=method)
        if data is not None:
            req.add_header("Content-Type", "application/json")
        if self.api_key:
            req.add_header("Authorization", f"Bearer {self.api_key}")
        try:
            return urllib.request.urlopen(req, timeout=timeout or self.timeout)
        except urllib.error.HTTPError as e:
            raise LushTrainingError(f"ACE-Step HTTP {e.code} cho {path}: {e.read()[:300]!r}") from None
        except urllib.error.URLError as e:
            raise LushTrainingError(f"không kết nối được ACE-Step tại {self.base_url}: {e.reason}") from None

    def call(self, method: str, path: str, body: Optional[dict[str, Any]] = None, timeout: Optional[float] = None) -> Any:
        with self._open(method, path, body, timeout) as r:
            payload = json.loads(r.read() or b"null")
        if not isinstance(payload, dict) or payload.get("error") or payload.get("code", 200) != 200:
            raise LushTrainingError(f"ACE-Step trả lỗi cho {path}: {payload}")
        return payload.get("data")

    def download(self, file_ref: str, dest: Path) -> Path:
        path = file_ref if file_ref.startswith("/v1/audio") else "/v1/audio?" + urllib.parse.urlencode({"path": file_ref})
        dest.parent.mkdir(parents=True, exist_ok=True)
        with self._open("GET", path, timeout=300) as r, open(dest, "wb") as f:
            while chunk := r.read(1 << 16):
                f.write(chunk)
        return dest

    def run_task(self, body: dict[str, Any]) -> dict[str, Any]:
        task_id = str((self.call("POST", "/release_task", body) or {}).get("task_id") or "")
        if not task_id:
            raise LushTrainingError("release_task không trả task_id")
        started = time.monotonic()
        while True:
            rows = self.call("POST", "/query_result", {"task_id_list": [task_id]}) or []
            row = next((r for r in rows if str(r.get("task_id")) == task_id), rows[0] if rows else {})
            raw = row.get("result")
            items = json.loads(raw) if isinstance(raw, str) and raw else (raw or [])
            items = [items] if isinstance(items, dict) else items
            status = int(row.get("status", 0))
            if status == STATUS_DONE:
                if not items or not items[0].get("file"):
                    raise LushTrainingError(f"task {task_id} xong nhưng không có file audio")
                return items[0]
            if status == STATUS_FAILED:
                err = (items[0].get("error") if items else None) or row.get("progress_text") or raw
                raise LushTrainingError(f"ACE-Step task {task_id} lỗi: {str(err)[-1000:]}")
            if time.monotonic() - started > self.task_timeout:
                raise LushTrainingError(f"ACE-Step task {task_id} quá {self.task_timeout:.0f} s")
            time.sleep(self.poll_seconds)


def adapter_name(lora_dir: Path) -> str:
    key = str(Path(lora_dir).resolve())
    safe = re.sub(r"[^A-Za-z0-9_]", "_", Path(lora_dir).name)[:40]
    return f"lush_eval_{safe}_{hashlib.sha1(key.encode()).hexdigest()[:8]}"


def _set_lora(client: AceStepHttp, lora_dir: Optional[Path]) -> Optional[str]:
    status = client.call("GET", "/v1/lora/status") or {}
    adapters = [a if isinstance(a, str) else (a or {}).get("name") for a in status.get("adapters") or []]
    if lora_dir is None:
        if status.get("lora_loaded") and status.get("use_lora"):
            client.call("POST", "/v1/lora/toggle", {"use_lora": False})
        return None
    if not (Path(lora_dir) / "adapter_config.json").is_file():
        raise LushTrainingError(f"{lora_dir} không có adapter_config.json (không phải adapter PEFT)")
    name = adapter_name(lora_dir)
    if name not in adapters:
        client.call("POST", "/v1/lora/load", {"lora_path": str(Path(lora_dir).resolve()), "adapter_name": name}, timeout=300)
    client.call("POST", "/v1/lora/toggle", {"use_lora": True})
    client.call("POST", "/v1/lora/scale", {"scale": 1.0, "adapter_name": name})
    return name


def release_body(item: dict[str, Any], *, model: str, steps: int, guidance: float) -> dict[str, Any]:
    return {"prompt": item["prompt"], "lyrics": item.get("lyrics") or "[Instrumental]", "vocal_language": "unknown",
            "task_type": "text2music", "model": model, "thinking": False, "use_cot_caption": False,
            "use_cot_language": False, "use_random_seed": False, "seed": int(item["seed"]), "batch_size": 1,
            "inference_steps": steps, "audio_format": "wav", "guidance_scale": guidance,
            "audio_duration": float(item["duration"])}


def render_side(client: AceStepHttp, plan: dict[str, Any], out_dir: Path, *, side: str, lora_dir: Optional[Path],
                model: str, steps: int = DEFAULT_STEPS, guidance: float = DEFAULT_GUIDANCE) -> list[Path]:
    out_dir = Path(out_dir)
    adapter = _set_lora(client, lora_dir)
    files = []
    for item in plan["items"]:
        result = client.run_task(release_body(item, model=model, steps=steps, guidance=guidance))
        files.append(client.download(str(result["file"]), out_dir / side / item["file"]))
    settings_path = out_dir / "render_settings.json"
    settings = read_json(settings_path) if settings_path.is_file() else {}
    settings[side] = {"rendered_at": now_iso(), "model": model, "inference_steps": steps, "guidance_scale": guidance,
                      "lora_dir": str(lora_dir) if lora_dir else None, "adapter_name": adapter,
                      "api": client.base_url, "items": len(files)}
    atomic_write_json(settings_path, settings)
    return files


def render_all(client: AceStepHttp, prompts_file: Path, eval_root: Path, checkpoints: list[Path], *, model: str,
               run_id: str, steps: int = DEFAULT_STEPS, guidance: float = DEFAULT_GUIDANCE) -> list[Path]:
    """Render the base model once, then every checkpoint; one evaluate-compatible folder per checkpoint
    (`plan.json`, `parent/` = base copy, `candidate/` = checkpoint) ready for make_ab / write_report."""
    import shutil

    from .evaluate import make_plan
    eval_root = Path(eval_root)
    base_plan = make_plan(prompts_file, eval_root / "_base", run_id=run_id, checkpoint_name="base")
    render_side(client, base_plan, eval_root / "_base", side="parent", lora_dir=None, model=model, steps=steps,
                guidance=guidance)
    out = []
    for ck in checkpoints:
        d = eval_root / Path(ck).name
        if d.exists():
            raise LushTrainingError(f"{d} đã tồn tại; dùng eval_root mới")
        plan = make_plan(prompts_file, d, run_id=run_id, checkpoint_name=Path(ck).name)
        for item in plan["items"]:
            shutil.copy2(eval_root / "_base" / "parent" / item["file"], d / "parent" / item["file"])
        base_settings = read_json(eval_root / "_base" / "render_settings.json")
        atomic_write_json(d / "render_settings.json", base_settings)
        render_side(client, plan, d, side="candidate", lora_dir=Path(ck), model=model, steps=steps, guidance=guidance)
        out.append(d)
    return out
