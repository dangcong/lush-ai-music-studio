# MEMORY_INDEX — Bản đồ bộ nhớ dự án

Không đọc hết `docs/`. Luôn đọc `tasks/CURRENT.md`, rồi chỉ mở file khớp với task.

## Mỗi file giữ gì (một nguồn sự thật, không chép sang file khác)

| File | Giữ | Cập nhật khi |
|---|---|---|
| `tasks/CURRENT.md` | Task đang làm, bước tiếp theo, việc chờ người dùng | Bắt đầu / dừng / xong một task |
| `tasks/COMPLETED.md` | Task đã xong + bằng chứng kiểm thử + việc còn tồn | Task xong **và** đã xác minh |
| `CHANGELOG.md` | Thay đổi của sản phẩm theo ngày | Có thay đổi code/hạ tầng/tài liệu đáng kể |
| `ROADMAP.md` | Trạng thái theo phase, nợ kỹ thuật | Trạng thái một hạng mục phase đổi |
| `TESTING.md` | Lệnh test, mức độ kiểm chứng (mock / thật) | Mức kiểm chứng của một hạng mục đổi |
| `DECISIONS.md` | Quyết định kiến trúc (D1, D2…), chỉ thêm cuối | Có quyết định mới hoặc thay quyết định cũ |

## Đọc theo loại task

| Task chạm vào | Đọc |
|---|---|
| Web ↔ API (endpoint, payload) | `API_CONTRACTS.md` |
| Worker ↔ API, engine adapter | `WORKER_PROTOCOL.md`, `MUSIC_GENERATION.md` |
| Database, migration | `DATABASE.md` |
| Thể loại, preset | `GENRE_SYSTEM.md` |
| Kiến trúc tổng thể, thành phần | `ARCHITECTURE.md`, `DECISIONS.md` |
| Phát hành bản mới (`/deploy`) | `DEPLOY.md` |
| Triển khai VPS | `VPS_DEPLOYMENT.md` |
| Triển khai Render Node, ACE-Step | `RENDER_NODE_DEPLOYMENT.md` |
| Dataset, LoRA training, AI Training trong Admin | `LORA_TRAINING.md` |
| Code cũ cần port (Phase 3) | `MODULES.md` |
| Phạm vi, mục tiêu sản phẩm | `PROJECT_BRIEF.md` |
| Lên kế hoạch phase mới | `ROADMAP.md` |

## Không lưu trong Markdown

Mật khẩu, token (`NODE_TOKEN`…), API key, nội dung file `.env`, private key, cookie phiên. Chỉ ghi **vị trí** của bí mật (ví dụ "mật khẩu admin ở `/root/...` trên VPS"), không ghi giá trị.
