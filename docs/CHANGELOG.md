# CHANGELOG

Thay đổi của sản phẩm theo ngày, mới nhất ở trên. Chi tiết kiểm thử nằm ở `tasks/COMPLETED.md`.

## Chưa phát hành

- Docs: `LORA_TRAINING.md` — audit và thiết kế AI Training (M0); chưa có code training.
- Render node: reboot 2026-11-13, driver NVIDIA khớp lại (595.99.02), `nvidia-smi` và số liệu GPU hoạt động trở lại.
- AI Training: smoke test E1 chạy thật trên RTX 3060 (LoRA bằng Side-Step: train, resume, nạp adapter vào inference) — kết quả ở `LORA_TRAINING.md` mục 5.1. Phạm vi chốt: chỉ instrumental (D22). `WORKER_PROTOCOL.md` tách phần đề xuất cho training khỏi hợp đồng đang chạy.
- **Reference Audio** (không bắt buộc) trong form Tạo nhạc: một form duy nhất, có file (WAV, MP3, FLAC ≤ 50 MB, kéo-thả, nghe thử) thì bài mới tham chiếu theo file, không có thì tạo theo prompt. Cover / Remix không có trên giao diện (API và worker vẫn hỗ trợ `mode: cover`, để dành). API `POST /audio-inputs`, `mode` trong `POST /generations`; worker tải file đầu vào và gọi ACE-Step bằng multipart. Track lưu chế độ và file đầu vào. *Đã chạy thật trên máy dev với GPU của render node; chưa deploy.*
- Web: sửa bố cục track card trong cột hẹp (hàng nút tải ép mất tiêu đề bài).
- Web: danh sách bài nhạc thay 5 sao và icon tải bằng hàng nút MP3 · WAV · MIDI · XML · PDF · ZIP. MIDI/MusicXML/PDF hiện ở trạng thái "chưa khả dụng" vì chưa có pipeline.
- API: `GET /tracks/{id}/archive` tải mọi file của track trong một ZIP.
- Website chuyển sang https://suno.darkambient.co; tên cũ studio.darkambient.co chuyển hướng 301.

- Quy trình deploy một lệnh `scripts/deploy.sh` + skill `/deploy`: kiểm tra trước, test, chỉ deploy phần thay đổi, tự rollback VPS và worker (`docs/DEPLOY.md`).
- Docs: thêm hệ thống bộ nhớ dự án (`MEMORY_INDEX.md`, `CHANGELOG.md`, `tasks/`).

## 2026-11-10

- Render Node `render-node-01` chạy thật: ACE-Step 1.5 (`ca1e85f`, model `acestep-v15-turbo`) + worker, mọi file trên NVMe `/mnt/ai-studio/AiStudio`; VPS listener 8081 (chỉ Tailscale) đã áp. Job tạo nhạc thật đầu tiên từ https://studio.darkambient.co. Gỡ Stable Diffusion Forge khỏi máy render theo yêu cầu. — `83f7938`
- Render Node: worker chạy bằng systemd **user unit** (không cần root), thêm `acestep-api.service` (ACE-Step chỉ nghe localhost). VPS nginx: listener 8081 cho worker, chỉ cho phép dải Tailscale. Đã xác minh trên node thật (xem mục trên). — `e1c48a1`
- Website công khai tại https://studio.darkambient.co (nginx + Let's Encrypt). — `27c5668`
- Control plane triển khai lên VPS `lush1-vps` (systemd `lush-api`). — `ed3b7c4`, `1f7d8e5`
- Phase 0–1: API Rust, worker Python + Mock Engine, web React, hợp đồng API + worker protocol, CI. Nền tảng Phase 2 (Genre/Model Registry, adapter ACE-Step chỉ test bằng HTTP giả). — `cdcd750`
