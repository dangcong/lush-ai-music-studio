# CHANGELOG

Thay đổi của sản phẩm theo ngày, mới nhất ở trên. Chi tiết kiểm thử nằm ở `tasks/COMPLETED.md`.

## Chưa phát hành

- Docs: thêm hệ thống bộ nhớ dự án (`MEMORY_INDEX.md`, `CHANGELOG.md`, `tasks/`).

## 2026-11-10

- Render Node `render-node-01` chạy thật: ACE-Step 1.5 (`ca1e85f`, model `acestep-v15-turbo`) + worker, mọi file trên NVMe `/mnt/ai-studio/AiStudio`; VPS listener 8081 (chỉ Tailscale) đã áp. Job tạo nhạc thật đầu tiên từ https://studio.darkambient.co. Gỡ Stable Diffusion Forge khỏi máy render theo yêu cầu. — `83f7938`
- Render Node: worker chạy bằng systemd **user unit** (không cần root), thêm `acestep-api.service` (ACE-Step chỉ nghe localhost). VPS nginx: listener 8081 cho worker, chỉ cho phép dải Tailscale. Đã xác minh trên node thật (xem mục trên). — `e1c48a1`
- Website công khai tại https://studio.darkambient.co (nginx + Let's Encrypt). — `27c5668`
- Control plane triển khai lên VPS `lush1-vps` (systemd `lush-api`). — `ed3b7c4`, `1f7d8e5`
- Phase 0–1: API Rust, worker Python + Mock Engine, web React, hợp đồng API + worker protocol, CI. Nền tảng Phase 2 (Genre/Model Registry, adapter ACE-Step chỉ test bằng HTTP giả). — `cdcd750`
