# DECISIONS — Nhật ký quyết định kiến trúc

Mỗi mục: quyết định, lý do, cái giá phải trả. Thêm mục mới ở cuối; không sửa lịch sử.

## D1. Viết mới thay vì fork `SA1NG/ai_music_studio`
Repo cũ (tag `v1.0.2`, cùng commit với `v1.0.1`, file `VERSION` ghi `1.0.0`) là app **một máy**: FastAPI + Gradio + worker chạy cùng máy với ACE-Step, không có đăng nhập người dùng, API và worker chia sẻ ổ đĩa. Mô hình đó không khớp Control Plane + Compute Plane. Ta giữ lại phần đã được kiểm chứng (xem `MODULES.md`): adapter REST tới `acestep-api`, bộ tổng hợp nhạc mock, và kinh nghiệm ghi trong comment (bug `lora/unload`, NaN loss…).
*Lưu ý pháp lý:* repo cũ **không có file LICENSE**. Mã do nhân viên công ty viết nên dùng nội bộ được, nhưng nên bổ sung giấy phép/xác nhận quyền sở hữu trước khi chia sẻ ra ngoài. ACE-Step 1.5: MIT. Side-Step: GitHub không nhận diện được giấy phép (`NOASSERTION`) — **phải đọc và xác nhận giấy phép trước Phase 3**.

## D2. SQLite WAL, một database
Vài chục người dùng nội bộ, một VPS, ghi ít. SQLite WAL đủ và bớt một dịch vụ phải vận hành. Mọi truy cập đi qua SQLx với SQL chuẩn; chuyển PostgreSQL khi cần là đổi driver + vài hàm JSON (`json_each`, `json_patch`). Giá: một writer tại một thời điểm.

## D3. Hàng đợi nằm trong database, không Redis
Claim là một câu `UPDATE … RETURNING` nguyên tử. Không thêm hạ tầng; job bền vững cùng transaction với metadata. Worker poll 3 giây — độ trễ này không đáng kể so với thời gian sinh nhạc.

## D4. Worker kéo việc (pull) qua HTTPS, kèm lease
Render Node nằm sau NAT văn phòng; pull nghĩa là không mở cổng nào trên node. Lease + heartbeat cho phép phát hiện worker chết mà không cần kết nối thường trực.

## D5. Tailscale cho mạng riêng VPS ↔ Render Node
Chọn **Tailscale** thay vì WireGuard thuần: tự xuyên NAT, không cần IP tĩnh hay mở cổng ở văn phòng, thêm node mới chỉ là một lệnh, có ACL. Giá: phụ thuộc dịch vụ điều phối của bên thứ ba (dữ liệu vẫn mã hóa đầu-cuối bằng WireGuard). Nếu sau này cần tự chủ hoàn toàn: chuyển sang WireGuard thuần hoặc Headscale — worker chỉ cần đổi `CONTROL_PLANE_URL`. Endpoint `/api/worker/*` chỉ nên được reverse proxy cho phép từ dải IP tailnet.

## D6. Một bảng `jobs` cho mọi loại job
`type` + `payload` JSON thay vì bảng riêng cho generation/training. State machine, lease, scheduling dùng chung; thêm loại job không cần bảng mới. Kết quả nghiệp vụ nằm ở bảng riêng (`tracks`, sau này `loras`).

## D7. Lược bớt bảng so với danh sách thực thể ban đầu
- *Roles*: cột `users.role` + phân quyền cố định trong code. Ba vai trò là đủ cho V1.
- *Subgenres*: là `genres` có `parent_id`.
- *Track Versions*: `tracks.parent_track_id` + `version`.
- *Worker Heartbeats*: chỉ giữ lần gần nhất trên `nodes` (không lưu lịch sử).
- *AI Engines*: là chuỗi `models.engine` + `nodes.engines` do node báo.
Thêm bảng khi có nhu cầu thật.

## D8. Model tự vào Registry khi node báo
Node `register` với model chưa có → tự thêm (`active`). Admin có thể `disabled`. Đỡ một bước nhập tay dễ sai tên; nguồn sự thật về "model nào chạy được" vốn là máy render.

## D9. VPS giữ bản master WAV và MP3 preview của nhạc đã hoàn thành
Để nhân viên nghe/tải được khi Render Node tắt. Một bài 3 phút WAV 48 kHz stereo khoảng 35 MB — chấp nhận được với đĩa VPS; khi đầy thì chuyển sang object storage (storage key đã trừu tượng). Model, LoRA, dataset, checkpoint **không** đi qua VPS.

## D10. VPS chọn seed
Job luôn mang seed cụ thể nên chạy lại (retry, lease hết) cho cùng kết quả và metadata luôn tái lập được.

## D11. Phiên bằng cookie, không JWT
Thu hồi được ngay (xóa dòng `sessions`), không cần quản lý khóa ký. SameSite=Lax + API chỉ nhận JSON là lớp chống CSRF của V1.

## D12. Gọi ACE-Step qua REST server `acestep-api`, không import thư viện
Giữ đúng cách repo cũ đã kiểm chứng. Worker và ACE-Step tách môi trường Python; nâng cấp ACE-Step không kéo theo worker. Giá: ACE-Step không có API hủy task đã được xác minh, nên hủy job đang sinh phải chờ task hiện tại kết thúc.

## D13. Thư viện nhạc dùng chung
Mọi nhân viên đã đăng nhập nghe được mọi track (đây là công cụ sản xuất nội bộ); chỉ chủ sở hữu và admin được sửa/xóa. Job thì riêng tư theo chủ job. Xóa track là xóa mềm.

## D14. `packages/shared-contracts` chưa tạo
Hợp đồng hiện là hai file Markdown trong `docs/` và được khóa bằng integration test ở cả ba phía. Sinh type tự động (OpenAPI) sẽ làm khi số endpoint đủ lớn để việc đồng bộ tay thành gánh nặng.

## D15. Tài liệu theo phase
`LORA_TRAINING.md`, `DATASET_MANAGEMENT.md`, `AUDIO_TO_MIDI.md`, `SHEET_MUSIC.md` sẽ được viết khi phase tương ứng bắt đầu và có kết quả nghiên cứu/benchmark thật — không viết trước thông số chưa kiểm chứng.

## D16. nginx thay vì Caddy trên VPS (2026-11-10)
VPS `lush1-vps` dùng chung, đã chạy nginx + certbot cho các site khác trên cổng 80/443. Thêm Caddy sẽ tranh cổng. `lush-api` tự phục vụ trang web nên nginx chỉ proxy một cổng (`127.0.0.1:8018`). Rust được build trong container Docker tạm để không cài toolchain lên máy dùng chung.

## D17. Tên miền studio.darkambient.co, bản ghi DNS-only + Let's Encrypt (2026-11-10)
Người dùng chọn `studio.darkambient.co`. Bản ghi trỏ thẳng về VPS (không qua proxy Cloudflare), nên chứng chỉ phải được trình duyệt tin: dùng Let's Encrypt (webroot, như các site khác trên VPS) thay vì chứng chỉ Cloudflare Origin của `darkambient.co`. Không qua proxy còn tránh giới hạn kích thước upload và timeout của Cloudflare với file nhạc.

## D18. Đổi tên miền sang suno.darkambient.co (2026-11-11)
Người dùng yêu cầu đổi. Cùng cách như D17 (DNS-only + Let's Encrypt webroot). Tên cũ `studio.darkambient.co` giữ lại chỉ để chuyển hướng 301, để link cũ vẫn dùng được; cái giá là thêm một chứng chỉ phải gia hạn (certbot tự làm).
