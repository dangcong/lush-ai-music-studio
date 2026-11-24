# CURRENT — Việc đang làm

Cập nhật: 2026-11-24. Đọc file này đầu mỗi phiên. Giữ ngắn: chỉ task đang mở; task xong chuyển sang `COMPLETED.md`.

## Đang làm

### T-007 Reference Audio trong form Tạo nhạc — 🟡 đã deploy, chờ thử trên production (2026-11-13)
- Đã làm: API (`routes/inputs.rs`, migration `0003`, `mode` trong `/generations`), worker (tải input + multipart tới ACE-Step, capability `reference_audio`/`cover`), web (3 chế độ, `AudioDropzone`, hiển thị ở track/job), nginx trong repo, hai hợp đồng, `MUSIC_GENERATION.md`, D19–D20.
- Đã xác minh (máy dev): `cargo test` 12/12, `pytest` 105/105, `npm test` 67/67, clippy/build/lint sạch. Chạy thật: API + worker bản mới ở local, ACE-Step thật qua SSH tunnel → text, reference, cover đều ra WAV 48 kHz stereo; đầu vào WAV/MP3/FLAC; track + metadata vào thư viện. Giao diện đã xem bằng Chrome headless.
- 2026-11-13, theo yêu cầu người dùng: gộp Text và Reference thành **một form** (Reference Audio không bắt buộc), gỡ Cover / Remix khỏi giao diện; API/worker giữ nguyên. `npm test` 65/65, build, lint sạch. Chưa xem lại bằng trình duyệt sau khi gộp.
- Chưa xác minh: production; kéo-thả trên trình duyệt thật; chất lượng nghe.
- 2026-11-13: đã áp nginx trên VPS (người dùng đồng ý), commit `7a481fb`, `/deploy` xong (VPS + Render, migration `0003`). Upload 2 MB chưa đăng nhập qua nginx trả `401` (không còn `413`).
- **Bước tiếp theo:** người dùng thử trên https://suno.darkambient.co (tạo nhạc không file, tạo nhạc có Reference Audio); xong thì chuyển T-007 sang `COMPLETED.md`.

### T-008 AI Training — M1 đã viết và chạy thật trên máy dev, chờ deploy (2026-11-13)
- Thiết kế, phạm vi, kết quả đo: `STYLE_TRAINING.md` (mục 5.1, 12, 13). Hợp đồng: `API_CONTRACTS.md` (AI Training), `WORKER_PROTOCOL.md` (job `train_lora`).
- Đã làm: migration `0004`, `routes/training.rs`, `phase` + `metrics` trong progress, `complete` cho `train_lora`, `PATCH /loras/{id}`; worker `training.py` (Side-Step, dừng/bật inference); web: trang Training, chi tiết run, LoRA Library, LoRA dùng qua Style Preset (không có ô chọn LoRA riêng ở form Tạo nhạc), thông báo node đang train.
- Đã xác minh: `cargo test` 13/13, `pytest` 132/132, `npm test` 132/132, clippy/lint/build sạch. Chạy thật trên GPU của render node (worker bản mới chạy tạm, API + web trên máy dev): một run hoàn tất, tạo nhạc có/không LoRA, hủy giữa chừng. Trang chi tiết run đã xem bằng Chrome headless.
- Chưa xác minh: production; các lỗi OOM/NaN/hết giờ trên GPU thật; chất lượng LoRA; dataset thật.
- Còn trên node sau khi thử: LoRA thử `data/loras/e2e-smoke-lofi-039429c0`, hai thư mục run trong `data/training/`, code worker tạm ở `data/training/m1-dev/`. Xóa khi người dùng đồng ý.
- **Bước tiếp theo:** người dùng thử trên máy dev rồi `/deploy` (có migration `0004`, đổi `lush-worker.service`). Sau deploy chạy một run thật trên production.

### T-009 Overnight full fine-tuning base model qua website — ⏸ dừng, thay bằng T-012 (2026-11-13)
- Người dùng đổi hướng (D28): chưa xây app, web UI hay điều khiển training qua VPS. Ba tài liệu `BASE_MODEL_TRAINING.md`, `TRAINING_SCHEDULER.md`, `TRAINING_TEST_PLAN.md` giữ lại làm tham khảo (phép tính VRAM, định dạng checkpoint, kế hoạch đo F0–F7); phần supervisor / Task Scheduler / API / web / NAS trong đó **không làm**. Chưa từng có code cho T-009.
- Hai mục "Training Model" và "Model Library" (nhãn "Sắp có") ở sidebar web từ T-010 vẫn còn; người dùng chốt 2026-11-13: giữ lại, triển khai sau.

