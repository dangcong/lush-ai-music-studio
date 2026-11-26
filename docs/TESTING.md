# TESTING

## Chạy test

| Phần | Lệnh (trong thư mục tương ứng) | Phạm vi |
|---|---|---|
| `apps/api` | `cargo test` | Integration test trên router thật + SQLite tạm; test đóng vai worker qua HTTP |
| `lush-training` | `.venv\Scripts\python -m pytest -q` (trong `lush-training/`) | Dataset, caption, vòng đời run, checkpoint/resume, export; backend giả, không GPU. Mức kiểm chứng: `lush-training/docs/STATUS.md` |
| `workers/music-worker` | `.venv\Scripts\python -m pytest -q` | Runner, mock engine, adapter ACE-Step (HTTP giả bằng respx) |
| `apps/web` | `npm test` | Lớp API, form tạo nhạc, job card, phân quyền menu, player |

CI (`.github/workflows/ci.yml`) chạy `apps/api`, `workers/music-worker` và `apps/web` trên mỗi push và pull request; `lush-training` chưa nằm trong CI.

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
- AI Training: chỉ admin tạo run; validation; một run tại một thời điểm; job generate đang chờ được claim trước run; `phase` + `metrics` (không trùng `sequence`); `complete` cần checksum adapter và tạo LoRA `evaluation`; admin dùng thử LoRA `evaluation`, duyệt rồi mọi người dùng được; không retry run; danh sách dataset theo node.

## Mức độ kiểm chứng — đọc kỹ

| Hạng mục | Mức |
|---|---|
| API + worker + mock engine chạy thật qua HTTP trên Windows | ✅ Đã chạy (2026-11-10) |
| Web chạy với backend thật trên Chrome (đăng nhập, Generate, tiến độ job, chi tiết track + waveform, Render Nodes) | ✅ Đã chạy (2026-11-10) |
| Adapter ACE-Step với GPU thật (RTX 3060, `acestep-v15-turbo`, 8 bước) | ✅ Đã chạy (2026-11-10) |
| MP3 preview bằng ffmpeg trên Render Node | ✅ Đã chạy (2026-11-10) |
| Reference to Music và Cover / Remix: API + worker bản mới chạy trên máy dev, nối ACE-Step thật của `render-node-01` qua SSH tunnel; đầu vào WAV, MP3, FLAC | ✅ Đã chạy (2026-11-12): audio thật 48 kHz stereo, track + metadata vào thư viện; cùng prompt/seed có và không có Reference Audio cho kết quả khác nhau |
| Reference / Cover trên production (suno.darkambient.co), upload qua nginx | ❌ Chưa (chờ deploy + sửa nginx) |
| Kéo-thả upload trên trình duyệt thật | ⚠️ Chỉ có test tự động (vitest); giao diện 3 chế độ đã xem bằng Chrome headless |
| Chất lượng âm nhạc của Reference / Cover (nghe đánh giá) | ❌ Chưa |
| API + web trên VPS thật (qua SSH tunnel), worker mock từ máy Windows, node offline | ✅ Đã chạy (2026-11-10) |
| HTTPS công khai https://suno.darkambient.co: trang web, đăng nhập, chặn worker API; tên cũ chuyển hướng 301 | ✅ Đã chạy (2026-11-11, bằng curl) |
| Render Node Ubuntu ↔ VPS qua Tailscale, job thật từ website công khai | ✅ Đã chạy (2026-11-10) |
| Khởi động lại máy render (dịch vụ tự lên, ổ NVMe tự mount) | ✅ Đã chạy (2026-11-13) |
| Train Style LoRA (M1) từ API: API + web bản mới trên máy dev, worker bản mới chạy tạm trên `render-node-01` với GPU thật | ✅ Đã chạy (2026-11-13): run 4 epoch / 8 optimizer step hoàn tất, metrics thật về API, LoRA vào Registry ở `evaluation`, inference tự bật lại; tạo nhạc cùng prompt/seed có và không có LoRA cho kết quả khác nhau; hủy giữa chừng → `cancelled`, inference bật lại, không còn tiến trình trainer |
| Train Style LoRA trên production (worker production, website công khai) | ❌ Chưa (chờ deploy) |
| Training: OOM, loss NaN, hết giờ, worker bị tắt, inference không lên lại | ⚠️ Chỉ có test tự động với trainer giả |
| Chất lượng Style LoRA (nghe, vocal contamination), dataset thật | ❌ Chưa |
| Chất lượng nhạc nghe bằng tai theo genre | ❌ Chưa |
| Chất lượng nhạc AI, benchmark GPU | ❌ Chưa (cần Render Node) |

Không ghi "đã kiểm thử" cho hạng mục mới chỉ chạy với mock.

## Kiểm thử chất lượng AI (khi có Render Node)

Bộ prompt cố định cho mỗi genre × seed cố định; đo thời gian sinh, VRAM đỉnh, duration thật so với yêu cầu; nghe đánh giá theo thang trong brief (chất lượng, đúng genre, bám prompt, cấu trúc). Kết quả ghi vào tài liệu riêng kèm ngày, model, phiên bản ACE-Step.

## Kết quả đo thật

