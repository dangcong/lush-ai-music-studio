# Máy chấm MERT (embedding phong cách) — Chưa chạy thật

Script `scorer.py` nói giao thức scorer của worker (`docs/WORKER_PROTOCOL.md`, "Scorer subprocess"): mỗi dòng stdin `{"path": "<file>"}`, mỗi dòng stdout `{"embedding": [float, ...]}` hoặc `{"error": "..."}`, log ra stderr, thoát 0 khi stdin đóng. Model nạp một lần lúc khởi động. Khai báo `provides: "embedding"`.

## Trạng thái kiểm chứng

- 🟢 Vòng đọc/ghi dòng, lỗi từng file, bảo vệ stdout: có test `tests/test_scorer_scripts.py` với `LUSH_SCORER_STUB=1` (model giả, không import torch/transformers).
- 🟡 **Chưa kiểm chứng:** `load_model()` / `score_file()` viết theo tài liệu thư viện (`transformers.AutoModel` với `trust_remote_code=True`, `Wav2Vec2FeatureExtractor`, `output_hidden_states=True`), chưa chạy lần nào. **Chưa chạy thật** trên GPU; cần đối chiếu với model card khi chạy thử lần đầu.
- Cách tính: nạp 60 giây đầu, resample về tần số của extractor (kỳ vọng 24 kHz), lấy hidden state lớp cuối (`hidden_states[-1]`), trung bình theo thời gian → vector (kỳ vọng 1024 chiều).

## Giấy phép

Model `m-a-p/MERT-v1-330M`: **cần kiểm tra LICENSE trên trang model trước khi tải** (https://huggingface.co/m-a-p/MERT-v1-330M). Chưa đọc nguyên văn nên chưa kết luận về việc dùng thương mại. Người dùng phải duyệt trước khi tải.

## Cài đặt trên render node (rtx6000)

Gốc: `/media/ai-rtx6000/AI_DATA/AiStudio/models/mert/`. Các lệnh dưới đây chưa chạy.

```bash
ssh rtx6000
M=/media/ai-rtx6000/AI_DATA/AiStudio/models/mert
mkdir -p $M && cd $M
python3 -m venv .venv
.venv/bin/pip install -U pip
# torch theo CUDA của node (kiểm tra bằng nvidia-smi), rồi:
.venv/bin/pip install torch transformers librosa soundfile huggingface_hub
```

### CHỈ CHẠY SAU KHI NGƯỜI DÙNG DUYỆT LICENSE (tải khoảng 1.3 GB, ước tính; AGENTS.md cấm tải model khi chưa có phê duyệt)

```bash
HF_HOME=$M/hf .venv/bin/huggingface-cli download m-a-p/MERT-v1-330M
```
Đặt `HF_HOME=$M/hf` trong môi trường worker để script tìm lại cache. Có thể đặt `MERT_MODEL_ID` (mặc định `m-a-p/MERT-v1-330M`).

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