### T-010 Style thay Presets; Training Model / Model Library (chỉ giao diện) — 🟡 xong trên máy dev, chưa commit, chưa deploy (2026-11-13)
- Đã làm (chỉ web, D25): `pages/admin/StylesPage.tsx` thay `PresetsPage.tsx`, route `/admin/styles` (`/admin/presets` chuyển hướng), sidebar thêm Training Model + Model Library ("Sắp có", route `/model-training`, `/model-library`), form Tạo nhạc chỉ còn Style có Train Style.
- Đã xác minh: `npm test` 137/137, lint, build sạch; xem trên trình duyệt với API local: sidebar mới, trang Style rỗng, duyệt Train Style thử thì hiện ở Style với "Cần chọn genre", mở được form Thiết lập. Chưa bấm Lưu trong form bằng trình duyệt (chỉ có test tự động), chưa thử chọn Style ở trang Tạo nhạc với dữ liệu thật.
- **Bước tiếp theo:** người dùng xem và góp ý; Training Model chạy thật phụ thuộc T-009.

### T-011 Khoảng Duration cho Genre / Subgenre / Style, thứ tự ưu tiên, max 300 s — 🟡 đã chạy thật trên máy dev, chưa commit, chưa deploy (2026-11-13)
- Quyết định: D26, D27. Đã làm: migration `0005_genre_duration_range.sql` (genres + style_presets, cho phép NULL), `routes/genres.rs` (`validate_duration_range`), `MAX_DURATION = 300`, hai hợp đồng/tài liệu; web: `durationRangeFor` + nhảy Duration trong `formState.ts`, form Genre và form Style có hai ô "Duration từ / đến", thanh Duration tới 300.
- Đã xác minh: `cargo test` 13/13, clippy sạch, `npm test` 146/146, lint, build sạch. Chạy thật trên trình duyệt với API local (đã khởi động lại, migration `0005` mới đã áp): chọn Lo-fi → 120; kéo 75 rồi chọn subgenre có khoảng 90–150 → 150; subgenre để trống → theo Lo-fi (120); chọn Style 180–300 → 300; kéo 200 rồi bỏ Style → 120; khoảng thiếu một mốc và mốc 301 trả `422`. Dữ liệu thử đã trả về như cũ.
- Chưa xác minh: tạo một bài 241–300 giây trên GPU thật; production.
- Còn lại trên máy dev: file sao lưu `apps/api/data/lush.db.bak-before-0005-rewrite` (xóa được); API local đang chạy nền từ phiên agent.
- **Bước tiếp theo:** người dùng điền khoảng cho từng genre / Style; commit + `/deploy` khi người dùng yêu cầu (có migration `0005`).

### T-012 Workspace `lush-training/` — 🟡 phần không GPU đã viết và test; trainer GPU chưa chạy (2026-11-13)
- Quyết định: D28. Workspace độc lập ở `lush-training/` (mở riêng bằng Claude Code): doctor, soi checkpoint, dataset instrumental, caption, run lifecycle + checkpoint/resume, backend ACE-Step full/partial fine-tuning DiT decoder, đánh giá, export gói inference, và chế độ **autopilot** (một job file do người dùng ủy quyền: tự chuẩn bị dataset, caption theo thư mục, train tới số epoch hoặc giờ chỉ định, chọn checkpoint theo validation loss, xuất gói). Tài liệu và trạng thái chi tiết nằm **trong workspace**: `lush-training/AGENTS.md`, `lush-training/docs/STATUS.md`.
- Đã xác minh (máy dev, không GPU): `pytest` 57 đạt, 1 bỏ qua (cần torch); các script chạy thật trên audio tổng hợp; kiểm checkout ACE-Step `ca1e85f` (chỉ source, lấy từ GitHub) đạt.
- Chưa xác minh: mọi thứ cần GPU và checkpoint thật (backend trainer, preprocess, VRAM, resume trên GPU, model xuất ra được engine nạp); dataset thật; máy RTX 5090.
- Không sửa `apps/`, `workers/`, `infrastructure/`. Chưa commit.
- **Bước tiếp theo:** người dùng mở `lush-training/` trên máy training, chạy doctor; các điều kiện còn thiếu liệt kê ở `lush-training/docs/STATUS.md`.

