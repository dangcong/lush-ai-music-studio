# CURRENT — Việc đang làm

Cập nhật: 2026-11-12. Đọc file này đầu mỗi phiên. Giữ ngắn: chỉ task đang mở; task xong chuyển sang `COMPLETED.md`.

## Đang làm

### T-007 Reference to Music + Cover/Remix — 🟡 code xong, chờ đưa lên production (2026-11-12)
- Đã làm: API (`routes/inputs.rs`, migration `0003`, `mode` trong `/generations`), worker (tải input + multipart tới ACE-Step, capability `reference_audio`/`cover`), web (3 chế độ, `AudioDropzone`, hiển thị ở track/job), nginx trong repo, hai hợp đồng, `MUSIC_GENERATION.md`, D19–D20.
- Đã xác minh (máy dev): `cargo test` 12/12, `pytest` 105/105, `npm test` 67/67, clippy/build/lint sạch. Chạy thật: API + worker bản mới ở local, ACE-Step thật qua SSH tunnel → text, reference, cover đều ra WAV 48 kHz stereo; đầu vào WAV/MP3/FLAC; track + metadata vào thư viện. Giao diện đã xem bằng Chrome headless.
- 2026-11-13, theo yêu cầu người dùng: gộp Text và Reference thành **một form** (Reference Audio không bắt buộc), gỡ Cover / Remix khỏi giao diện; API/worker giữ nguyên. `npm test` 65/65, build, lint sạch. Chưa xem lại bằng trình duyệt sau khi gộp.
- Chưa xác minh: production; kéo-thả trên trình duyệt thật; chất lượng nghe.
- **Bước tiếp theo, theo đúng thứ tự (đều cần người dùng):**
  1. Áp block `location = /api/v1/audio-inputs` vào nginx trên VPS (file của root, `deploy.sh` sẽ dừng vì lệch cấu hình cho tới khi áp): chép `infrastructure/vps/nginx-lush-studio.conf` → `/etc/nginx/sites-available/lush-studio.conf`, `nginx -t && systemctl reload nginx`.
  2. `/deploy` (VPS + Render: có migration `0003`, worker mới).
  3. Thử trên https://suno.darkambient.co: upload, reference, cover; rồi chuyển T-007 sang `COMPLETED.md`.
- Lưu ý render node: driver NVIDIA trên đĩa (595.99) lệch kernel module đang nạp (595.91) — `nvidia-smi` báo lỗi, worker không đọc được số liệu GPU, ACE-Step đang chạy vẫn tạo nhạc được. Deploy chỉ restart `lush-worker` (không dùng GPU) nên an toàn; **đừng restart `acestep-api` trước khi reboot máy**. Reboot cần người dùng đồng ý.

### T-008 AI Training — M1 đang làm, dừng ở E1 chờ người dùng cho phép dừng `acestep-api` (2026-11-13)
- Thiết kế và kế hoạch: `LORA_TRAINING.md`. Người dùng chọn làm M1 theo đề xuất (E0 → E1 chạy tay → code).
- E0: agent không có sudo trên render node nên không reboot được. Đã kiểm: module NVIDIA 595.99 có sẵn cho kernel mới `7.0.0-43`, NVMe trong fstab (`nofail`), linger bật, hai dịch vụ `enabled`. Tiến trình CUDA mới **vẫn khởi tạo được** trước reboot (chỉ NVML/`nvidia-smi` hỏng), nên E1 không bắt buộc chờ reboot.
- E1 đã chuẩn bị trên node: dataset `data/datasets/e1-feasibility/v1` (8 bài lo-fi 60 s do chính ACE-Step của studio tạo, có `dataset.json`, `manifest.json`); script `data/training/e1/e1.sh` (Side-Step `train.py --yes fixed`, rank 32, AdamW, gradient checkpointing, offload encoder, có đo VRAM bằng `torch.cuda.mem_get_info`). SHA-256 `acestep-v15-turbo/model.safetensors` bắt đầu bằng `3f6e0797`.
- **Bước tiếp theo:** dừng `acestep-api` → `bash e1.sh train 4` → resume tới 8 epoch → bật lại `acestep-api`, chờ `/health` → `/v1/lora/load` adapter → tạo nhạc bật/tắt cùng seed. Ghi số đo vào `LORA_TRAINING.md` mục 5 rồi mới viết code M1. Chưa có số đo nào.

## Chờ người dùng

Danh sách đầy đủ ở `ROADMAP.md` (mục "Việc cần người quyết định"). Mọi bước deploy, push, tải model đều cần phê duyệt.

## Tiếp theo

1. Người dùng nghe thử, đánh giá chất lượng theo genre; điều chỉnh prompt template của genre nếu cần.
2. Thử khởi động lại máy render (cần người dùng đồng ý): xác nhận ổ NVMe tự mount và hai dịch vụ tự chạy.
3. Benchmark đầy đủ hơn (180–240 s, nhiều genre) — `TESTING.md`.
4. Phase 3 (dataset, LoRA training) — dữ liệu và checkpoint đặt ở `/mnt/ai-studio/AiStudio/data/`.
