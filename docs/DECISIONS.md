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
`STYLE_TRAINING.md`, `DATASET_MANAGEMENT.md`, `AUDIO_TO_MIDI.md`, `SHEET_MUSIC.md` sẽ được viết khi phase tương ứng bắt đầu và có kết quả nghiên cứu/benchmark thật — không viết trước thông số chưa kiểm chứng.

## D16. nginx thay vì Caddy trên VPS (2026-11-10)
VPS `lush1-vps` dùng chung, đã chạy nginx + certbot cho các site khác trên cổng 80/443. Thêm Caddy sẽ tranh cổng. `lush-api` tự phục vụ trang web nên nginx chỉ proxy một cổng (`127.0.0.1:8018`). Rust được build trong container Docker tạm để không cài toolchain lên máy dùng chung.

## D17. Tên miền studio.darkambient.co, bản ghi DNS-only + Let's Encrypt (2026-11-10)
Người dùng chọn `studio.darkambient.co`. Bản ghi trỏ thẳng về VPS (không qua proxy Cloudflare), nên chứng chỉ phải được trình duyệt tin: dùng Let's Encrypt (webroot, như các site khác trên VPS) thay vì chứng chỉ Cloudflare Origin của `darkambient.co`. Không qua proxy còn tránh giới hạn kích thước upload và timeout của Cloudflare với file nhạc.

