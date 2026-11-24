"""Protocol loop of the captioner scripts (captioners/*/run.py), with load_model/generate monkeypatched (no torch)."""
import importlib.util
import io
import json
import sys
from pathlib import Path

import pytest

CAPTIONERS = Path(__file__).resolve().parents[1] / "captioners"
# script dir -> env var that makes load_model/generate fake (and noisy on stdout like real libraries)
RUNNERS = {"moss_music": "MOSS_FAKE_GENERATE", "music_flamingo": "FLAMINGO_FAKE_GENERATE",
           "acestep_captioner": "ACESTEP_CAPTIONER_FAKE_GENERATE", "qwen3_omni_captioner": "QWEN3_CAPTIONER_FAKE_GENERATE"}


@pytest.fixture(params=sorted(RUNNERS))
def runner(request):
    return request.param


@pytest.fixture()
def run_mod(runner):
    spec = importlib.util.spec_from_file_location(f"{runner}_run", CAPTIONERS / runner / "run.py")
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


def test_real_subprocess_stdout_only_has_protocol_lines(runner):
    import os
    import subprocess

    reqs = "".join(json.dumps({"id": i, "audio": f"/x/{i}.mp3", "prompt": None}) + "\n" for i in ("a", "b"))
    r = subprocess.run([sys.executable, str(CAPTIONERS / runner / "run.py")], input=reqs.encode(), capture_output=True, timeout=60,
                       env={**os.environ, RUNNERS[runner]: "1"})
    assert r.returncode == 0, r.stderr
    lines = r.stdout.decode("utf-8").splitlines()
    assert [json.loads(l)["id"] for l in lines] == ["a", "b"]
    assert all("text" in json.loads(l) for l in lines)
    assert b"junk" in r.stderr  # noise was diverted to stderr


def _load(name):
    spec = importlib.util.spec_from_file_location(f"{name}_mw", CAPTIONERS / name / "run.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


R = 16000


def test_qwen3_long_track_gives_intro_middle_outro_30s_each():
    sw = _load("qwen3_omni_captioner").section_windows
    assert sw(R * 180, R, 30) == [("Intro", 0, R * 30), ("Middle", R * 75, R * 105), ("Outro", R * 150, R * 180)]


def test_qwen3_short_track_is_split_in_equal_thirds_without_overlap():
    sw = _load("qwen3_omni_captioner").section_windows
    assert sw(R * 60, R, 30) == [("Intro", 0, R * 20), ("Middle", R * 20, R * 40), ("Outro", R * 40, R * 60)]


def test_qwen3_track_under_one_window_is_heard_once_whole():
    sw = _load("qwen3_omni_captioner").section_windows
    assert sw(R * 25, R, 30) == [("Whole", 0, R * 25)]


def test_qwen3_clip_text_cuts_at_last_sentence_end_within_limit():
    clip = _load("qwen3_omni_captioner").clip_text
    assert clip("One two. Three four. Five six seven.", 22) == "One two. Three four."
    assert clip("Short.", 100) == "Short."
    assert clip("nosentenceendhere at all", 10) == "nosentence"  # no sentence end -> hard cut


def test_qwen3_join_sections_labels_each_part():
    join = _load("qwen3_omni_captioner").join_sections
    assert join([("Intro", "A."), ("Middle", "B."), ("Outro", "C.")]) == "Intro: A.\nMiddle: B.\nOutro: C."
    assert join([("Whole", "A.")]) == "A."


def test_qwen3_trim_silence_drops_leading_and_trailing_quiet():
    import numpy as np
    trim = _load("qwen3_omni_captioner").trim_silence
    t = np.arange(R * 10) / R
    tone = (0.3 * np.sin(2 * np.pi * 220 * t)).astype("float32")
    wav = np.concatenate([np.zeros(R * 3, "float32"), tone, np.zeros(R * 5, "float32")])
    out = trim(wav, R)
    assert abs(out.size - tone.size) < R  # within a second of the tone
