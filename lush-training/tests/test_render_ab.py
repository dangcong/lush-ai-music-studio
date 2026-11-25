"""A/B rendering against acestep-api: same seed/steps both sides, base = LoRA disabled, each adapter loaded once."""
import io
import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse

import numpy as np
import pytest
import soundfile as sf

from lush_training.render_ab import AceStepHttp, render_side
from lush_training.util import LushTrainingError, read_json


def wav_bytes(seconds=1.0, rate=16000):
    buf = io.BytesIO()
    sf.write(buf, np.zeros((int(seconds * rate), 2), dtype=np.float32), rate, format="WAV")
    return buf.getvalue()


class FakeAceStep:
    def __init__(self, fail=False):
        self.calls = []
        self.adapters = []
        self.use_lora = False
        self.fail = fail
        self.tasks = {}

    def handle(self, method, path, body):
        self.calls.append((method, path, body))
        wrap = lambda data: {"data": data, "code": 200, "error": None}  # noqa: E731
        if path == "/v1/lora/status":
            return wrap({"lora_loaded": bool(self.adapters), "use_lora": self.use_lora, "adapters": list(self.adapters)})
        if path == "/v1/lora/load":
            self.adapters.append(body["adapter_name"])
            return wrap({})
        if path == "/v1/lora/toggle":
            self.use_lora = body["use_lora"]
            return wrap({})
        if path == "/v1/lora/scale":
            return wrap({})
        if path == "/release_task":
            tid = f"t{len(self.tasks) + 1}"
            self.tasks[tid] = body
            return wrap({"task_id": tid})
        if path == "/query_result":
            tid = body["task_id_list"][0]
            if self.fail:
                return wrap([{"task_id": tid, "status": 2, "result": json.dumps([{"error": "boom"}])}])
            return wrap([{"task_id": tid, "status": 1, "result": json.dumps([{"file": f"/v1/audio?path={tid}.wav"}])}])
        return None


