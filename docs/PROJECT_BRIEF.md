# PROJECT_BRIEF

**Lush AI Music Studio** — nền tảng web tạo nhạc AI phục vụ sản xuất nội bộ của Lush Media Company Limited. Trải nghiệm kiểu "nhập prompt → Generate → nghe/tải", thiết kế cho workflow sản xuất nhạc không lời của công ty.

## Mục tiêu

1. Nhân viên tạo nhạc 1–3 phút từ prompt + thể loại + preset, nghe và tải WAV/MP3.
2. Train phong cách riêng của công ty bằng LoRA từ dữ liệu có quyền sử dụng, rồi dùng LoRA + prompt để tạo bài mới.
3. Xuất file phục vụ hậu kỳ trong FL Studio (WAV, MP3, MIDI, MusicXML, PDF).
4. Mở rộng bằng dữ liệu và cấu hình: thêm thể loại, preset, LoRA, model, engine, render node mà không viết lại hệ thống.

## Phạm vi V1

Nhạc không lời, bốn nhóm: Lo-fi, Jazz, Dark Ambient, Dark Academia. Một Render Node (`render-node-01`, Ubuntu 24.04, RTX 3060 12GB), một job GPU tại một thời điểm. Engine: ACE-Step 1.5 (ưu tiên Turbo).

## Môi trường

| Môi trường | Vai trò |
|---|---|
| Windows (dev) | Phát triển toàn bộ source; chạy được không cần GPU nhờ Mock Engine |
| GitHub private | Source control, CI, release |
| VPS | Control Plane: web, API, database, queue, registry |
| Ubuntu Render Node | Compute Plane: worker, ACE-Step, model/LoRA/dataset |

## Nguyên tắc

Đơn giản, rõ ràng, ổn định; không overengineering. Không bịa thông số; không báo hoàn thành khi chưa kiểm thử thật. Không thay đổi production, không xóa dữ liệu, không push khi chưa được phê duyệt.

## Tiêu chí MVP

Xem mục 20 của master prompt; trạng thái từng tiêu chí được theo dõi trong `ROADMAP.md`.
