# Captioner MOSS-Music 8B (chưa chạy với GPU thật)

Script `run.py` nói giao thức captioner của worker (`docs/WORKER_PROTOCOL.md`, "Captioner subprocess"):
stdin mỗi dòng `{"id","audio","prompt"}`, stdout mỗi dòng `{"id","text"}` hoặc `{"id","error"}`, log ra stderr, thoát 0 khi stdin đóng. Model nạp một lần lúc khởi động.

## Trạng thái kiểm chứng

- 🟢 Vòng đọc/ghi dòng, bắt lỗi từng bài: có test `tests/test_moss_runner_protocol.py` (monkeypatch, không cần torch).
- 🟡 **Chưa kiểm chứng:** `load_model()` và `generate()` được viết theo `infer.py` của tác giả nhưng chưa chạy lần nào. Việc `infer.py` import `src.*` từ repo GitHub (nên cần `MOSS_MUSIC_REPO_DIR`), tên class `MossMusicModel`/`MossMusicProcessor`, `enable_time_marker=True`, `processor.config.mel_sr` đều lấy từ bản tóm tắt của trang đó, chưa đọc nguyên văn. Cần đối chiếu với `infer.py` thật khi chạy thử lần đầu.
- 🟡 Prompt mặc định là prompt JSON của worker; model có thể không trả JSON sạch (worker có đường lùi: tách trường từ văn bản).

- Khi `prompt` là null, script dùng prompt JSON mặc định, nên `captioner.json` phải để `"mode": "json"` (hoặc tự đặt `prompt`).
- Thời gian nạp model tính vào hạn 600 s của bài đầu tiên (worker bắt đầu đếm từ lúc gửi dòng đầu); nếu nạp chậm, bài đầu có thể bị coi là treo.
- Stdout của script được bảo vệ: fd 1 bị chuyển sang stderr, chỉ dòng giao thức đi qua bản sao stdout thật.

## Nguồn đã đọc (2026-11-24)

- Model card: https://huggingface.co/OpenMOSS-Team/MOSS-Music-8B-Instruct — HF revision `fce7f8304e96cc2d3398b8106456cbb2ecec3139` (nhánh main, "Upload moss-music_img.png"). Model card chỉ có `AutoModel.from_pretrained(..., trust_remote_code=True, device_map="auto")`, không nêu processor/cách đưa audio; khuyến nghị dùng SGLang.
- GitHub: https://github.com/OpenMOSS/MOSS-Music — commit `main` `ad107c7ddaa06de168a0dfbc18d3e1e6a40c0e5e` (2026-05-09); `infer.py` ở gốc repo (https://raw.githubusercontent.com/OpenMOSS/MOSS-Music/main/infer.py).
- `captioner.json.example` dùng 12 ký tự đầu của HF revision làm `version`.

## Giấy phép

Apache License 2.0 (model card hiển thị `license: apache-2.0` và README nói như vậy). Cho phép dùng thương mại; giữ thông báo giấy phép khi phân phối lại.

## Cài đặt trên render node (rtx6000)

Gốc: `/media/ai-rtx6000/AI_DATA/AiStudio/models/moss-music/`. Các lệnh dưới đây chưa chạy.

```bash
ssh rtx6000
M=/media/ai-rtx6000/AI_DATA/AiStudio/models/moss-music
mkdir -p $M && cd $M
python3 -m venv .venv
.venv/bin/pip install -U pip
# torch theo CUDA của node (kiểm tra bằng nvidia-smi), rồi:
.venv/bin/pip install torch transformers accelerate soundfile librosa huggingface_hub
git clone https://github.com/OpenMOSS/MOSS-Music repo   # cần cho `src.*` mà infer.py import; ghim commit ad107c7
.venv/bin/pip install -e repo   # hoặc cài theo pyproject.toml của repo
```

### CHỈ CHẠY SAU KHI NGƯỜI DÙNG DUYỆT (tải ~16 GB; AGENTS.md cấm tải model lớn khi chưa có phê duyệt)

```bash
.venv/bin/huggingface-cli download OpenMOSS-Team/MOSS-Music-8B-Instruct \
  --revision fce7f8304e96cc2d3398b8106456cbb2ecec3139 \
  --local-dir $M/weights/MOSS-Music-8B-Instruct
```

## Khai báo cho worker

```bash
mkdir -p $CAPTIONER_DIR/moss_music   # CAPTIONER_DIR là biến môi trường của worker
cp run.py $CAPTIONER_DIR/moss_music/
cp captioner.json.example $CAPTIONER_DIR/moss_music/captioner.json
```

Biến môi trường cho script (đặt trong môi trường worker): `MOSS_MUSIC_MODEL_DIR=$M/weights/MOSS-Music-8B-Instruct`, `MOSS_MUSIC_REPO_DIR=$M/repo`, tùy chọn `MOSS_MUSIC_MAX_NEW_TOKENS`.

## Tự kiểm (sau khi đã tải model)

```bash
cd $CAPTIONER_DIR/moss_music
echo '{"id":"x","audio":"/path.mp3","prompt":null}' | \
  MOSS_MUSIC_MODEL_DIR=$M/weights/MOSS-Music-8B-Instruct MOSS_MUSIC_REPO_DIR=$M/repo \
  $M/.venv/bin/python run.py
```

Kỳ vọng: một dòng `{"id":"x","text":"..."}` trên stdout, log `[moss-music] ...` trên stderr. Khi lỗi: `{"id":"x","error":"..."}`. Nếu nạp model lỗi, script thoát mã 1.
