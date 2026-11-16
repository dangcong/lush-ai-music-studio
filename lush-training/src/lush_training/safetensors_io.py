"""Read and write `.safetensors` files with the standard library and numpy only.

Format: 8 bytes little-endian header length N, N bytes of JSON
(`{name: {dtype, shape, data_offsets: [begin, end]}, "__metadata__": {...}}`), then the raw tensor
bytes. Reading the header needs no torch and no GPU, so a checkpoint can be inspected anywhere.
"""
from __future__ import annotations

import json
import struct
from dataclasses import dataclass
from pathlib import Path
from typing import Any, BinaryIO, Iterator

import numpy as np

from .util import LushTrainingError

DTYPE_BYTES = {"F64": 8, "F32": 4, "F16": 2, "BF16": 2, "I64": 8, "I32": 4, "I16": 2, "I8": 1, "U8": 1, "BOOL": 1}
MAX_HEADER = 100 * 1024 * 1024


@dataclass(frozen=True)
class TensorInfo:
    name: str
    dtype: str
    shape: tuple[int, ...]
    begin: int
    end: int

    @property
    def count(self) -> int:
        n = 1
        for d in self.shape:
            n *= d
        return n

    @property
    def nbytes(self) -> int:
        return self.end - self.begin


@dataclass(frozen=True)
class Header:
    tensors: dict[str, TensorInfo]
    metadata: dict[str, str]
    data_start: int
    file_bytes: int


def read_header(path: Path) -> Header:
    path = Path(path)
    size = path.stat().st_size
    with open(path, "rb") as f:
        raw = f.read(8)
        if len(raw) < 8:
            raise LushTrainingError(f"{path}: file quá ngắn, không phải safetensors")
        (n,) = struct.unpack("<Q", raw)
        if n <= 0 or n > MAX_HEADER or 8 + n > size:
            raise LushTrainingError(f"{path}: header safetensors không hợp lệ (độ dài {n})")
        try:
            doc = json.loads(f.read(n).decode("utf-8"))
        except (UnicodeDecodeError, ValueError) as e:
            raise LushTrainingError(f"{path}: header safetensors không đọc được: {e}") from e
    metadata = doc.pop("__metadata__", None) or {}
    tensors: dict[str, TensorInfo] = {}
    for name, t in doc.items():
        try:
            begin, end = (int(x) for x in t["data_offsets"])
            info = TensorInfo(name, str(t["dtype"]), tuple(int(d) for d in t["shape"]), begin, end)
        except (KeyError, TypeError, ValueError) as e:
            raise LushTrainingError(f"{path}: mô tả tensor `{name}` không hợp lệ") from e
        width = DTYPE_BYTES.get(info.dtype)
        if width is None:
            raise LushTrainingError(f"{path}: dtype `{info.dtype}` của `{name}` không được hỗ trợ")
        if info.nbytes != info.count * width or 8 + n + end > size or begin < 0:
            raise LushTrainingError(f"{path}: kích thước tensor `{name}` không khớp header (file bị cắt?)")
        tensors[name] = info
    return Header(tensors=tensors, metadata=metadata, data_start=8 + n, file_bytes=size)


def read_tensor_bytes(f: BinaryIO, header: Header, info: TensorInfo) -> bytes:
    f.seek(header.data_start + info.begin)
    data = f.read(info.nbytes)
    if len(data) != info.nbytes:
        raise LushTrainingError(f"không đọc đủ dữ liệu của tensor `{info.name}`")
    return data


def f32_to_bf16(values: np.ndarray) -> np.ndarray:
    """float32 -> bfloat16 bit patterns (uint16), round-to-nearest-even like `tensor.to(torch.bfloat16)`.

    The caller must have rejected non-finite values first.
    """
    bits = np.ascontiguousarray(values, dtype="<f4").view("<u4").astype(np.uint64)
    rounded = bits + 0x7FFF + ((bits >> 16) & 1)
    return (rounded >> 16).astype("<u2")


def cast_f32_bytes(data: bytes, target: str, name: str) -> bytes:
    values = np.frombuffer(data, dtype="<f4")
    if not np.isfinite(values).all():
        raise LushTrainingError(f"tensor `{name}` chứa giá trị không hữu hạn (NaN/Inf); không xuất model này")
    if target == "F32":
        return data
    if target == "BF16":
        return f32_to_bf16(values).tobytes()
    if target == "F16":
        return values.astype("<f2").tobytes()
    raise LushTrainingError(f"không chuyển được F32 sang {target} cho tensor `{name}`")


def write_safetensors(path: Path, entries: list[tuple[str, str, tuple[int, ...], int]],
                      chunks: Iterator[bytes], metadata: dict[str, str] | None = None) -> None:
    """Stream a safetensors file. `entries` = (name, dtype, shape, nbytes) in the order `chunks` yields."""
    header: dict[str, Any] = {}
    offset = 0
    for name, dtype, shape, nbytes in entries:
        header[name] = {"dtype": dtype, "shape": list(shape), "data_offsets": [offset, offset + nbytes]}
        offset += nbytes
    if metadata:
        header["__metadata__"] = {str(k): str(v) for k, v in metadata.items()}
    raw = json.dumps(header, separators=(",", ":")).encode("utf-8")
    raw += b" " * ((8 - len(raw) % 8) % 8)
    with open(path, "wb") as f:
        f.write(struct.pack("<Q", len(raw)))
        f.write(raw)
        written = 0
        for (name, _dtype, _shape, nbytes), data in zip(entries, chunks, strict=True):
            if len(data) != nbytes:
                raise LushTrainingError(f"tensor `{name}`: {len(data)} byte, cần {nbytes}")
            f.write(data)
            written += nbytes
        f.flush()
        import os
        os.fsync(f.fileno())


def write_numpy(path: Path, arrays: dict[str, np.ndarray], metadata: dict[str, str] | None = None) -> None:
    """Small helper (tests, tools): write float32 arrays."""
    items = [(k, np.ascontiguousarray(v, dtype="<f4")) for k, v in arrays.items()]
    write_safetensors(path, [(k, "F32", tuple(v.shape), v.nbytes) for k, v in items],
                      (v.tobytes() for _, v in items), metadata)