### 2026-11-25 — rtx6000, ACE-Step XL SFT + LM 4B (D36) [Đo]
- Máy: RTX PRO 6000 Blackwell 96 GB, ACE-Step `ca1e85f`. `acestep-v15-xl-sft` (4 shard, 19,95 GB) và `acestep-5Hz-lm-4B` (8,38 GB) tải từ Hugging Face; `ACESTEP_INIT_LLM=true`; LM chạy backend PyTorch (fallback).
- VRAM sau khi nạp XL + LM: 28,1 GB.
- text2music 30 s, 50 bước, guidance 7, seed 1234, gọi thẳng `/release_task` (không qua website): `shift` mặc định REST (3.0) 6,3 s; `shift` 1.0 4,0 s. Có WAV, **chưa nghe**.
- Khi LM đã nạp, ACE-Step luôn chạy LM "CoT metas" (~1–1,5 s) để điền BPM/key/nhịp còn trống, kể cả `thinking: false`.
- Chạy tuần tự trực tiếp ACE-Step → MOSS → Flamingo → ACE-Step: đều xong (6,1 s / 5 s / 7 s / 6,3 s), đỉnh VRAM 38,9 GB, không lỗi lần hai. Thinking bật 6,1 s (LM 4B). `full_analysis_only` 2,0 s trả `prompt`, `bpm`, `keyscale` (một mẫu: bài tạo ở 80 BPM, LM đo 136).
- Lỗi tái hiện trước bản `d8adf8d`: cùng seed, để trống BPM/Key/Nhịp → LM chọn ngẫu nhiên, 3 file khác sha256. Model yêu cầu không nạp (`acestep-v15-turbo`) → ACE-Step lặng lẽ chạy XL.
- Website sau deploy `d8adf8d` (tài khoản admin, trình duyệt thật): bài "Test ghim A" để trống seed/BPM/Key/Nhịp → bài lưu seed 1735397042, 79 BPM, G# minor, 4/4; "Tạo lại với thiết lập này" điền đúng các giá trị đó; bài "Test ghim B" ra **cùng sha256** `5fd304bf…` với A (hai task ACE-Step khác nhau). Mỗi bài 30 s: 50 bước, 3,4–4,3 s.
- LoRA trên XL (website, run kiểu cũ từ dataset trên node `relax-sleep-20/v1`: 20 bài ngẫu nhiên từ thư mục test "Relax + Sleep", caption bằng `acestep-captioner`, tempo/key đo tự động). Run 1 hỏng vì trainer `ca1e85f` không nhận alias `xl_sft` → sửa ở `8bca75d`. Run 2 (rank 64, alpha 128, lr 1e-4, 30 epoch, 150 step): 5:20, VRAM đỉnh 12,8 GB, loss 1,02 → 0,57, adapter 160 MB nạp được. Run 3 (lr 2e-4, 100 epoch, 500 step, warmup 10): ~18 phút, bản lưu epoch 25/50/75/100. Cả hai: inference tự bật lại, duyệt → Style (Chill Lo-fi, scale 1).
- Tạo nhạc qua website, cùng prompt, seed 777, 75 BPM, C major, 4/4, 60 s, `dit_model` = XL: không LoRA (B) và chạy lại không LoRA (D) **cùng sha256** `49e139ab…` (không sót adapter cũ). Run 2 (A) so với B: tương quan 0,98, phần khác −14 dB → LoRA có tác dụng nhưng rất nhẹ. Run 3 (C) so với B: tương quan 0,83, phần khác −4,9 dB → khác rõ. **Chưa nghe** để biết có giống chất dataset hơn không.
- Chưa đo: Thinking và `acestep-lm` qua website; eval tự động cho run kiểu cũ.

### 2026-11-26 — LoRA Lofi Hip Hop XL, 95 bài (run kiểu cũ, `lofi-hiphop-100/v1`) [Đo]
- Dataset: 100 bài ngẫu nhiên từ thư mục "Lofi hip hop" (272 bài); 5 bài giữ riêng làm chuẩn chấm, 95 bài train (~3 giờ 54 phút nhạc), caption `acestep-captioner`, tempo/key đo tự động.
- Run: rank 64, alpha 128, lr 1e-4, 50 epoch (1.200 step), warmup 10, lưu mỗi 10 epoch. Khoảng 40 phút kể cả tiền xử lý. Loss epoch 10/20/30/40/50: 0,569 / 0,575 / 0,568 / 0,568 / 0,557. Adapter cuối trùng sha256 với epoch 50.
- Không thử được từng epoch qua website: `/loras/{id}/try` và `/publish` đòi `genre_id`, LoRA kiểu cũ có `genre_id` null và API không có chỗ đặt.
- Chấm bằng MERT (cosine tới tâm 5 bài giữ riêng) + Audiobox, 5 prompt (caption của bài giữ riêng) × seed cố định, 60 s. Để LM chọn BPM/key: cùng seed nhưng BPM/key khác nhau giữa có/không LoRA, nên so sánh bị nhiễu. Ghim 85 BPM, A minor: không LoRA 0,9420 / 7,20; scale 0,6 0,9420 / 7,11; scale 1,0 0,9445 / 7,12. Chênh lệch nhỏ, chỉ 5 mẫu.
- Bài 2:45 (scale 1,0, 82 BPM, 3 seed): seed 24601 điểm cao nhất (0,976 / 7,41). **Chưa nghe** bằng tai.

| Ngày | Model | Thời lượng | Thời gian ACE-Step | Bấm Generate → có trong thư viện | VRAM đỉnh | Ghi chú |
|---|---|---|---|---|---|---|
| 2026-11-10 | `acestep-v15-turbo`, 8 bước, không LM | 30 s | ~6 s | 17 s | — | WAV 48 kHz stereo 16-bit, RMS −15 dBFS (không im lặng), seed 20261110 |
| 2026-11-10 | `acestep-v15-turbo`, 8 bước, không LM | 120 s | 10 s | 56 s | 6,3 GB / 12 GB | Phần lớn thời gian còn lại là upload WAV ~23 MB lên VPS qua Tailscale |

Mới đo một lần mỗi điểm; chưa phải benchmark đầy đủ.
