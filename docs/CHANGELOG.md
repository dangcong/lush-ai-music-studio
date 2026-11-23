# CHANGELOG

Thay đổi của sản phẩm theo ngày, mới nhất ở trên. Chi tiết kiểm thử nằm ở `tasks/COMPLETED.md`.

## Chưa phát hành

- **LoRA Studio P1, Catalog** (D31, T-015): trang Datasets / Catalog trên website: upload bài, tự phân tích tempo/key/loudness (job `analyze_track`, librosa), tạo mô tả bằng model captioner (job `caption_tracks`, tiến trình con theo `CAPTIONER_DIR`), sửa tay có undo/redo, lưu nguồn từng mô tả. Model Registry có `kind` (generator / captioner), form Tạo nhạc / Style / Training chỉ liệt kê model sinh nhạc. Migration `0006`, role Trainer. Thay trang duyệt localhost của T-014. *Chỉ test tự động với mock; chưa deploy, MOSS-Music chưa tải.*
- **Hạ tầng (D29):** website đang chuyển sang server `lushvietnix2` sau Cloudflare proxy, database mới. `scripts/deploy.sh` mặc định deploy vào server mới; cấu hình nginx trong repo viết lại cho server đó. Chờ đổi DNS và nối lại Render Node.
- **Workspace `lush-training/`** (D28): công cụ độc lập, mở riêng bằng Claude Code, để chuẩn bị dataset instrumental, caption, fine-tune checkpoint ACE-Step, đánh giá và xuất gói inference. Thay phương án train qua website (T-009). Phần không cần GPU có test; trainer trên GPU chưa chạy. Không đổi gì ở API, web, worker.
- **Khoảng Duration gợi ý** cho Genre, Subgenre và Style (hai ô tự điền "từ / đến", 10–300 giây, có thể để trống) thay cho một số duy nhất (D26, D27). Ưu tiên Style → Subgenre → Genre. Ở trang Tạo nhạc, mỗi lần chọn hoặc đổi Genre / Subgenre / Style thì Duration nhảy về mốc "đến" của khoảng đang áp dụng, kể cả khi người dùng đã kéo trước đó; sau đó vẫn kéo tự do. Duration tối đa nâng từ 240 lên **300 giây**. Migration `0005`: genre và Style cũ nhận cả hai mốc bằng giá trị cũ.
- Web: trang **Style** (Quản trị) thay trang Presets (D25): liệt kê các Train Style đã duyệt, admin chọn genre và tham số cho từng Style; không còn tạo preset bằng tay, preset không gắn Train Style không còn hiện ở giao diện (dữ liệu giữ nguyên). Form Tạo nhạc đổi "Style Preset" thành "Style". Thêm hai mục **Training Model** và **Model Library** ở dạng "Sắp có" (chỉ giao diện, chưa có API/worker). Không đổi API, không có migration.
- **Train Style (M1)** trong Admin — tên gọi trên giao diện của việc train LoRA (D23): trang Train Style (tạo run, danh sách, chi tiết có biểu đồ loss thật, log, hủy) và Train Style Library (duyệt, lưu trữ). Run là job `train_lora` trên hàng đợi chung; worker dừng tạo nhạc trong lúc train rồi tự bật lại; job tạo nhạc đang chờ luôn chạy trước một run mới. LoRA train xong ở trạng thái `evaluation` (admin nghe thử được), duyệt rồi mới dùng chung. LoRA được dùng qua Style Preset (preset loại “trained”); người dùng chỉ thấy preset khi LoRA của nó đã được duyệt, admin thấy thêm preset có LoRA đang đánh giá để nghe thử. Migration `0004`. *Đã chạy thật trên máy dev với GPU của render node; chưa deploy. Dataset vẫn là thư mục có sẵn trên node, chưa có trang Dataset.*

- Docs: `STYLE_TRAINING.md` — audit và thiết kế AI Training (M0); chưa có code training.
- Render node: reboot 2026-11-13, driver NVIDIA khớp lại (595.99.02), `nvidia-smi` và số liệu GPU hoạt động trở lại.
- AI Training: smoke test E1 chạy thật trên RTX 3060 (LoRA bằng Side-Step: train, resume, nạp adapter vào inference) — kết quả ở `STYLE_TRAINING.md` mục 5.1. Phạm vi chốt: chỉ instrumental (D22). `WORKER_PROTOCOL.md` tách phần đề xuất cho training khỏi hợp đồng đang chạy.
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
