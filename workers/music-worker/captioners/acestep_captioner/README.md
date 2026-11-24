# Captioner ACE-Step Captioner

Script `run.py` nói giao thức captioner của worker (`docs/WORKER_PROTOCOL.md`, "Captioner subprocess"), giống các captioner khác: stdin mỗi dòng `{"id","audio","prompt"}`, stdout mỗi dòng `{"id","text"}` hoặc `{"id","error"}`, log ra stderr, thoát 0 khi stdin đóng. Model nạp một lần mỗi job.

## Trạng thái kiểm chứng

- 🟢 Vòng đọc/ghi dòng, bắt lỗi từng bài: `tests/test_captioner_runner_protocol.py` (không cần torch).
- 🟢 **Đã tự kiểm thật trên rtx6000 (2026-11-24):** bài ACE-Step 60 s và bản ghép 180 s → mỗi bài một đoạn mô tả tiếng Anh sạch, đúng chất lo-fi (bass, electric piano/guitar, trống, vinyl crackle, "instrumental"); file không có → `{"id","error"}`; nạp model + 3 bài khoảng 8 s. Hai lần mô tả cùng một nhạc có thể khác nhau ở chi tiết (piano hay guitar). Log transformers có vài cảnh báo token id của phần talker (không dùng), không ảnh hưởng. Chưa chạy trong một job `caption_tracks` thật.

## Model và giấy phép

- HF: `ACE-Step/acestep-captioner`, revision `7109e9af1c6cfe3371730310b3a5cb773145b11e`, khoảng 22 GB. Model gắn nhãn dữ liệu train của ACE-Step 1.5, tinh chỉnh từ Qwen2.5-Omni-7B.
- Script chỉ nạp phần "thinker" (ra chữ), không nạp phần tạo giọng nói. Prompt theo model card: `*Task* Describe this audio in detail`.
- Trả **văn bản tự do** (không phải JSON) nên `captioner.json` để `"mode": "text"`: worker dùng nguyên văn làm Caption, Mood/Instruments/Vocals rút bằng từ khóa.
- Cả bài được đưa vào model (không cắt).
- **Giấy phép:** model card ghi MIT; repo không có file LICENSE riêng. Người dùng yêu cầu cài 2026-11-24.

## Cài đặt trên rtx6000 (đã chạy 2026-11-24)

```bash
ssh rtx6000
export PATH=$HOME/.local/bin:$PATH
R=/media/ai-rtx6000/AI_DATA/AiStudio; M=$R/models/acestep-captioner
export UV_CACHE_DIR=$R/cache/uv UV_PYTHON_INSTALL_DIR=$R/cache/python
mkdir -p $M && cd $M
uv venv --python 3.12 .venv
uv pip install --python .venv/bin/python torch==2.10.0 torchaudio==2.10.0 --index-url https://download.pytorch.org/whl/cu128
uv pip install --python .venv/bin/python transformers==5.24.0 accelerate soundfile librosa huggingface_hub
# Processor Qwen-Omni đòi cả bộ xử lý ảnh (torchvision + Pillow), dù chỉ dùng audio:
uv pip install --python .venv/bin/python torchvision==0.25.0 pillow --index-url https://download.pytorch.org/whl/cu128 --extra-index-url https://pypi.org/simple
HF_HUB_CACHE=$R/cache/hf .venv/bin/hf download ACE-Step/acestep-captioner --revision 7109e9af1c6cfe3371730310b3a5cb773145b11e --local-dir $M/weights/acestep-captioner
```

## Khai báo cho worker

```bash
mkdir -p $CAPTIONER_DIR/acestep_captioner
cp run.py $CAPTIONER_DIR/acestep_captioner/
cp captioner.json.example $CAPTIONER_DIR/acestep_captioner/captioner.json
```

Biến môi trường (trong `config/worker.env` trên rtx6000): `ACESTEP_CAPTIONER_MODEL_DIR=$M/weights/acestep-captioner`, tùy chọn `ACESTEP_CAPTIONER_MAX_NEW_TOKENS`. `deploy.sh` không cập nhật thư mục captioner: sửa script thì chép tay lên máy.

## Tự kiểm

```bash
cd $CAPTIONER_DIR/acestep_captioner
echo '{"id":"x","audio":"/path.wav","prompt":null}' | ACESTEP_CAPTIONER_MODEL_DIR=$M/weights/acestep-captioner $M/.venv/bin/python run.py
```
