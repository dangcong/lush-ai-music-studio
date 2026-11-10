# MODULES

## Audit repo cũ `SA1NG/ai_music_studio` @ v1.0.2

Khoảng 10.700 dòng Python. Kiến trúc: FastAPI (`api/`) + Gradio (`ui/`) + worker poll (`worker/`) + launcher desktop Windows (`scripts/`), tất cả chạy trên một máy cùng ACE-Step.

| Module cũ | Phân loại | Xử lý trong dự án mới |
|---|---|---|
| `worker/backends/acestep.py` — `AceStepClient`, generate | **Keep / Refactor** | Port sang `engines/acestep.py`, đổi payload theo protocol mới |
| `worker/backends/music_synth.py`, `mock.py` | **Keep** | Port sang `engines/music_synth.py`, `engines/mock.py` |
| `worker/backends/base.py` (interface backend) | **Refactor** | Thành `engines/base.py` có khai báo capabilities |
| `worker/worker.py`, `api_client.py` | **Rewrite** | `runner.py` theo protocol mới: lease, cancel, checksum, node token |
| `api/` (FastAPI, SQLite, queue) | **Replace** | Rust `apps/api`; giữ ý tưởng claim nguyên tử và requeue job treo |
| `ui/` (Gradio) | **Replace** | React `apps/web` |
| `acestep.py` — autolabel, train LoRA | **Keep cho Phase 3** | Chưa port. Chứa kinh nghiệm quý: quy trình scan → preprocess → train, xử lý NaN loss, phải `/v1/init` lại sau train |
| `shared/essentia_check.py`, `shared/audio.py` | **Keep cho Phase 3** | Đối chiếu BPM/key khi gán nhãn dataset |
| `shared/dataset_files.py`, `api/routes/datasets.py` | **Refactor ở Phase 3** | Dataset sẽ nằm trên Render Node, có version |
| `prompt_refiner/` | **Hoãn** | Tính năng tùy chọn, xét lại sau MVP |
| `scripts/desktop.py`, `update.py`, `make_release.py`, `*.bat` | **Remove** | Mô hình cài một máy + tự cập nhật không còn phù hợp |
| `scripts/model_manager.py`, `model_store.py` | **Tham khảo** | Cho script cài model trên Render Node |

Điều rút ra từ code cũ (đã được tác giả đối chiếu với mã nguồn ACE-Step commit `ca1e85f`):
- Luồng sinh nhạc: `POST /release_task` → poll `POST /query_result` → tải `GET /v1/audio`.
- `/v1/lora/unload` không gỡ được LoRA; dùng `/v1/lora/toggle` để tắt và `/v1/init` để có decoder sạch.
- LoRA là trạng thái dùng chung của model đang nạp → mỗi job phải tự đặt lại cho đúng.
- `audio_duration` nhận 10–600 giây; `lora scale` 0–1.

## Module hiện tại

Xem bảng thành phần trong `ARCHITECTURE.md`. Trạng thái theo phase: `ROADMAP.md`.
