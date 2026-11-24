# Captioner NVIDIA Music Flamingo (2601)

Script `run.py` nói giao thức captioner của worker (`docs/WORKER_PROTOCOL.md`, "Captioner subprocess"), giống `moss_music/run.py`: stdin mỗi dòng `{"id","audio","prompt"}`, stdout mỗi dòng `{"id","text"}` hoặc `{"id","error"}`, log ra stderr, thoát 0 khi stdin đóng. Model nạp một lần mỗi job.

## Trạng thái kiểm chứng

- 🟢 Vòng đọc/ghi dòng, bắt lỗi từng bài: `tests/test_captioner_runner_protocol.py` (chạy chung cho mọi captioner, không cần torch).
- 🟢 **Đã tự kiểm thật trên rtx6000 (2026-11-24):** một bài ACE-Step 60 s → JSON sạch đủ 4 khóa (mood, instruments, vocals "Instrumental; no vocal lines are present.", caption); file không có → `{"id","error"}`; nạp model + một bài khoảng 5 s, VRAM thừa nhiều. Câu trả lời ở mỗi khóa dài hơn MOSS (mood là cả câu). Worker đã báo `music-flamingo-2601` khi register. Chưa chạy trong một job `caption_tracks` thật, chưa thử bài dài 3–4 phút.

## Model và giấy phép

- HF: `nvidia/music-flamingo-2601-hf`, revision `6b5be086d52f65a1e204cb0faf70bf54e2741ecd` (bản mới hơn `music-flamingo-hf`, model card của bản cũ khuyên dùng bản này). Khoảng 16.5 GB, một file `model.safetensors`.
- Chạy bằng transformers chuẩn (`MusicFlamingoForConditionalGeneration` + `AutoProcessor`), không cần repo riêng.
- Nghe theo cửa sổ 30 s, tối đa **20 phút** mỗi bài; dài hơn bị cắt (model card).
- **Giấy phép: NVIDIA OneWay Noncommercial License** — chỉ dùng phi thương mại/nghiên cứu. Spec LoRA Studio (W4) cho phép dùng, không chặn; mỗi mô tả lưu tên model đã viết nó để sau này lọc được. Người dùng yêu cầu cài 2026-11-24.

## Cài đặt trên rtx6000 (đã chạy 2026-11-24)

```bash
ssh rtx6000
export PATH=$HOME/.local/bin:$PATH
R=/media/ai-rtx6000/AI_DATA/AiStudio; M=$R/models/music-flamingo
export UV_CACHE_DIR=$R/cache/uv UV_PYTHON_INSTALL_DIR=$R/cache/python
mkdir -p $M && cd $M
uv venv --python 3.12 .venv
uv pip install --python .venv/bin/python torch==2.10.0 torchaudio==2.10.0 --index-url https://download.pytorch.org/whl/cu128
uv pip install --python .venv/bin/python "transformers>=5" accelerate soundfile librosa huggingface_hub   # được 5.24.0
HF_HUB_CACHE=$R/cache/hf .venv/bin/hf download nvidia/music-flamingo-2601-hf \
  --revision 6b5be086d52f65a1e204cb0faf70bf54e2741ecd --local-dir $M/weights/music-flamingo-2601-hf
```

## Khai báo cho worker

```bash
mkdir -p $CAPTIONER_DIR/music_flamingo
cp run.py $CAPTIONER_DIR/music_flamingo/
cp captioner.json.example $CAPTIONER_DIR/music_flamingo/captioner.json
```

Biến môi trường (trong `config/worker.env` trên rtx6000): `FLAMINGO_MODEL_DIR=$M/weights/music-flamingo-2601-hf`, tùy chọn `FLAMINGO_MAX_NEW_TOKENS`. `deploy.sh` không cập nhật thư mục captioner: sửa script thì chép tay lên máy.

## Tự kiểm

```bash
cd $CAPTIONER_DIR/music_flamingo
echo '{"id":"x","audio":"/path.wav","prompt":null}' | \
  FLAMINGO_MODEL_DIR=$M/weights/music-flamingo-2601-hf $M/.venv/bin/python run.py
```