### T-013 Chuyển website sang server `lushvietnix2` — 🟡 server mới đã chạy, chưa đổi DNS, chưa nối Render Node (2026-11-16)
- Quyết định: D29. Hiện trạng và bảng việc còn lại: `VPS_DEPLOYMENT.md` mục "Server mới".
- Đã làm và đã kiểm tại chỗ trên server: Docker, Tailscale, `lush-api` bản `023dc7a` với database mới, site nginx với chứng chỉ Cloudflare Origin; `/api/health`, đăng nhập admin, chặn `/api/worker/` ở site công khai đều đạt qua nginx + TLS (curl trên chính server). Site sẵn có `app.lushmedia.net` vẫn 200.
- Chưa xác minh: truy cập từ Internet qua Cloudflare; tạo nhạc; `scripts/deploy.sh` chạy trọn với server mới (mới đổi host mặc định, lần cài đầu làm tay). `cargo test` / `npm test` không chạy lại cho lần cài này.
- **Bước tiếp theo (cần người dùng):** (1) Cloudflare: bản ghi `suno` → `14.225.220.26`, bật Proxied, SSL mode Full (strict); (2) ~~mở cổng 8081 trên `tailscale0`~~ xong 2026-11-16; (3) đưa máy render online / vào tailnet, rồi tạo node mới và đổi `worker.env`. Sau đó kiểm tra từ Internet và tạo một bài thật. VPS cũ giữ nguyên (còn chạy site khác).
- 2026-11-23: bước (3) xong bằng máy mới `rtx6000` thay máy render cũ (`RENDER_NODE_DEPLOYMENT.md`): node Online, một bài 60 s tạo thật qua API trên server. Bước (1) cũng đã xong: từ máy dev, `suno.darkambient.co` đi qua Cloudflare proxy tới `lushvietnix2` (log nginx trên server khớp), `/api/health` 200, đăng nhập 200, `/api/worker/` bị chặn 403. Chưa thử tạo nhạc bằng trình duyệt.

### T-014 Dataset Factory + Train Style Lo-fi (pilot) — 🟡 code xong trên nhánh `feat/lofi-dataset-factory`, chưa chạy trên rtx6000 (2026-11-23)
- Thiết kế: `docs/superpowers/specs/2026-11-23-lofi-style-lora-design.md`. Kế hoạch: `docs/superpowers/plans/2026-11-23-lofi-style-lora.md`. Quyết định và tech stack: D30. Sổ thực thi (rulings, kết quả từng task): `.superpowers/sdd/2026-11-23-lofi-style-lora/progress.md` (git-ignored).
- Đã làm (chỉ `lush-training/`, không đụng `apps/`, `workers/`):
  - bộ xuất dataset cho Train Style
  - BPM/key có độ tin cậy
  - adapter caption (`AceStepCaptioner` chưa nối: chờ duyệt tải model)
  - trang duyệt caption (localhost + SSH tunnel)
  - loại cả bài có giọng; build lại giữ split cũ (`--previous`), carry-over caption
  - báo cáo pilot
  - render A/B qua `acestep-api` (cần `--confirm-gpu`)
  - `scripts/install_rtx6000.sh`
- Đã xác minh (máy dev, Windows, Python 3.14):
  - `pytest` 106 đạt, 2 bỏ qua.
  - Trang review bấm thử bằng trình duyệt trên dataset tổng hợp: duyệt, ghi log, lý do "cần nghe kỹ", từ chối duyệt caption có từ vocal, tua audio.
  - Review toàn nhánh bởi reviewer độc lập; 5 lỗi Important + 5 Minor đã sửa, có test tái hiện.
- Chưa xác minh: mọi thứ trên rtx6000 (cài đặt, MP3 thật, captioner, worker Studio đọc bản xuất, train, render A/B thật); ngưỡng BPM/key trên nhạc thật; nghe audio thật trên trang review.
- Minor để sau: evidence "human" lặp khi lưu nhiều lần; POST không kiểm Content-Type/Origin; script cài không báo tham số lạ; chưa có fixture MP3 VBR / có ảnh bìa.
- **Bước tiếp theo (cần người dùng duyệt từng bước):**
  1. Cài factory lên rtx6000 (Task 6).
  2. Chọn 10–20 bài pilot trên máy dev.
  3. Duyệt tải `acestep-captioner` sau khi được báo dung lượng và giấy phép (Task 7).
  4. ~~Duyệt caption trên trang review (Task 8).~~ — thay bằng trang Catalog của T-015.
  5. Chọn khung giờ train và render A/B (Task 9–10).

