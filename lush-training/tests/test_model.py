import json
import struct

import numpy as np
import pytest

from lush_training import model, safetensors_io
from lush_training.util import LushTrainingError


def make_checkpoint(d, *, turbo=False, arch="AceStepConditionGenerationModel", seed=0):
    d.mkdir(parents=True)
    rng = np.random.default_rng(seed)
    arrays = {"decoder.block.0.weight": rng.standard_normal((8, 4)), "decoder.block.0.bias": rng.standard_normal(8),
              "decoder.block.1.weight": rng.standard_normal((4, 4)), "encoder.proj.weight": rng.standard_normal((3, 3)),
              "tokenizer.emb": rng.standard_normal((5, 2)), "null_condition_emb": rng.standard_normal((1, 1, 2))}
    safetensors_io.write_numpy(d / "model.safetensors", arrays, {"format": "pt"})
    (d / "config.json").write_text(json.dumps({"architectures": [arch], "is_turbo": turbo, "timestep_mu": -0.4,
                                               "auto_map": {"AutoModel": "modeling_acestep_v15_base.X"}}))
    (d / "modeling_acestep_v15_base.py").write_text("# engine code\n")
    (d / "silence_latent.pt").write_bytes(b"\x00" * 16)
    return arrays


def test_inspect_counts_parameters_from_the_file(tmp_path):
    make_checkpoint(tmp_path / "m")
    r = model.inspect_checkpoint(tmp_path / "m", with_hash=True)
    assert r["components"]["decoder"]["parameters"] == 32 + 8 + 16
    assert r["components"]["encoder"]["parameters"] == 9
    assert r["total_parameters"] == 56 + 9 + 10 + 2
    assert len(r["weights_sha256"]) == 64
    assert r["compatibility"]["compatible"] and r["compatibility"]["backend"] == "acestep15-dit"


def test_full_decoder_is_not_reported_as_full_model(tmp_path):
    make_checkpoint(tmp_path / "m")
    r = model.inspect_checkpoint(tmp_path / "m")
    plan = model.trainable_plan(r, "full_decoder")
    assert plan["trainable_parameters"] == 56 < plan["total_parameters"]
    assert "encoder" in plan["frozen_components"] and "KHÔNG phải" in plan["description"]
    partial = model.trainable_plan(r, "partial_decoder", [r"block\.1\."], model.tensor_counts(tmp_path / "m"))
    assert partial["trainable_parameters"] == 16


@pytest.mark.parametrize("method", ["lora", "full_model"])
def test_unimplemented_methods_are_refused_not_substituted(tmp_path, method):
    make_checkpoint(tmp_path / "m")
    with pytest.raises(LushTrainingError):
        model.trainable_plan(model.inspect_checkpoint(tmp_path / "m"), method)


def test_incompatible_checkpoints(tmp_path):
    make_checkpoint(tmp_path / "other")
    (tmp_path / "other" / "config.json").write_text(json.dumps({"architectures": ["SomeOtherModel"]}))
    r = model.inspect_checkpoint(tmp_path / "other")
    assert not r["compatibility"]["compatible"] and r["compatibility"]["methods"] == []
    make_checkpoint(tmp_path / "turbo", turbo=True)
    assert any("turbo" in w for w in model.inspect_checkpoint(tmp_path / "turbo")["compatibility"]["warnings"])


def test_truncated_or_garbage_weights_are_rejected(tmp_path):
    make_checkpoint(tmp_path / "m")
    f = tmp_path / "m" / "model.safetensors"
    f.write_bytes(f.read_bytes()[:-10])
    with pytest.raises(LushTrainingError):
        model.inspect_checkpoint(tmp_path / "m")
    f.write_bytes(struct.pack("<Q", 10**12) + b"x")
    with pytest.raises(LushTrainingError):
        model.inspect_checkpoint(tmp_path / "m")


def test_bf16_rounding_is_nearest_even():
    def f32(bits):
        return np.array([bits], dtype="<u4").view("<f4")
    assert safetensors_io.f32_to_bf16(np.array([1.0], dtype=np.float32))[0] == 0x3F80
    assert safetensors_io.f32_to_bf16(np.array([-2.5], dtype=np.float32))[0] == 0xC020
    assert safetensors_io.f32_to_bf16(f32(0x3F808000))[0] == 0x3F80   # tie -> even (down)
    assert safetensors_io.f32_to_bf16(f32(0x3F818000))[0] == 0x3F82   # tie -> even (up)
    assert safetensors_io.f32_to_bf16(f32(0x3F808001))[0] == 0x3F81   # above half -> up
    assert safetensors_io.f32_to_bf16(f32(0x3F807FFF))[0] == 0x3F80   # below half -> down


def test_bf16_rounding_matches_torch():
    torch = pytest.importorskip("torch")
    values = np.random.default_rng(1).standard_normal(20000).astype(np.float32) * 3
    expected = torch.from_numpy(values).to(torch.bfloat16).view(torch.int16).numpy().view("<u2")
    assert np.array_equal(safetensors_io.f32_to_bf16(values), expected)
