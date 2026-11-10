# ROADMAP

Cập nhật: 2026-11-10. Ký hiệu: ✅ xong và đã kiểm thử · 🟡 có code, chưa kiểm thử thật · ⬜ chưa làm.

## Phase 0 — Khởi tạo ✅
Audit repo cũ, chốt kiến trúc, cấu trúc monorepo, hợp đồng API + worker protocol, hạ tầng test.

## Phase 1 — Core Platform ✅ (trên máy dev, mock engine)
- ✅ Rust backend: auth, users/roles, projects, database + migrations, job queue, worker protocol, render node registry, audit log.
- ✅ Python worker + Mock Engine.
- ✅ Tích hợp thật trên Windows: tạo job qua API → worker claim → WAV → upload → track trong thư viện.
- ✅ Frontend: đăng nhập, Tạo nhạc, Thư viện, Chi tiết track, Hàng đợi, Quản trị. Đã chạy với backend thật trên Chrome (đăng nhập, Generate, tiến độ, waveform, Render Nodes). Các trang quản trị Genres/Presets/Users/Audit và Thư viện/Hàng đợi chưa được bấm thử từng thao tác.

## Phase 2 — Music Generation MVP 🟡
- ✅ Create Music, Genre Registry + presets, Model Registry, hàng đợi, thư viện, player, tải WAV.
- ✅ **ACE-Step adapter** chạy thật trên `render-node-01` (RTX 3060 12GB): job tạo nhạc đầu tiên qua website công khai ngày 2026-11-10.
- ✅ MP3 preview: `ffmpeg` đã cài trên Render Node, artifact `audio_preview` tạo thật.
- ✅ VPS: website chạy tại https://studio.darkambient.co (2026-11-10). Xem `VPS_DEPLOYMENT.md`.
- ✅ Render Node `render-node-01`: ACE-Step 1.5 turbo + worker (systemd user unit), mọi file trên NVMe `/mnt/ai-studio/AiStudio`, nối VPS qua Tailscale. Xem `RENDER_NODE_DEPLOYMENT.md`.
- 🟡 Benchmark RTX 3060 12GB: mới đo 2 điểm (30 s và 120 s, xem `TESTING.md`). Chưa đánh giá chất lượng bằng tai theo genre, chưa đo 180–240 s.

**Việc cần người quyết định / cung cấp để hoàn tất Phase 2:**
1. Nghe thử và đánh giá chất lượng nhạc theo từng genre.
2. GitHub private repository để push.

## Phase 3 — Custom Style Training ⬜
Dataset Manager (upload thẳng tới Render Node, không qua VPS), auto caption + human review, validation, versioning; LoRA training qua Side-Step; tiến độ thật (loss, epoch, VRAM); checkpoint; LoRA Library + phê duyệt; trained style presets.
Đã chuẩn bị sẵn: bảng `loras`, `lora_installations`, kiểm tra tương thích LoRA ↔ base model khi tạo job, scheduler theo LoRA, role `trainer`, worker nạp LoRA từ `LORA_DIR`.
Trước khi bắt đầu: xác nhận giấy phép Side-Step; benchmark rank/alpha/LR trên 12GB VRAM; quyết định cách train và generate chia sẻ một GPU (giải phóng model inference).

## Phase 4 — Music Production Export ⬜
Audio analysis, Audio→MIDI (đánh giá Basic Pitch và các model piano/multi-instrument), MusicXML, PDF (music21 / MuseScore CLI / LilyPond), gói export cho FL Studio. Mỗi định dạng là một `kind` artifact mới + một loại job mới.

## Phase 5 — Advanced ⬜
Remix, Extend, long-form, reference audio, stem separation — theo capability thật của engine.

## Phase 6 — Multi-Render ⬜
Scheduler đã theo capability và đã test với nhiều node giả. Còn lại: kiểm thử với node thật thứ hai, resource-aware scheduling, kiểm tra tương thích phiên bản worker chi tiết hơn.

## Nợ kỹ thuật đã biết
- Chưa giới hạn tần suất đăng nhập sai (đã có: nginx giới hạn 10 lần/phút mỗi IP).
- Web poll 2 giây; chuyển SSE nếu cần.
- Danh sách chưa phân trang (chỉ `limit`).
- Favorite/rating là thuộc tính chung của track, chưa theo từng người.
- Web chưa lọc LoRA theo `status = approved` (API vẫn từ chối LoRA chưa duyệt); chưa có UI cho Projects.
- `scripts/dev.ps1` mới được kiểm tra cú pháp, chưa chạy trọn vẹn.
- File của job thất bại và track đã xóa mềm chưa có tác vụ dọn dẹp.
