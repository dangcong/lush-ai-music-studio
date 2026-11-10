# CURRENT — Việc đang làm

Cập nhật: 2026-11-10. Đọc file này đầu mỗi phiên. Giữ ngắn: chỉ task đang mở; task xong chuyển sang `COMPLETED.md`.

## Đang làm

Không có task đang mở.

## Chờ người dùng

Danh sách đầy đủ ở `ROADMAP.md` (mục "Việc cần người quyết định"). Mọi bước deploy, push, tải model đều cần phê duyệt.

## Tiếp theo

1. Người dùng nghe thử, đánh giá chất lượng theo genre; điều chỉnh prompt template của genre nếu cần.
2. Thử khởi động lại máy render (cần người dùng đồng ý): xác nhận ổ NVMe tự mount và hai dịch vụ tự chạy.
3. Benchmark đầy đủ hơn (180–240 s, nhiều genre) — `TESTING.md`.
4. Phase 3 (dataset, LoRA training) — dữ liệu và checkpoint đặt ở `/mnt/ai-studio/AiStudio/data/`.
