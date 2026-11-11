# COMPLETED — Task đã xong

Mới nhất ở trên. Chỉ ghi task đã **xác minh** (ghi rõ "mock" hay "thật"). Mỗi mục: kết quả, bằng chứng, còn tồn.

## T-006 Track card: hàng nút tải MP3 · WAV · MIDI · XML · PDF · ZIP thay 5 sao — 2026-11-11
- Kết quả: danh sách bài nhạc bỏ đánh giá sao và icon tải đơn. Thay bằng 6 pill (`apps/web/src/components/ExportActions.tsx`, cấu hình `EXPORT_FORMATS`), giữ nút tim. Trang chi tiết track vẫn còn đánh giá sao. API thêm `GET /tracks/{id}/archive`: zip các file đang có (stored), ghi vào file tạm ẩn danh trong `data_dir/tmp` rồi stream (dependency mới: `zip`, `tokio-util`; `tempfile` chuyển từ dev-dependency sang dependency chính).
- Bằng chứng (máy dev): `cargo test` 10/10, có kiểm tra zip giải nén ra đúng từng byte WAV, 401/404, track đã xóa → 404; `cargo clippy` sạch. Web: `npm test` 47/47, build và lint sạch. Chưa xem giao diện trên trình duyệt, chưa deploy.
- Còn tồn: backend chưa có pipeline MIDI/MusicXML/PDF, nên cả 3 nút đang ở trạng thái "chưa khả dụng". Nút tự bật khi track có artifact kind `midi` / `musicxml` / `score_pdf`. Phase 4 cần thêm các kind này vào `API_CONTRACTS.md`, `ArtifactKind` và `artifact_type()` trong `worker.rs`.

## T-005 Render Node thật + ACE-Step + job tạo nhạc đầu tiên — 2026-11-10
- Kết quả: `render-node-01` (`ssh render`) chạy `acestep-api` + `lush-worker` bằng systemd user unit, toàn bộ dưới `/mnt/ai-studio/AiStudio` (NVMe 1,8 TB, mount bằng fstab `nofail`). Worker gọi VPS qua Tailscale `http://100.113.164.7:8081`. ffmpeg đã cài. Forge đã gỡ (ảnh cũ ở `~/archive/`). Commit `e1c48a1`, `83f7938`.
- Bằng chứng (thật): job từ https://studio.darkambient.co → `completed`; WAV 48 kHz stereo 30,00 s, RMS −15 dBFS; MP3 preview; tải được qua website. Bài 120 s: ACE-Step 10 s, tổng 56 s, VRAM đỉnh 6,3 GB (`TESTING.md`).
- Còn tồn: chưa thử khởi động lại máy render; chưa nghe đánh giá chất lượng; upload WAV qua Tailscale chậm (~0,5 MB/s) — cân nhắc nếu bài dài; node thử `qa-windows-mock` và model `mock-v1` vẫn còn trong registry (đã tắt node).

## T-004 Bộ nhớ dự án — 2026-11-10
- Kết quả: thêm `MEMORY_INDEX.md`, `CHANGELOG.md`, `tasks/CURRENT.md`, `tasks/COMPLETED.md`; quy trình đọc/ghi bộ nhớ trong `AGENTS.md`.
- Bằng chứng: chỉ đổi tài liệu, không đổi source code.
- Còn tồn: không.

## T-003 Website công khai https://studio.darkambient.co — 2026-11-10
- Kết quả: nginx + Let's Encrypt, DNS-only (D16, D17). Commit `27c5668`.
- Bằng chứng (thật): trang web, đăng nhập, cookie Secure, chặn worker API từ Internet (`TESTING.md`).
- Còn tồn: chưa có Render Node nên chưa sinh nhạc thật trên production.

## T-002 Triển khai control plane lên VPS — 2026-11-10
- Kết quả: systemd `lush-api` trên `127.0.0.1:8018`, dữ liệu `/var/lib/lush`. Commit `ed3b7c4`, `1f7d8e5`.
- Bằng chứng (thật): API + web trên VPS qua SSH tunnel, worker mock từ Windows, node offline.
- Còn tồn: xem `VPS_DEPLOYMENT.md`.

## T-001 Phase 0–1 Core Platform — 2026-11-10
- Kết quả: API Rust, worker Python + Mock Engine, web React, hai hợp đồng, CI. Commit `cdcd750`.
- Bằng chứng: `cargo test`, `pytest`, `npm test`; chạy thật trên Windows + Chrome **với mock engine**.
- Còn tồn: adapter ACE-Step chỉ test bằng HTTP giả; một số trang quản trị chưa bấm thử; nợ kỹ thuật ở `ROADMAP.md`.
