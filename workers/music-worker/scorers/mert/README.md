# Máy chấm MERT (embedding phong cách) — đã tự kiểm trên rtx6000 (2026-11-24)

Script `scorer.py` nói giao thức scorer của worker (`docs/WORKER_PROTOCOL.md`, "Scorer subprocess"): mỗi dòng stdin `{"path": "<file>"}`, mỗi dòng stdout `{"embedding": [float, ...]}` hoặc `{"error": "..."}`, log ra stderr, thoát 0 khi stdin đóng. Model nạp một lần lúc khởi động. Khai báo `provides: "embedding"`.

## Trạng thái kiểm chứng

- 🟢 Vòng đọc/ghi dòng, lỗi từng file, bảo vệ stdout: có test `tests/test_scorer_scripts.py` với `LUSH_SCORER_STUB=1` (model giả, không import torch/transformers).
- 🟢 **Đã tự kiểm thật trên rtx6000 (2026-11-24), offline:** vector 1024 chiều; hai sóng sin 440/445 Hz cosine 0.993, sin với nhiễu hồng 0.353; 3 file khoảng 3 giây. Với transformers 5, code của MERT bỏ qua `output_hidden_states`, nên script dùng `last_hidden_state` (lớp cuối). Revision ghim: `5240c2708a5acaee1007f43fb9735c7dcd0b78c9`. Chưa chạy trong một lượt `eval_lora` thật.
- Cách tính: nạp 60 giây đầu, resample về tần số của extractor (kỳ vọng 24 kHz), lấy hidden state lớp cuối (`hidden_states[-1]`), trung bình theo thời gian → vector (kỳ vọng 1024 chiều).

## Giấy phép

Model `m-a-p/MERT-v1-330M`: **CC-BY-NC 4.0** (đọc 2026-11-24) — **không dùng thương mại**. Người dùng duyệt tải 2026-11-24 để thử nghiệm; trước khi dùng cho sản phẩm thương mại phải xem lại.

## Cài đặt trên render node (rtx6000)

Gốc: `/media/ai-rtx6000/AI_DATA/AiStudio/models/mert/`. Các lệnh dưới đây chưa chạy.

```bash
ssh rtx6000
M=/media/ai-rtx6000/AI_DATA/AiStudio/models/mert
mkdir -p $M && cd $M
python3 -m venv .venv
.venv/bin/pip install -U pip
# torch theo CUDA của node (kiểm tra bằng nvidia-smi), rồi:
uv pip install --python .venv/bin/python torch==2.10.0 torchaudio==2.10.0 --index-url https://download.pytorch.org/whl/cu128
echo torch==2.10.0 > /tmp/c.txt
uv pip install --python .venv/bin/python -c /tmp/c.txt transformers librosa soundfile huggingface_hub nnAudio
```

### CHỈ CHẠY SAU KHI NGƯỜI DÙNG DUYỆT LICENSE (khoảng 1.2 GB; venv khoảng 7 GB; AGENTS.md cấm tải model khi chưa có phê duyệt)

```bash
# Chỉ các file cần (khoảng 1.2 GB), không tải MERT-v1-330M_fairseq.pt (4 GB):
HF_HOME=$M/hf .venv/bin/hf download m-a-p/MERT-v1-330M config.json preprocessor_config.json configuration_MERT.py modeling_MERT.py pytorch_model.bin --revision 5240c2708a5acaee1007f43fb9735c7dcd0b78c9
```
`scorer.json` tự đặt `HF_HOME=$M/hf` cho máy chấm (qua `/usr/bin/env`). Có thể đặt `MERT_MODEL_ID` (mặc định `m-a-p/MERT-v1-330M`).

## Khai báo cho worker

```bash
mkdir -p $SCORER_DIR/mert          # SCORER_DIR là biến môi trường của worker
cp scorer.py scorer.json README.md $SCORER_DIR/mert/
```
`scorer.json` đã trỏ `command` tới `$M/.venv/bin/python` (đường dẫn tuyệt đối trên rtx6000); sửa nếu venv ở nơi khác.

## Tự kiểm (sau khi đã tải model)

```bash
echo '{"path":"/path/a.wav"}' | $M/.venv/bin/python scorer.py
```
Kỳ vọng: một dòng `{"embedding":[...]}`; file không tồn tại → `{"error":"..."}`. Chạy thử không cần model: `LUSH_SCORER_STUB=1 python scorer.py`.