### T-016 LoRA Studio P2: Tạo LoRA từ Catalog — 🟡 code xong trên nhánh `feat/lora-studio-web`, chỉ test tự động (mock), chưa deploy (2026-11-24)
- Nhánh `feat/lora-studio-web`, commit `26f6a8a..f49e24d` (cộng commit tài liệu ngay sau). Kế hoạch: `docs/superpowers/plans/2026-11-24-lora-studio-p2-create-lora.md`. Quyết định: D32. Thiết kế: `docs/superpowers/specs/2026-11-23-lora-studio-web-design.md`.
- Đã làm:
  - API: xem trước (preview) và tạo LoRA từ Catalog với bản chụp dataset bất biến (migration `0007`, trigger SQLite); cổng claim theo `snapshot_id`; khi train xong ghi `lora_checkpoints` (đường dẫn node chỉ nằm trong bảng, bị cắt khỏi `jobs.result`); Trainer xem được các run train.
  - Worker: dựng dataset Side-Step từ bản chụp (ffmpeg, WAV 48 kHz) trước khi dừng inference; báo mọi checkpoint `{epoch, step, path, sha256 của adapter_model.safetensors, bytes}`.
  - Web: hộp thoại Tạo LoRA đúng mockup đã duyệt; trang run có "LoRA này được train từ" và "Bản lưu" (có đường lùi cho run cũ); Sidebar Train Style hiện với Trainer.
- Đã chạy thật (máy dev, chỉ test tự động và mock), 2026-11-24:
  - `cd apps/api; cargo test`: 43 đạt, 0 lỗi (42 integration + 1 unit). `cargo clippy --all-targets`: không cảnh báo.
  - `cd workers/music-worker; .venv/Scripts/python -m pytest -q`: 178 đạt, 1 warning (librosa).
  - `cd apps/web; npm test`: 219 đạt (20 file). `npm run build`: thành công (chỉ cảnh báo chunk > 500 kB).
- **Chưa** chạy thật:
  - Chưa train trên rtx6000; chưa chạy ACE-Step / Side-Step thật với dataset dựng từ bản chụp.
  - Chưa kiểm ffmpeg có trên rtx6000.
  - Chưa đo VRAM và thời gian của run "Thử nhanh".
  - Migration `0006` và `0007` chưa chạy trên database thật.
  - Giao diện chưa xem bằng trình duyệt với API thật.
  - Khi deploy: cập nhật worker cùng lúc hoặc trước API (worker cũ trước P2 làm job snapshot thất bại an toàn, trước khi dừng inference).
  - WAV đã chuyển và tensor cache dưới `datasets/<lora_key>` trên node chưa có cơ chế dọn (để P3).
  - Run bị hủy tay chưa giữ checkpoint nào (chỉ dừng do hết giới hạn thời gian mới giữ); để P3.
- **Bước tiếp theo (cần người dùng duyệt từng bước):**
  1. Deploy API + web (migration `0006` và `0007`).
  2. Trên rtx6000: `pip install -e ".[analysis]"`, đặt `CAPTIONER_DIR`, kiểm `ffmpeg`, khởi động lại worker.
  3. Duyệt tải MOSS-Music (báo dung lượng và giấy phép trước).
  4. Chạy thật một run "Thử nhanh" trên rtx6000 theo nghiệm thu 9.2 của spec, ghi VRAM và thời gian.
- Để lại cho P3: nghe thử / chấm điểm (eval), chọn bản, xóa bản (hạn 30 ngày), Style Preset. P3 phải đọc `node_path` từ bảng `lora_checkpoints`.
- Còn tồn từ P1 (không chặn): job catalog thất bại vẫn hiện nút "Thử lại" nhưng API trả `409`.
- Tồn khác: các Minor trong sổ `.superpowers/sdd/2026-11-24-lora-studio-p2-create-lora/progress.md` (git-ignored).

