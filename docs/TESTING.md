# TESTING

## Chạy test

| Phần | Lệnh (trong thư mục tương ứng) | Phạm vi |
|---|---|---|
| `apps/api` | `cargo test` | Integration test trên router thật + SQLite tạm; test đóng vai worker qua HTTP |
| `workers/music-worker` | `.venv\Scripts\python -m pytest -q` | Runner, mock engine, adapter ACE-Step (HTTP giả bằng respx) |
| `apps/web` | `npm test` | Lớp API, form tạo nhạc, job card, phân quyền menu, player |

CI (`.github/workflows/ci.yml`) chạy cả ba trên mỗi push và pull request.

## Backend test bao phủ gì (`apps/api/tests/api.rs`)

- Đăng nhập, phiên, đăng xuất, phân quyền admin, quản lý user, khóa admin cuối cùng.
- Genre Registry: seed, thêm genre/subgenre bằng API, validation, tắt genre, preset.
- Luồng đầy đủ: tạo job → claim → progress → upload (checksum sai, sai định dạng, sai lease) → complete → track + metadata → tải file + Range → quyền sửa/xóa.
- Idempotency (double submit), một job mỗi node, validation request.
- Hủy job đang chờ và đang chạy; retry.
- Worker fail có/không retry, hết lượt thử, release.
- Node offline: lease hết → requeue → fail khi hết lượt; node khởi động lại; heartbeat giữ lease; hủy job của worker đã chết.
- Scheduling theo capability với nhiều node, bảo trì, vô hiệu hóa, đổi token, sai phiên bản protocol.
- 6 node claim đồng thời 3 job: không job nào bị giao hai lần.

## Mức độ kiểm chứng — đọc kỹ

| Hạng mục | Mức |
|---|---|
| API + worker + mock engine chạy thật qua HTTP trên Windows | ✅ Đã chạy (2026-11-10) |
| Web chạy với backend thật trên Chrome (đăng nhập, Generate, tiến độ job, chi tiết track + waveform, Render Nodes) | ✅ Đã chạy (2026-11-10) |
| Adapter ACE-Step với GPU thật (RTX 3060, `acestep-v15-turbo`, 8 bước) | ✅ Đã chạy (2026-11-10) |
| MP3 preview bằng ffmpeg trên Render Node | ✅ Đã chạy (2026-11-10) |
| API + web trên VPS thật (qua SSH tunnel), worker mock từ máy Windows, node offline | ✅ Đã chạy (2026-11-10) |
| HTTPS công khai https://studio.darkambient.co: trang web, đăng nhập, cookie Secure, chặn worker API | ✅ Đã chạy (2026-11-10) |
| Render Node Ubuntu ↔ VPS qua Tailscale, job thật từ website công khai | ✅ Đã chạy (2026-11-10) |
| Khởi động lại máy render (dịch vụ tự lên, ổ NVMe tự mount) | ❌ Chưa thử |
| Chất lượng nhạc nghe bằng tai theo genre | ❌ Chưa |
| Chất lượng nhạc AI, benchmark GPU | ❌ Chưa (cần Render Node) |

Không ghi "đã kiểm thử" cho hạng mục mới chỉ chạy với mock.

## Kiểm thử chất lượng AI (khi có Render Node)

Bộ prompt cố định cho mỗi genre × seed cố định; đo thời gian sinh, VRAM đỉnh, duration thật so với yêu cầu; nghe đánh giá theo thang trong brief (chất lượng, đúng genre, bám prompt, cấu trúc). Kết quả ghi vào tài liệu riêng kèm ngày, model, phiên bản ACE-Step.

## Kết quả đo thật

| Ngày | Model | Thời lượng | Thời gian ACE-Step | Bấm Generate → có trong thư viện | VRAM đỉnh | Ghi chú |
|---|---|---|---|---|---|---|
| 2026-11-10 | `acestep-v15-turbo`, 8 bước, không LM | 30 s | ~6 s | 17 s | — | WAV 48 kHz stereo 16-bit, RMS −15 dBFS (không im lặng), seed 20261110 |
| 2026-11-10 | `acestep-v15-turbo`, 8 bước, không LM | 120 s | 10 s | 56 s | 6,3 GB / 12 GB | Phần lớn thời gian còn lại là upload WAV ~23 MB lên VPS qua Tailscale |

Mới đo một lần mỗi điểm; chưa phải benchmark đầy đủ.
