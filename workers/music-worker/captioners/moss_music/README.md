# Captioner MOSS-Music 8B — đã tự kiểm trên rtx6000 (2026-11-24)

Script `run.py` nói giao thức captioner của worker (`docs/WORKER_PROTOCOL.md`, "Captioner subprocess"):
stdin mỗi dòng `{"id","audio","prompt"}`, stdout mỗi dòng `{"id","text"}` hoặc `{"id","error"}`, log ra stderr, thoát 0 khi stdin đóng. Model nạp một lần lúc khởi động.

## Trạng thái kiểm chứng

- 🟢 Vòng đọc/ghi dòng, bắt lỗi từng bài: có test `tests/test_moss_runner_protocol.py` (monkeypatch, không cần torch).
- 🟢 **Đã tự kiểm thật trên rtx6000 (2026-11-24):** một bài ACE-Step 60 s → JSON sạch đủ 4 khóa (mood, instruments, vocals "Instrumental", caption); file không có → `{"id","error"}`; nạp model + một bài khoảng 60 s. `load_model()`/`generate()` khớp `infer.py` thật ở commit `ad107c7`. Câu mô tả vẫn có thể nêu BPM/key dù prompt dặn không. Chưa chạy trong một job `caption_tracks` thật.
- 🟡 Prompt mặc định là prompt JSON của worker; model có thể không trả JSON sạch (worker có đường lùi: tách trường từ văn bản).

- Khi `prompt` là null, script dùng prompt JSON mặc định, nên `captioner.json` phải để `"mode": "json"` (hoặc tự đặt `prompt`).
- Thời gian nạp model tính vào hạn 600 s của bài đầu tiên (worker bắt đầu đếm từ lúc gửi dòng đầu); nếu nạp chậm, bài đầu có thể bị coi là treo.
- Stdout của script được bảo vệ: fd 1 bị chuyển sang stderr, chỉ dòng giao thức đi qua bản sao stdout thật.

## Nguồn đã đọc (2026-11-24)

- Model card: https://huggingface.co/OpenMOSS-Team/MOSS-Music-8B-Instruct — HF revision `fce7f8304e96cc2d3398b8106456cbb2ecec3139` (nhánh main, "Upload moss-music_img.png"). Model card chỉ có `AutoModel.from_pretrained(..., trust_remote_code=True, device_map="auto")`, không nêu processor/cách đưa audio; khuyến nghị dùng SGLang.
- GitHub: https://github.com/OpenMOSS/MOSS-Music — commit `main` `ad107c7ddaa06de168a0dfbc18d3e1e6a40c0e5e` (2026-05-09); `infer.py` ở gốc repo (https://raw.githubusercontent.com/OpenMOSS/MOSS-Music/main/infer.py).
- `captioner.json.example` dùng 12 ký tự đầu của HF revision làm `version`.

## Giấy phép

Trọng số: Apache License 2.0 (model card `license: apache-2.0`, đọc lại 2026-11-24). Cho phép dùng thương mại; giữ thông báo giấy phép khi phân phối lại. Repo GitHub `OpenMOSS/MOSS-Music` **không có file LICENSE** (pyproject trỏ tới `LICENSE` nhưng file không tồn tại); ta chỉ dùng `src/` để chạy nội bộ, không phân phối lại. Người dùng duyệt tải 2026-11-24.

## Cài đặt trên render node (rtx6000)

Gốc: `/media/ai-rtx6000/AI_DATA/AiStudio/models/moss-music/`. Các lệnh dưới đây chưa chạy.

```bash
ssh rtx6000
R=/media/ai-rtx6000/AI_DATA/AiStudio; M=$R/models/moss-music
export UV_CACHE_DIR=$R/cache/uv UV_PYTHON_INSTALL_DIR=$R/cache/python
mkdir -p $M && cd $M
# Chỉ src/ của repo (bỏ thư mục sglang/ rất lớn), ghim commit ad107c7.
git clone --filter=blob:none --no-checkout https://github.com/OpenMOSS/MOSS-Music repo
git -C repo sparse-checkout set --no-cone "/src/" "/infer.py" "/pyproject.toml" "/README.md"
git -C repo checkout ad107c7ddaa06de168a0dfbc18d3e1e6a40c0e5e
uv venv --python 3.12 .venv
# Đúng bộ tác giả ghim (pyproject: torch-runtime). torchcodec phải là bản CPU từ PyPI:
# bản +cu128 cần thư viện NVIDIA NPP (libnppicc) không có trên máy.
uv pip install --python .venv/bin/python torch==2.9.1 torchaudio==2.9.1 --index-url https://download.pytorch.org/whl/cu128
echo torch==2.9.1 > /tmp/cm.txt
uv pip install --python .venv/bin/python -c /tmp/cm.txt "torchcodec==0.9.1" transformers==4.57.1 accelerate "safetensors>=0.4" "numpy>=2" soundfile tiktoken einops scipy tqdm packaging requests huggingface_hub librosa
```
venv khoảng 7 GB.

### CHỈ CHẠY SAU KHI NGƯỜI DÙNG DUYỆT (tải 18 GB, chiếm 17 GB; AGENTS.md cấm tải model lớn khi chưa có phê duyệt)

```bash
.venv/bin/hf download OpenMOSS-Team/MOSS-Music-8B-Instruct \
  --revision fce7f8304e96cc2d3398b8106456cbb2ecec3139 \
  --local-dir $M/weights/MOSS-Music-8B-Instruct
```

## Khai báo cho worker

```bash
mkdir -p $CAPTIONER_DIR/moss_music   # CAPTIONER_DIR là biến môi trường của worker
cp run.py $CAPTIONER_DIR/moss_music/
cp captioner.json.example $CAPTIONER_DIR/moss_music/captioner.json
```

Biến môi trường cho script (đã đặt trong `config/worker.env` trên rtx6000, cùng `CAPTIONER_DIR=$R/captioners`): `MOSS_MUSIC_MODEL_DIR=$M/weights/MOSS-Music-8B-Instruct`, `MOSS_MUSIC_REPO_DIR=$M/repo`, tùy chọn `MOSS_MUSIC_MAX_NEW_TOKENS`.

## Tự kiểm (sau khi đã tải model)

```bash
cd $CAPTIONER_DIR/moss_music
echo '{"id":"x","audio":"/path.mp3","prompt":null}' | \
  MOSS_MUSIC_MODEL_DIR=$M/weights/MOSS-Music-8B-Instruct MOSS_MUSIC_REPO_DIR=$M/repo \
  $M/.venv/bin/python run.py
```

Kỳ vọng: một dòng `{"id":"x","text":"..."}` trên stdout, log `[moss-music] ...` trên stderr. Khi lỗi: `{"id":"x","error":"..."}`. Nếu nạp model lỗi, script thoát mã 1.
