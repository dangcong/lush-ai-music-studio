# Máy chấm Audiobox Aesthetics (chất lượng sản xuất) — Chưa chạy thật

Script `scorer.py` nói giao thức scorer của worker (`docs/WORKER_PROTOCOL.md`, "Scorer subprocess"): mỗi dòng stdin `{"path": "<file>"}`, mỗi dòng stdout `{"quality": float}` hoặc `{"error": "..."}`, log ra stderr, thoát 0 khi stdin đóng. Khai báo `provides: "quality"`. Giá trị là `PQ` (production quality) của predictor.

## Trạng thái kiểm chứng

- 🟢 Vòng giao thức: có test `tests/test_scorer_scripts.py` với `LUSH_SCORER_STUB=1` (không import audiobox/torch).
- 🟡 **Chưa kiểm chứng:** `load_model()` / `score_file()` dùng `audiobox_aesthetics.infer.initialize_predictor()` và `predictor.forward([{"path": ...}])` theo tài liệu gói, đọc kết quả khóa `PQ`; chưa chạy lần nào. **Chưa chạy thật**; kiểm tra tên hàm và dạng kết quả khi cài thật.

## Giấy phép

Gói và trọng số Audiobox Aesthetics (Meta/facebook): **cần kiểm tra LICENSE trên trang model trước khi tải** (https://huggingface.co/facebook/audiobox-aesthetics và repo GitHub facebookresearch/audiobox-aesthetics). Chưa đọc nguyên văn nên chưa kết luận về việc dùng thương mại. Người dùng phải duyệt trước khi tải.

## Cài đặt trên render node (rtx6000)

Gốc: `/media/ai-rtx6000/AI_DATA/AiStudio/models/audiobox/`. Các lệnh dưới đây chưa chạy.

```bash
ssh rtx6000
M=/media/ai-rtx6000/AI_DATA/AiStudio/models/audiobox
mkdir -p $M && cd $M
python3 -m venv .venv
.venv/bin/pip install -U pip
# torch theo CUDA của node, rồi:
.venv/bin/pip install audiobox_aesthetics
```

### CHỈ CHẠY SAU KHI NGƯỜI DÙNG DUYỆT LICENSE (trọng số tải về lần chạy đầu, ước tính dưới 1 GB; AGENTS.md cấm tải model khi chưa có phê duyệt)

Gói tự tải checkpoint lần đầu gọi `initialize_predictor()`. Để tải có kiểm soát, chạy một lần thủ công sau khi duyệt:
```bash
HF_HOME=$M/hf .venv/bin/python -c "from audiobox_aesthetics.infer import initialize_predictor; initialize_predictor()"
```
Đặt `HF_HOME=$M/hf` trong môi trường worker.

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