@pytest.fixture
def fake_server():
    def start(fake):
        class H(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def _do(self, method):
                path = urlparse(self.path).path
                n = int(self.headers.get("Content-Length") or 0)
                body = json.loads(self.rfile.read(n)) if n else None
                if path == "/v1/audio":
                    fake.calls.append((method, path, None))
                    data = wav_bytes()
                    self.send_response(200)
                    self.send_header("Content-Length", str(len(data)))
                    self.end_headers()
                    self.wfile.write(data)
                    return
                resp = fake.handle(method, path, body)
                data = json.dumps(resp).encode()
                self.send_response(200 if resp is not None else 404)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def do_GET(self):
                self._do("GET")

            def do_POST(self):
                self._do("POST")

        srv = ThreadingHTTPServer(("127.0.0.1", 0), H)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        servers.append(srv)
        return AceStepHttp(f"http://127.0.0.1:{srv.server_address[1]}", poll_seconds=0.01)

    servers = []
    yield start
    for s in servers:
        s.shutdown()
        s.server_close()


PLAN = {"items": [{"id": "lofi__s11", "prompt": "lo-fi instrumental, warm keys", "seed": 11, "duration": 30.0,
                   "lyrics": "[Instrumental]", "file": "lofi__s11.wav"},
                  {"id": "lofi__s22", "prompt": "lo-fi instrumental, warm keys", "seed": 22, "duration": 30.0,
                   "lyrics": "[Instrumental]", "file": "lofi__s22.wav"}]}


def releases(fake):
    return [b for m, p, b in fake.calls if p == "/release_task"]


def test_render_base_toggles_off(fake_server, tmp_path):
    fake = FakeAceStep()
    fake.adapters, fake.use_lora = ["old"], True
    client = fake_server(fake)
    files = render_side(client, PLAN, tmp_path, side="parent", lora_dir=None, model="acestep-v15-xl-sft")
    assert [f.name for f in files] == ["lofi__s11.wav", "lofi__s22.wav"] and all(f.is_file() for f in files)
    assert ("POST", "/v1/lora/toggle", {"use_lora": False}) in fake.calls
    assert fake.use_lora is False
    settings = read_json(tmp_path / "render_settings.json")
    assert settings["parent"]["lora_dir"] is None and settings["parent"]["inference_steps"] == 50


def test_render_lora_loads_once_per_checkpoint(fake_server, tmp_path):
    fake = FakeAceStep()
    client = fake_server(fake)
    ck1, ck2 = tmp_path / "epoch_1", tmp_path / "epoch_2"
    for ck in (ck1, ck2):
        ck.mkdir()
        (ck / "adapter_config.json").write_text("{}")
    render_side(client, PLAN, tmp_path / "e1", side="candidate", lora_dir=ck1, model="acestep-v15-xl-sft")
    render_side(client, PLAN, tmp_path / "e1b", side="candidate", lora_dir=ck1, model="acestep-v15-xl-sft")
    render_side(client, PLAN, tmp_path / "e2", side="candidate", lora_dir=ck2, model="acestep-v15-xl-sft")
    loads = [b for m, p, b in fake.calls if p == "/v1/lora/load"]
    assert len(loads) == 2 and loads[0]["lora_path"].endswith("epoch_1") and loads[1]["lora_path"].endswith("epoch_2")
    assert fake.use_lora is True


def test_same_seed_and_steps_both_sides(fake_server, tmp_path):
    fake = FakeAceStep()
    client = fake_server(fake)
    ck = tmp_path / "ck"
    ck.mkdir()
    (ck / "adapter_config.json").write_text("{}")
    render_side(client, PLAN, tmp_path / "ev", side="parent", lora_dir=None, model="acestep-v15-xl-sft")
    render_side(client, PLAN, tmp_path / "ev", side="candidate", lora_dir=ck, model="acestep-v15-xl-sft")
    bodies = releases(fake)
    base, cand = bodies[:2], bodies[2:]
    keys = ("prompt", "seed", "inference_steps", "guidance_scale", "audio_duration", "lyrics", "use_random_seed")
    for a, b in zip(base, cand):
        assert {k: a[k] for k in keys} == {k: b[k] for k in keys}
    assert base[0]["seed"] == 11 and base[0]["use_random_seed"] is False and base[0]["lyrics"] == "[Instrumental]"


def test_failed_task_raises(fake_server, tmp_path):
    client = fake_server(FakeAceStep(fail=True))
    with pytest.raises(LushTrainingError, match="boom"):
        render_side(client, PLAN, tmp_path, side="parent", lora_dir=None, model="acestep-v15-xl-sft")


def test_lora_dir_without_adapter_config_rejected(fake_server, tmp_path):
    client = fake_server(FakeAceStep())
    with pytest.raises(LushTrainingError, match="adapter_config.json"):
        render_side(client, PLAN, tmp_path, side="candidate", lora_dir=tmp_path / "missing", model="acestep-v15-xl-sft")


def test_render_all_base_once_then_each_checkpoint(fake_server, tmp_path):
    from lush_training.render_ab import render_all
    prompts = tmp_path / "prompts.yaml"
    prompts.write_text('version: "t1"\nseeds: [11, 22]\nprompts:\n'
                       '  - {id: lofi, in_style: true, duration: 30, prompt: "lo-fi instrumental"}\n', encoding="utf-8")
    cks = []
    for n in (1, 2):
        ck = tmp_path / f"epoch_{n}"
        ck.mkdir()
        (ck / "adapter_config.json").write_text("{}")
        cks.append(ck)
    fake = FakeAceStep()
    client = fake_server(fake)
    dirs = render_all(client, prompts, tmp_path / "eval", cks, model="acestep-v15-xl-sft", run_id="run1")
    assert [d.name for d in dirs] == ["epoch_1", "epoch_2"]
    assert len(releases(fake)) == 2 + 2 * 2            # base once (2 items) + 2 checkpoints x 2 items
    for d in dirs:
        assert (d / "plan.json").is_file()
        assert sorted(p.name for p in (d / "parent").iterdir()) == ["lofi__s11.wav", "lofi__s22.wav"]
        assert sorted(p.name for p in (d / "candidate").iterdir()) == ["lofi__s11.wav", "lofi__s22.wav"]
