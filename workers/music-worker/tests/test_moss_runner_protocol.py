"""Protocol loop of captioners/moss_music/run.py, with load_model/generate monkeypatched (no torch)."""
import importlib.util
import io
import json
import sys
from pathlib import Path

import pytest

RUN_PY = Path(__file__).resolve().parents[1] / "captioners" / "moss_music" / "run.py"


@pytest.fixture()
def run_mod():
    spec = importlib.util.spec_from_file_location("moss_run", RUN_PY)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def drive(mod, monkeypatch, lines, generate):
    monkeypatch.setattr(mod, "load_model", lambda: "MODEL")
    monkeypatch.setattr(mod, "generate", generate)
    out = io.StringIO()
    code = mod.main(io.StringIO("".join(l + "\n" for l in lines)), out)
    return code, [json.loads(l) for l in out.getvalue().splitlines()]


def test_good_line(run_mod, monkeypatch):
    seen = []

    def gen(model, audio, prompt):
        seen.append((model, audio, prompt))
        return '{"caption": "x"}'

    code, out = drive(run_mod, monkeypatch,
                      [json.dumps({"id": "a", "audio": "/p.mp3", "prompt": None})], gen)
    assert code == 0
    assert out == [{"id": "a", "text": '{"caption": "x"}'}]
    assert seen == [("MODEL", "/p.mp3", run_mod.DEFAULT_PROMPT)]  # null prompt -> own default


def test_prompt_passed_through(run_mod, monkeypatch):
    seen = []
    drive(run_mod, monkeypatch, [json.dumps({"id": "a", "audio": "/p", "prompt": "hi"})],
          lambda m, a, p: seen.append(p) or "t")
    assert seen == ["hi"]


def test_generate_error_gives_error_line_and_continues(run_mod, monkeypatch):
    def gen(model, audio, prompt):
        if audio == "/bad":
            raise RuntimeError("boom")
        return "ok"

    lines = [json.dumps({"id": "1", "audio": "/bad", "prompt": None}),
             json.dumps({"id": "2", "audio": "/good", "prompt": None})]
    code, out = drive(run_mod, monkeypatch, lines, gen)
    assert code == 0
    assert out[0]["id"] == "1" and "boom" in out[0]["error"] and "text" not in out[0]
    assert out[1] == {"id": "2", "text": "ok"}


def test_malformed_line_gives_error_with_null_id(run_mod, monkeypatch):
    code, out = drive(run_mod, monkeypatch, ["not json", "[1]", json.dumps({"id": "z"}), ""],
                      lambda m, a, p: "ok")
    assert code == 0
    assert out[0]["id"] is None and out[0]["error"]
    assert out[1]["id"] is None and out[1]["error"]
    assert out[2]["id"] == "z" and out[2]["error"]  # best-effort id, missing audio
    assert len(out) == 3  # blank line ignored


def test_eof_exits_zero_without_output(run_mod, monkeypatch):
    code, out = drive(run_mod, monkeypatch, [], lambda m, a, p: "x")
    assert code == 0 and out == []


def test_model_load_failure_exits_nonzero(run_mod, monkeypatch):
    def boom():
        raise RuntimeError("no gpu")

    monkeypatch.setattr(run_mod, "load_model", boom)
    assert run_mod.main(io.StringIO(""), io.StringIO()) == 1
