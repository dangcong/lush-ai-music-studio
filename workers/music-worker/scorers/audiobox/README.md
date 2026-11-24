# Máy chấm Audiobox Aesthetics (chất lượng sản xuất) — đã tự kiểm trên rtx6000 (2026-11-24)

Script `scorer.py` nói giao thức scorer của worker (`docs/WORKER_PROTOCOL.md`, "Scorer subprocess"): mỗi dòng stdin `{"path": "<file>"}`, mỗi dòng stdout `{"quality": float}` hoặc `{"error": "..."}`, log ra stderr, thoát 0 khi stdin đóng. Khai báo `provides: "quality"`. Giá trị là `PQ` (production quality) của predictor.

## Trạng thái kiểm chứng

- 🟢 Vòng giao thức: có test `tests/test_scorer_scripts.py` với `LUSH_SCORER_STUB=1` (không import audiobox/torch).
- 🟢 **Đã tự kiểm thật trên rtx6000 (2026-11-24), chế độ offline:** sóng sin 10 s → 7.55, nhiễu hồng → 4.96, MP3 → 7.57; file không có → `{"error": ...}`. Chưa chạy trong một lượt `eval_lora` thật.

## Giấy phép

Trọng số `facebook/audiobox-aesthetics` và repo GitHub: **CC-BY 4.0** (đọc 2026-11-24), dùng thương mại được, cần ghi nguồn. Người dùng đã duyệt tải 2026-11-24.

## Cài đặt trên render node (rtx6000)

Gốc: `/media/ai-rtx6000/AI_DATA/AiStudio/models/audiobox/`. Các lệnh dưới đây chưa chạy.

```bash
ssh rtx6000
M=/media/ai-rtx6000/AI_DATA/AiStudio/models/audiobox
mkdir -p $M && cd $M
python3 -m venv .venv
.venv/bin/pip install -U pip
# Đúng bộ đã chạy được (Blackwell, CUDA 12.8; torch cùng bản với ACE-Step). torchaudio và torchcodec phải khớp torch.
uv pip install --python .venv/bin/python torch==2.10.0 torchaudio==2.10.0 "torchcodec==0.10.*" --index-url https://download.pytorch.org/whl/cu128 --extra-index-url https://pypi.org/simple
echo torch==2.10.0 > /tmp/c.txt
uv pip install --python .venv/bin/python -c /tmp/c.txt audiobox_aesthetics requests soundfile
```

### CHỈ CHẠY SAU KHI NGƯỜI DÙNG DUYỆT LICENSE (trọng số khoảng 400 MB trong `$M/hf`; venv khoảng 7 GB; AGENTS.md cấm tải model khi chưa có phê duyệt)

Gói tự tải checkpoint lần đầu gọi `initialize_predictor()`. Để tải có kiểm soát, chạy một lần thủ công sau khi duyệt:
```bash
HF_HOME=$M/hf .venv/bin/python -c "from audiobox_aesthetics.infer import initialize_predictor; initialize_predictor()"
```
`scorer.json` tự đặt `HF_HOME=$M/hf` cho máy chấm (qua `/usr/bin/env`); worker chạy máy chấm offline nên không tự tải.

## Khai báo cho worker

```bash
mkdir -p $SCORER_DIR/audiobox
cp scorer.py scorer.json README.md $SCORER_DIR/audiobox/
```
`scorer.json` đã trỏ `command` tới `$M/.venv/bin/python`; sửa nếu venv ở nơi khác.

## Tự kiểm (sau khi đã tải model)

```bash
echo '{"path":"/path/a.wav"}' | $M/.venv/bin/python scorer.py
```
Kỳ vọng: `{"quality": <số>}`. Thử không cần model: `LUSH_SCORER_STUB=1 python scorer.py`.