### T-015 LoRA Studio P1: Catalog trên website — 🟡 code xong trên nhánh `feat/lora-studio-web`, chỉ test tự động (mock), chưa deploy (2026-11-24)
- Thiết kế: `docs/superpowers/specs/2026-11-23-lora-studio-web-design.md`. Kế hoạch: `docs/superpowers/plans/2026-11-23-lora-studio-p1-catalog.md`. Quyết định: D31. Hợp đồng: `API_CONTRACTS.md`, `WORKER_PROTOCOL.md`.
- Đã làm:
  - API: migration `0006` (`catalogs`, `catalog_tracks`, `caption_revisions`, `caption_heads`, `models.kind`), role Trainer, endpoint catalog / track / caption thủ công / undo / redo / reanalyze / `caption-jobs`.
  - Worker: job `analyze_track` (librosa, extra `analysis`) và `caption_tracks` (tiến trình con theo model, thư mục `CAPTIONER_DIR`); script captioner MOSS-Music (`captioner.json` + script).
  - Web: trang `/datasets` và `/catalogs/:id` (upload, chọn bài, tạo mô tả, sửa tại chỗ, undo/redo, cột tech stack); form Tạo nhạc / Style / Training chỉ liệt kê model sinh nhạc; cột "Loại" ở trang Models.
- Đã xác minh (máy dev, chỉ test tự động và mock): số test cuối P1 (HEAD 5ca2bde) là `cargo test` 33 + 1 unit, `pytest` 161, `npm test` 181 (18 file); số đo lại ngày 2026-11-24 trên nhánh sau P2 (gồm cả P1) nằm ở T-016: `cargo test` 43, `pytest` 178, `npm test` 219 (20 file), clippy và build sạch. Mỗi task đã qua review độc lập.
- Chưa xác minh:
  - Chạy thật trên rtx6000, kể cả migration `0006` trên database thật.
  - librosa trên node (tempo/key/loudness trên nhạc thật; test dùng tín hiệu tổng hợp).
  - MOSS-Music thật: protocol đã test bằng script giả; code nạp model và generate viết theo `infer.py` diễn giải, ghi "chưa kiểm chứng", chưa tải model.
  - Giao diện chưa xem bằng trình duyệt với API thật sau Task 9–10.
- Quan hệ với T-014: phần giao diện duyệt caption của T-014 (trang review localhost + SSH tunnel) được **thay bằng** Catalog trên website của T-015. Phần CLI factory còn lại của T-014 (export dataset, BPM/key, train, render A/B) giữ nguyên.
- Minor để sau: xem các dòng "minor (deferred)" trong sổ `.superpowers/sdd/2026-11-23-lora-studio-p1-catalog/progress.md` (git-ignored).
- **Bước tiếp theo (cần người dùng duyệt từng bước):**
  1. Deploy API + web lên server (có migration `0006`).
  2. Đặt `CAPTIONER_DIR` trong `worker.env` của rtx6000 và chạy `pip install -e ".[analysis]"` trong venv worker (kèm khởi động lại worker (tự đăng ký lại) để nhận job type mới).
  3. Duyệt tải MOSS-Music (báo dung lượng và giấy phép trước), kiểm chứng code nạp model, rồi chạy thử một catalog nhỏ.
- Còn tồn (không chặn):
  - Lỗi mô tả theo từng bài chỉ nằm trong log của job (xem ở trang Hàng đợi).
  - Job `caption_tracks` đang chạy thì chặn tạo nhạc trên node đó.
  - Thời gian nạp model tính vào timeout của bài đầu tiên.
  - Hai `POST caption-jobs` đồng thời có thể trả `500`.
  - Ô tempo dùng `Number()` nên chấp nhận chuỗi lỏng (vd. `1e2`).
  - Code nạp model MOSS chưa kiểm chứng trên GPU thật.

## Chờ người dùng

Danh sách đầy đủ ở `ROADMAP.md` (mục "Việc cần người quyết định"). Mọi bước deploy, push, tải model đều cần phê duyệt.

## Tiếp theo

1. Người dùng nghe thử, đánh giá chất lượng theo genre; điều chỉnh prompt template của genre nếu cần.
2. ~~Thử khởi động lại máy render~~ — xong 2026-11-13: NVMe tự mount, hai dịch vụ tự chạy.
3. Benchmark đầy đủ hơn (180–240 s, nhiều genre) — `TESTING.md`.
4. Phase 3 (dataset, LoRA training) — dữ liệu và checkpoint đặt ở `/mnt/ai-studio/AiStudio/data/`.
