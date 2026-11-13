# CURRENT — Việc đang làm

Cập nhật: 2026-11-13. Đọc file này đầu mỗi phiên. Giữ ngắn: chỉ task đang mở; task xong chuyển sang `COMPLETED.md`.

## Đang làm

### T-007 Reference Audio trong form Tạo nhạc — 🟡 đã deploy, chờ thử trên production (2026-11-13)
- Đã làm: API (`routes/inputs.rs`, migration `0003`, `mode` trong `/generations`), worker (tải input + multipart tới ACE-Step, capability `reference_audio`/`cover`), web (3 chế độ, `AudioDropzone`, hiển thị ở track/job), nginx trong repo, hai hợp đồng, `MUSIC_GENERATION.md`, D19–D20.
- Đã xác minh (máy dev): `cargo test` 12/12, `pytest` 105/105, `npm test` 67/67, clippy/build/lint sạch. Chạy thật: API + worker bản mới ở local, ACE-Step thật qua SSH tunnel → text, reference, cover đều ra WAV 48 kHz stereo; đầu vào WAV/MP3/FLAC; track + metadata vào thư viện. Giao diện đã xem bằng Chrome headless.
- 2026-11-13, theo yêu cầu người dùng: gộp Text và Reference thành **một form** (Reference Audio không bắt buộc), gỡ Cover / Remix khỏi giao diện; API/worker giữ nguyên. `npm test` 65/65, build, lint sạch. Chưa xem lại bằng trình duyệt sau khi gộp.
- Chưa xác minh: production; kéo-thả trên trình duyệt thật; chất lượng nghe.
- 2026-11-13: đã áp nginx trên VPS (người dùng đồng ý), commit `7a481fb`, `/deploy` xong (VPS + Render, migration `0003`). Upload 2 MB chưa đăng nhập qua nginx trả `401` (không còn `413`).
- **Bước tiếp theo:** người dùng thử trên https://suno.darkambient.co (tạo nhạc không file, tạo nhạc có Reference Audio); xong thì chuyển T-007 sang `COMPLETED.md`.

### T-008 AI Training — E0 + E1 xong, chờ người dùng duyệt giai đoạn tiếp (2026-11-13)
- Thiết kế, phạm vi đã chốt (chỉ instrumental, chỉ RTX 3060, Style LoRA) và kết quả đo: `LORA_TRAINING.md` (mục 5.1 và 12). Đề xuất mở rộng protocol, **chưa triển khai**: cuối `WORKER_PROTOCOL.md`.
- E0: render node đã reboot (kernel `7.0.0-43`, driver 595.99.02, `nvidia-smi` chạy lại). Reboot gọi qua phiên desktop vì agent không có sudo. Desktop không tự đăng nhập sau reboot.
- E1 (smoke test kỹ thuật, chạy thật): preprocess 17 s; train 4 epoch rồi resume tới 8 epoch (16 optimizer step), loss hữu hạn, VRAM đỉnh ~5,4 GB, dùng 55 s trên trần 30 phút; adapter nạp qua `/v1/lora/load`, output bật/tắt khác nhau; inference đã khôi phục (bài kiểm tra trùng từng byte với trước E1).
- Chưa kiểm chứng: chất lượng (chưa ai nghe), vocal contamination, segment dài hơn 60 s, dừng giữa chừng / OOM / NaN, dataset thật.
- Dữ liệu E1 nằm ở `data/training/e1/` và `data/datasets/e1-feasibility/` trên node; không phải dataset thật, không đăng ký vào registry.
- **Bước tiếp theo:** người dùng duyệt giai đoạn kế (đề xuất: E2 baseline + E3 với dataset thật do người dùng cung cấp, rồi mới code M1). Chưa viết code training, chưa đổi database/API/UI/queue.

## Chờ người dùng

Danh sách đầy đủ ở `ROADMAP.md` (mục "Việc cần người quyết định"). Mọi bước deploy, push, tải model đều cần phê duyệt.

## Tiếp theo

1. Người dùng nghe thử, đánh giá chất lượng theo genre; điều chỉnh prompt template của genre nếu cần.
2. ~~Thử khởi động lại máy render~~ — xong 2026-11-13: NVMe tự mount, hai dịch vụ tự chạy.
3. Benchmark đầy đủ hơn (180–240 s, nhiều genre) — `TESTING.md`.
4. Phase 3 (dataset, LoRA training) — dữ liệu và checkpoint đặt ở `/mnt/ai-studio/AiStudio/data/`.