## D18. Đổi tên miền sang suno.darkambient.co (2026-11-11)
Người dùng yêu cầu đổi. Cùng cách như D17 (DNS-only + Let's Encrypt webroot). Tên cũ `studio.darkambient.co` giữ lại chỉ để chuyển hướng 301, để link cũ vẫn dùng được; cái giá là thêm một chứng chỉ phải gia hạn (certbot tự làm).

## D19. Reference Audio / Source Audio đi qua VPS dưới dạng artifact `audio_input` (2026-11-12)
Người dùng chỉ nói chuyện với VPS, và worker chỉ kéo việc từ VPS (D4), nên file đầu vào của một job được tải lên VPS, lưu như artifact có chủ (`owner_id`), rồi worker tải về qua worker protocol kèm kiểm tra SHA-256. Một file không mang vai trò; `mode` của job quyết định nó là Reference hay Source. Giới hạn 50 MB/file nên không mâu thuẫn với D9 (dataset train, model, LoRA vẫn không đi qua VPS). Giá: thêm dung lượng đĩa VPS và một lượt tải VPS → Render Node qua Tailscale cho mỗi job có audio.

## D20. Chế độ tạo nhạc là capability của model, không có chế độ giả (2026-11-12)
`reference` cần capability `reference_audio`, `cover` cần `cover`; API từ chối (`422`) khi model không có, worker `fail` không retry khi engine không có. Mock Engine cố ý không khai báo hai capability này để không bao giờ có kết quả "thành công" giả cho tính năng AI. Kiểm thử pipeline hai chế độ này vì vậy dùng worker giả trong test và ACE-Step thật khi chạy tay.

## D21. Khi train thì dừng tạo nhạc (2026-11-13)
Người dùng chốt (trả lời Q1 của `STYLE_TRAINING.md`), tạm thời cho giai đoạn đầu: render node chỉ chạy một việc GPU tại một thời điểm, nên trong suốt một training run worker dừng `acestep-api`, website không tạo nhạc được, và job generate gửi trong lúc đó nằm chờ trong queue tới khi inference lên lại. Lý do: 12 GB VRAM không đủ cho inference và training cùng lúc, và cách này đơn giản nhất. Giá: mỗi lần train là một khoảng ngừng dịch vụ, nên UI phải báo rõ trước khi tạo run và trong lúc đang train. Xem lại khi có GPU thứ hai hoặc cloud worker. Quyết định này là về thiết kế; mỗi lần dừng `acestep-api` bằng tay (kể cả thí nghiệm E1) vẫn cần người dùng đồng ý riêng.

## D22. AI Training: chỉ instrumental, chỉ RTX 3060, Style LoRA; generate được ưu tiên trước run mới (2026-11-13)
Người dùng chốt phạm vi: chỉ tạo và train nhạc instrumental (không làm tính năng train giọng hát, không xoá tính năng vocal sẵn có); chỉ dùng render node RTX 3060 12 GB, không cloud; ưu tiên Style LoRA và không gọi nó là train model; partial/full fine-tuning chỉ được đề xuất khi có trainer tương thích và bằng chứng tài nguyên. Job generate đang chờ được chạy trước khi một training run mới bắt đầu, nhưng run đã chạy thì không bị ngắt; Admin có khung giờ/lịch train để run không chờ vô hạn. Dataset thật do người dùng cung cấp kèm quyền sử dụng; file nghi có vocal chỉ được gắn cờ, người nghe quyết định. Lý do: nhóm 2–4 người, một GPU, nhu cầu thật là nhạc không lời. Giá: bỏ qua mọi nhánh vocal và cloud; thời điểm train phụ thuộc lúc queue rỗng hoặc khung giờ.

## D23. Tên gọi trên giao diện: LoRA là "Train Style", model là "model" (2026-11-13)
Người dùng chốt: mọi chữ hiển thị cho người dùng gọi LoRA là **Train Style** (trang Train Style, Train Style Library, "Tên Train Style"…); "model" chỉ dùng cho base model. Người dùng chọn Train Style qua Style Preset, không có ô chọn riêng ở form Tạo nhạc. Code, API, database và worker giữ nguyên tên `lora` (đổi tên ở đó là đổi hợp đồng mà không thêm giá trị). Tài liệu kỹ thuật vẫn có thể viết "LoRA" khi nói về adapter, trainer hay file.

## D24. Full fine-tuning base model qua đêm trên máy RTX 5090 riêng, chạy trong WSL2 (2026-11-13)
Người dùng yêu cầu: full fine-tuning ACE-Step 1.5 Base 2B (cập nhật 100% tham số DiT, không dùng LoRA thay thế khi chưa được chấp thuận) trên máy `DESKTOP-QNU7F08` (Windows 11, RTX 5090 32 GB, RAM 128 GB), chỉ trong khung 18:00–06:00, môi trường training là WSL2 Ubuntu; không cài lại Windows, không đổi phân vùng; hệ thống không tự tắt máy, reboot, kill ứng dụng người dùng hay tự đổi training method; Training Worker chạy độc lập với Claude Code. Quyết định này mở rộng D22 (vốn giới hạn ở RTX 3060 và Style LoRA): Style LoRA trên render node giữ nguyên, đây là hệ thống thứ hai trên máy thứ hai. Phạm vi instrumental của D22 vẫn áp dụng. Lý do: có GPU 32 GB và người dùng muốn một base model riêng. Giá: ACE-Step không có trainer full fine-tune nên phải tự viết và tự bảo trì trainer + supervisor; một run kéo dài nhiều đêm nên không dùng được mô hình job/lease hiện có; **tính khả thi trên 32 GB chưa được đo** — nếu kiểm thử (`TRAINING_TEST_PLAN.md`) cho thấy không khả thi thì dừng ở báo cáo kỹ thuật, không tự chuyển phương án. Thiết kế: `BASE_MODEL_TRAINING.md`.

## D25. "Style" thay "Presets"; Training Model và Model Library mới chỉ là giao diện (2026-11-13)
Người dùng chốt: preset chính là các style do studio thiết kế, nên bỏ mục Presets và thay bằng **Style** — một Train Style sau khi bấm Duyệt thì thành Style. Cách làm (lựa chọn của agent): Style vẫn lưu trong `style_presets` loại `trained`, API và database giữ tên `preset`; trang Style liệt kê mọi Train Style `approved`, cái nào chưa có dòng preset thì hiện "Cần chọn genre" và admin bấm Thiết lập để tạo. Không tự tạo preset lúc duyệt vì `style_presets.genre_id` là NOT NULL trong khi Train Style train ra chưa có genre; đổi cột đó cần dựng lại bảng SQLite đang được `tracks` tham chiếu. Cái giá: duyệt xong còn một bước chọn genre; preset loại `genre` (không có Train Style) bị ẩn khỏi cả trang quản trị lẫn form Tạo nhạc; admin không còn nghe thử Train Style đang đánh giá qua preset mới (phải duyệt trước, không ưng thì "Đưa về đánh giá"). Người dùng cũng yêu cầu luồng **Training Model → Model Library → duyệt → Models**; đợt này chỉ thêm hai mục menu và trang "Sắp có", phần chạy thật chờ duyệt T-009 (D24).

## D26. Genre có khoảng thời gian gợi ý thay cho một default duration (2026-11-13)
Người dùng yêu cầu đổi "Default Duration" của genre thành mốc thời gian tự điền (ví dụ 60–120 giây). Quyết định: hai cột `duration_min`, `duration_max` (10–240, min ≤ max); cột `default_duration` giữ lại trong bảng nhưng không còn đọc/ghi (không sửa migration cũ, không dựng lại bảng). Cách dùng ở trang Tạo nhạc là lựa chọn của agent: khoảng này là **gợi ý**, không khóa thanh Duration — khi chọn genre/subgenre mà chưa chọn Style, Duration ngoài khoảng được kéo về mốc gần nhất, trong khoảng thì giữ nguyên; người dùng vẫn kéo ra ngoài được. Cái giá: genre cũ sau migration có khoảng một điểm (min = max = default cũ) cho tới khi admin nới ra.

## D27. Thứ tự ưu tiên thiết lập ở form Tạo nhạc; Duration tối đa 300 giây (2026-11-13)
Người dùng chốt sau khi xem đề xuất: (1) Style cũng có khoảng Duration, được để trống; (2) chọn Genre hay Style thì Duration tự nhảy; (3) người dùng vẫn kéo được, trần 300 giây; (4) đổi Genre hoặc Style thì Duration nhảy lại, ghi đè giá trị đã kéo; (5) prompt template giữ "chọn một, cái cụ thể nhất" (Style → Subgenre → Genre), không cộng dồn. Thứ tự chung: người dùng chỉnh tay sau cùng → Style → Subgenre → Genre → mặc định hệ thống; mức nào để trống thì lùi xuống mức kế. Lựa chọn của agent (người dùng chưa nói rõ): Duration nhảy về **mốc "đến"** của khoảng (không phải mốc giữa, để luôn là bội số người dùng tự điền); Subgenre cũng tính là một lần "đổi"; khoảng phải điền đủ hai mốc hoặc trống cả hai; D26 thay đổi theo: hai cột khoảng nay cho phép NULL và `style_presets` có thêm hai cột cùng tên (cột `duration` cũ giữ lại, không dùng). Migration `0005` được viết lại trước khi commit; database dev đã được gỡ bản 0005 cũ bằng tay (có sao lưu `apps/api/data/lush.db.bak-before-0005-rewrite`). Cái giá: giá trị người dùng kéo tay mất khi đổi Genre/Style; bài 241–300 giây chưa đo trên GPU thật.

## D28. Training model làm trong workspace độc lập `lush-training/`, không qua website (2026-11-13)
Người dùng thay phương án của D24: chưa xây app, web UI hay hệ thống điều khiển training qua VPS. Thay vào đó là workspace `lush-training/` ở root repository, mở riêng bằng Claude Code trên máy training, hoạt động không cần VPS hay website: chuẩn bị dataset instrumental, caption, fine-tune pretrained checkpoint ACE-Step, đánh giá, xuất gói inference để người dùng tự chép sang Render Node. Claude Code điều phối script; trainer là tiến trình riêng và không cần Claude trong lúc chạy. Giữ từ D24 và D22: chỉ instrumental; không tự đổi training method, không thay bằng LoRA; mỗi lần tải model và mỗi lần dùng GPU cần phê duyệt. Bỏ khỏi phạm vi hiện tại: supervisor theo khung giờ, Task Scheduler, đồng bộ VPS, trang Admin, sao lưu NAS, yêu cầu bắt buộc WSL2 (hệ điều hành của máy training chưa xác minh). Lựa chọn của agent: workspace không import gì từ `apps/` hay `workers/` và không đổi hợp đồng nào; ACE-Step ghim cùng commit `ca1e85f` với Render Node; backend tự viết vì upstream không có full fine-tuning; `full_decoder` và `partial_decoder` có backend, `full_model` và `lora` bị từ chối; export ghép weights không cần torch. Cái giá: không theo dõi được từ website; không có lịch tự chạy hay tự phục hồi sau reboot; đưa model vào Lush Studio là thao tác tay; trainer GPU chưa chạy lần nào nên tính khả thi trên 32 GB vẫn chưa biết.
