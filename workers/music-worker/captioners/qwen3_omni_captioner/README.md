# Captioner Qwen3-Omni Captioner

Script `run.py` nói giao thức captioner của worker (`docs/WORKER_PROTOCOL.md`, "Captioner subprocess"), giống các captioner khác: stdin mỗi dòng `{"id","audio","prompt"}`, stdout mỗi dòng `{"id","text"}` hoặc `{"id","error"}`, log ra stderr, thoát 0 khi stdin đóng. Model nạp một lần mỗi job.

## Trạng thái kiểm chứng

- 🟢 Vòng đọc/ghi dòng, bắt lỗi từng bài: `tests/test_captioner_runner_protocol.py` (không cần torch).
- 🟡 **Đã tự kiểm thật trên rtx6000 (2026-11-24), chạy được nhưng chất lượng đáng ngờ:** bài ACE-Step 60 s và bản ghép 180 s → mô tả đúng lo-fi (kick, snare, hi-hat, bass, electric piano, không lời), nhưng **dài khoảng 400 từ và bịa âm thanh không có**: cả hai lần đều tả một "tiếng buzzer" không có trong đoạn 30 s đã gửi (đã đo RMS, nhạc đều đặn); gửi cả bài thì tả thêm một "tiếng sine 110 Hz" trong đoạn im lặng cuối. Model là máy mô tả âm thanh chung nên hay tả hiệu ứng/cắt ghép. File không có → `{"id","error"}`; nạp model + 3 bài khoảng 26 s. Nên đọc và sửa mô tả trước khi dùng train. Chưa chạy trong một job `caption_tracks` thật.

## Model và giấy phép

- HF: `Qwen/Qwen3-Omni-30B-A3B-Captioner`, revision `a2bd106cbf527db5676e79662674da22b0545ec0`, khoảng 63 GB (MoE 30B, 3B hoạt động).
- Model **chỉ nhận audio, không nhận câu lệnh**: trường `prompt` của worker bị bỏ qua. Trả văn bản tự do nên `"mode": "text"`.
- Model card khuyên đoạn audio **không quá 30 s**. Script mặc định gửi **30 s ở giữa bài** (*lựa chọn của tôi*: phần giữa đại diện bài hơn intro); đặt `QWEN3_CAPTION_SECONDS=0` để gửi cả bài.
- Đây là model mô tả âm thanh nói chung (lời nói, tiếng động, nhạc), không chuyên cho nhạc.
- **Giấy phép:** Apache 2.0 (model card). Người dùng yêu cầu cài 2026-11-24.

## Cài đặt trên rtx6000 (đã chạy 2026-11-24)

```bash
ssh rtx6000
export PATH=$HOME/.local/bin:$PATH
R=/media/ai-rtx6000/AI_DATA/AiStudio; M=$R/models/qwen3-omni-captioner
export UV_CACHE_DIR=$R/cache/uv UV_PYTHON_INSTALL_DIR=$R/cache/python
mkdir -p $M && cd $M
uv venv --python 3.12 .venv
uv pip install --python .venv/bin/python torch==2.10.0 torchaudio==2.10.0 --index-url https://download.pytorch.org/whl/cu128
uv pip install --python .venv/bin/python transformers==5.24.0 accelerate soundfile librosa huggingface_hub
# Processor Qwen-Omni đòi cả bộ xử lý ảnh (torchvision + Pillow), dù chỉ dùng audio:
uv pip install --python .venv/bin/python torchvision==0.25.0 pillow --index-url https://download.pytorch.org/whl/cu128 --extra-index-url https://pypi.org/simple
HF_HUB_CACHE=$R/cache/hf .venv/bin/hf download Qwen/Qwen3-Omni-30B-A3B-Captioner --revision a2bd106cbf527db5676e79662674da22b0545ec0 --local-dir $M/weights/Qwen3-Omni-30B-A3B-Captioner
```

## Khai báo cho worker

```bash
mkdir -p $CAPTIONER_DIR/qwen3_omni_captioner
cp run.py $CAPTIONER_DIR/qwen3_omni_captioner/
cp captioner.json.example $CAPTIONER_DIR/qwen3_omni_captioner/captioner.json
```

Biến môi trường (trong `config/worker.env` trên rtx6000): `QWEN3_CAPTIONER_MODEL_DIR=$M/weights/Qwen3-Omni-30B-A3B-Captioner`, tùy chọn `QWEN3_CAPTION_SECONDS` (mặc định 30; 0 = cả bài), `QWEN3_CAPTIONER_MAX_NEW_TOKENS`. `deploy.sh` không cập nhật thư mục captioner: sửa script thì chép tay lên máy.

## Tự kiểm

```bash
cd $CAPTIONER_DIR/qwen3_omni_captioner
echo '{"id":"x","audio":"/path.wav","prompt":null}' | QWEN3_CAPTIONER_MODEL_DIR=$M/weights/Qwen3-Omni-30B-A3B-Captioner $M/.venv/bin/python run.py
```
