"""The MERT and Audiobox scorer scripts speak the scorer protocol (docs/WORKER_PROTOCOL.md).

Only the protocol is tested, with LUSH_SCORER_STUB=1 (no torch / transformers / audiobox).
The real model-loading code has never been run (see scorers/*/README.md)."""
import json
import math
import os
import subprocess
import sys
from pathlib import Path

import pytest

from lush_worker.scoring import load_scorers

SCORERS = Path(__file__).resolve().parents[1] / "scorers"
CASES = [("mert", "embedding"), ("audiobox", "quality")]


def _run(name: str, lines: list[str]) -> tuple[list[str], str, int]:
    env = {**os.environ, "LUSH_SCORER_STUB": "1"}
    p = subprocess.run([sys.executable, "scorer.py"], cwd=SCORERS / name, input="".join(l + "\n" for l in lines),
                       capture_output=True, text=True, encoding="utf-8", env=env, timeout=60)
    return p.stdout.splitlines(), p.stderr, p.returncode


@pytest.mark.parametrize("name,provides", CASES)
def test_scripts_speak_protocol_with_stub_model(name, provides, tmp_path):
    a, b = tmp_path / "a.wav", tmp_path / "b.wav"
    a.write_bytes(b"aaaa")
    b.write_bytes(b"bbbbbbbb")
    missing = tmp_path / "missing.wav"
    out, _err, code = _run(name, [json.dumps({"path": str(a)}), json.dumps({"path": str(b)}),
                                  json.dumps({"path": str(missing)}), json.dumps({"path": str(a)})])
    assert code == 0
    assert len(out) == 4  # exactly one line out per line in; the process survived the error
    objs = [json.loads(l) for l in out]
    for o in (objs[0], objs[1], objs[3]):
        assert set(o) == {provides}
        if provides == "embedding":
            assert isinstance(o["embedding"], list) and o["embedding"]
            assert all(isinstance(x, float) and math.isfinite(x) for x in o["embedding"])
        else:
            assert isinstance(o["quality"], float) and math.isfinite(o["quality"])
    assert objs[0] == objs[3]  # deterministic
    assert objs[0] != objs[1]
    assert set(objs[2]) == {"error"} and objs[2]["error"]


@pytest.mark.parametrize("name,provides", CASES)
def test_bad_lines_get_an_error_line_and_stub_never_imports_heavy_libs(name, provides):
    out, _err, code = _run(name, ["not json", json.dumps({"nopath": 1})])
    assert code == 0
    objs = [json.loads(l) for l in out]
    assert len(objs) == 2 and all(set(o) == {"error"} for o in objs)
    probe = "\n".join([
        "import io, runpy, sys",
        "mod = runpy.run_path('scorer.py', run_name='scorer')",
        "assert mod['main'](io.StringIO(''), io.StringIO()) == 0",
        "heavy = ('torch', 'transformers', 'audiobox_aesthetics', 'torchaudio', 'librosa')",
        "print(sorted(m for m in heavy if m in sys.modules))",
    ])
    p = subprocess.run([sys.executable, "-c", probe], cwd=SCORERS / name, capture_output=True, text=True,
                       env={**os.environ, "LUSH_SCORER_STUB": "1"})
    assert p.stdout.strip().splitlines()[-1] == "[]", p.stdout + p.stderr


@pytest.mark.parametrize("name,provides", CASES)
def test_scorer_json_is_valid_for_the_worker(name, provides):
    specs = load_scorers(SCORERS)
    assert name in specs and specs[name].provides == provides
    assert (SCORERS / name / "README.md").read_text(encoding="utf-8").count("Chưa chạy thật") >= 1


def test_library_prints_do_not_corrupt_stdout(tmp_path):
    f = tmp_path / "a.wav"
    f.write_bytes(b"x")
    env = {**os.environ, "LUSH_SCORER_STUB": "1", "LUSH_SCORER_STUB_NOISY": "1"}
    for name, _ in CASES:
        p = subprocess.run([sys.executable, "scorer.py"], cwd=SCORERS / name, input=json.dumps({"path": str(f)}) + "\n",
                           capture_output=True, text=True, env=env, timeout=60)
        assert len(p.stdout.splitlines()) == 1
        json.loads(p.stdout)
        assert "junk" in p.stderr
