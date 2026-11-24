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

## D29. Website chuyển sang server `lushvietnix2`, sau Cloudflare proxy, dữ liệu bắt đầu trống (2026-11-16)
Người dùng yêu cầu chuyển Control Plane từ VPS cũ sang server Vietnix `lushvietnix2`, giữ tên miền `suno.darkambient.co`, **không chuyển dữ liệu cũ** (tài khoản, thư viện, Train Style bắt đầu lại), và đồng ý chép chứng chỉ Cloudflare Origin của `darkambient.co` từ VPS cũ. Hệ quả kỹ thuật: server mới chỉ nhận 80/443 từ Cloudflare nên bản ghi DNS phải Proxied (thay cho DNS-only + Let's Encrypt của D17/D18); IP thật của người dùng lấy qua snippet real-ip sẵn có trên server; worker dùng địa chỉ Tailscale mới `100.96.4.9:8081`; vì database mới nên Render Node phải được tạo lại và nhận token mới. Tên cũ `studio.darkambient.co` không còn được phục vụ ở server mới. Cái giá: mất dữ liệu cũ theo lựa chọn; upload qua Cloudflare giới hạn 100 MB mỗi request (đủ cho Reference Audio 50 MB); server mới nhỏ hơn (2 CPU, 3,8 GB RAM, không swap) nên build Rust chạy trong container giới hạn bộ nhớ; thêm một lớp phụ thuộc vào Cloudflare.

## D30. Dataset Factory + Train Style Lo-fi: tech stack và cách làm pilot (2026-11-23)
Người dùng duyệt thiết kế `docs/superpowers/specs/2026-11-23-lofi-style-lora-design.md`. Mục đích: train Style LoRA từ khoảng 250 bài Lo-fi instrumental của chính người dùng (toàn quyền, MP3). Người dùng chốt:
- Làm thử với một nhóm nhỏ trước. Mỗi lần train là một nhóm bài, thành một phiên bản dataset bất biến và một LoRA.
- Bài có vocal chop hoặc tiếng nói bị loại.
- Train cả bài, không cắt đoạn 60 giây; trainer chỉ đọc 240 giây đầu.
- Không fine-tune base model, không publish trong pilot.

Tech stack hiện tại. Người dùng có thể đổi sau nếu không ổn.

| Thành phần | Công nghệ |
|---|---|
| Base Model | ACE-Step 1.5 `acestep-v15-turbo`, commit `ca1e85f` |
| Training | Side-Step tích hợp trong ACE-Step (MIT) + PyTorch + PEFT, qua job `train_lora` của Studio |
| Music Understanding | `acestep-captioner` (thẻ MIT; tải cần duyệt riêng); không dùng Music Flamingo |
| Audio Analysis | librosa + pyloudnorm (`beat_this` nếu cần); không dùng Essentia/madmom |
| Caption Generator | `acestep-captioner` + người duyệt; dự phòng viết tay |
| Metadata Builder | Python + Pydantic |
| Audio Processing | FFmpeg + SoundFile |
| Dataset Manager | Module dataset của `lush-training` + manifest JSONL + bộ xuất `dataset.json` |
| GPU | RTX PRO 6000 96 GB (`rtx6000`) |

Lựa chọn của agent:
- Dataset Factory nằm trong `lush-training/` (tái dùng module có sẵn), cài riêng ở `factory/` trên rtx6000. Nó chỉ chuẩn bị dữ liệu; train vẫn đi qua Studio. Việc này không đổi phạm vi trainer của D28.
- Pilot không sửa worker/API/web.
- Không dùng trigger tag.

Cái giá:
- Train và render A/B dừng tạo nhạc trên node duy nhất.
- Giấy phép `acestep-captioner` chỉ có thẻ MIT, không có file LICENSE riêng.
- Bài trên 240 giây phải cắt hoặc tách.

## D31. Catalog trên website thay trang duyệt localhost; model mô tả nằm trong Model Registry (2026-11-24)
Người dùng chốt thiết kế `docs/superpowers/specs/2026-11-23-lora-studio-web-design.md` (P1: Catalog). Thay trang duyệt caption chạy localhost + SSH tunnel của D30 bằng trang Catalog trên website: upload bài, phân tích tự động, tạo mô tả bằng model, sửa tay có undo/redo. Model mô tả là Model Registry với `kind=captioner` (cột `models.kind`, migration `0006`); mỗi model chạy trong **tiến trình con riêng, venv riêng** do worker khởi động từ `CAPTIONER_DIR/<tên>/captioner.json`, nói chuyện bằng JSON lines. Cho phép dùng cả model phi thương mại để thử nghiệm; mỗi bản mô tả lưu nguồn (model nào, hay người sửa) trong `caption_revisions`. Model sinh nhạc và model mô tả tách theo `kind`: các form Tạo nhạc / Style / Training chỉ thấy model sinh nhạc.

Quyết định kỹ thuật kèm theo:
- Trạng thái phân tích `failed` suy ra lúc đọc từ job analyze (job failed/cancelled thì `failed`), không ghi thêm ở đường fail/reaper: một nguồn sự thật, kể cả khi job hết lượt do lease hết hạn.
- Captioner chết lần thứ hai (hoặc timeout sau khi đã khởi động lại) thì job `caption_tracks` fail không retry, giữ các mô tả đã ghi; output rỗng hoặc hỏng chỉ lỗi từng bài. Tránh captioner hỏng lặp crash cả catalog.
- Tempo/key do người sửa tay không bị phân tích lại ghi đè; tùy chọn analysis nằm ở extra `analysis` của worker để node không cần librosa vẫn chạy bình thường.

Cái giá: model phi thương mại chỉ dùng cho thử nghiệm, cần xem lại giấy phép trước khi dùng ra production; mỗi captioner cần venv và phụ thuộc riêng (tốn dung lượng, cài tay); code nạp MOSS-Music chưa kiểm chứng đến khi người dùng duyệt tải model; trang review của T-014 bị bỏ; Catalog hiện chỉ có tới mô tả, chưa có dataset version và train từ website (các phase sau).

## D32. Tạo LoRA từ Catalog: bản chụp dataset bất biến, worker dựng dataset, checkpoint vào Registry (2026-11-24)
LoRA Studio P2. Tạo LoRA từ Catalog trên website thay cho việc chỉ thư mục dataset có sẵn trên node.

- **Bản chụp bất biến.** Khi tạo LoRA, API ghi `dataset_snapshots.content` (JSON: bài, caption đang chọn, tempo/key, bài giữ lại). Trigger SQLite chặn UPDATE/DELETE bản chụp (migration `0007`). Job được claim chỉ khi `snapshot_id` hợp lệ.
- **Payload train mang `samples` và `inputs`.** Worker tải audio từ `inputs`, tự dựng `dataset.json` theo định dạng Side-Step (ffmpeg, WAV 48 kHz) trước khi dừng inference, vì định dạng đó thuộc trainer adapter.
- **Checkpoint ghi vào Registry.** Khi hoàn tất, API ghi `lora_checkpoints` (epoch, step, sha256, bytes, `node_path`, trạng thái `kept`); đường dẫn node bị cắt khỏi `jobs.result` để không lộ ra API công khai.

Lý do: dataset đổi sau khi train thì LoRA không còn truy được nguồn; định dạng trainer không nên lộ ra API/web; P3 cần danh sách bản lưu để nghe thử và chọn.
Cái giá: bản chụp lưu thêm dữ liệu và không sửa được (sai thì tạo lại); worker phụ thuộc ffmpeg trên node (chưa kiểm); P3 phải đọc `node_path` từ bảng; toàn bộ mới test bằng mock.

Ruling P1–P12 của plan (chi tiết trong plan):
- P1 mọi kiểm tra trước train ở `plan_build` phía API, `holdout_seed` do web sinh một lần. P2 bài giữ lại 0–5, còn ít nhất 1 bài train. P3 từ khóa gọi style tùy chọn, 1–40 ký tự `[A-Za-z0-9_-]`, ghép vào caption, `custom_tag` để trống. P4 câu lệnh nghe thử bắt buộc 1–500 ký tự (P2 chỉ lưu). P5 tên LoRA 1–80 ký tự, không trùng không phân biệt hoa thường (so sánh Unicode ở Rust).
- P6 bản lưu = epoch chia hết `save_every` + epoch cuối (worker thêm `out/final` nếu thiếu). P7 số bài nghe thử = bản lưu x (1 + giữ lại) + 1. P8 thời gian ước theo run hoàn tất gần nhất, không có thì "—". P9 lỗi chặn (create 422): thiếu mô tả, giọng người chưa xác nhận, tên trùng, hết bài; thiếu Tempo/Key chỉ cảnh báo.
- P10 bản chụp nằm trong `dataset_snapshots.content` và `payload.samples`; dataset key là `lora_key` một phần nên `list_datasets` không báo như dataset cũ. P11 `/training` có nút chính "Tạo LoRA từ Catalog", form cũ vào `<details>` chỉ cho admin, Trainer xem được run. P12 P2 chỉ ghi danh sách checkpoint (`kept`); chọn bản, xóa bản, hạn 30 ngày thuộc P3.

## D33. LoRA Studio P3: nghe thử, chấm điểm, chọn một bản, xóa bản còn lại (2026-11-24)
Sau khi train từ bản chụp, API tự tạo job `eval_lora` (render nhạc nghe thử + chấm điểm); người dùng nghe, ghi chú, chọn **một** epoch đưa vào Style Preset; bản không chọn tự xóa sau 30 ngày kể từ lúc chọn hoặc xóa ngay. Job mới đều ghim vào node đã train (`payload.node_id`): `eval_lora`, `lora_files` (publish/delete, không dùng GPU), thử nhanh (`generate`). Migration `0008`.

Ruling R1–R12 của plan:
- R1 Khóa adapter theo epoch `<lora_key>-e<epoch>` vì ACE-Step nạp adapter một lần theo tên và `unload` hỏng; đổi epoch là khóa mới, `loras.base_key` giữ khóa gốc.
- R2 "Đưa vào Style" tạo Style tắt và job `lora_files publish`; xong mới duyệt LoRA và bật Style. Đổi epoch sau đó cập nhật Style cũ; thư mục bản trước không xóa.
- R3 Xóa file qua job `lora_files` (có lease, kết quả, lỗi), không qua heartbeat; heartbeat chỉ khởi động dọn theo hạn khi node không có job `lora_files` đang chờ.
- R4 Đánh giá tối đa 10 bản lưu (10 epoch cuối), số render <= 61, artifact kind `eval_00`..`eval_99`.
- R5 Bài train đã nằm trên node (`datasets/<key>/audio/sNNN.wav`), eval không tải lại; bài giữ lại nghe qua artifact catalog.
- R6 Câu lệnh render: bài gốc và mỗi bản lưu dùng `test_prompt`; "tạo lại bài giữ lại" dùng `train_caption`; mọi render qua `build_generation` như tạo nhạc thường.
- R7 Điểm: giống style (MERT, trung bình), giống bài train (max cosine MERT), giai điệu (chroma + DTW), chất lượng (Audiobox PQ 1-10). Cảnh báo học thuộc: MERT >= 0.95 hoặc giai điệu >= 0.80. Ngưỡng **chưa hiệu chỉnh**, điểm chỉ để tham khảo.
- R8 Ghi chú tối đa 500 ký tự thay cho chấm sao (W13), lưu khi rời ô.
- R9 Thử nhanh là job `generate` của người bấm, ra track trong thư viện của họ, ghim node bằng `payload.node_id`, chỉ với bản lưu chưa xóa.
- R10 Xóa LoRA bị chặn (409) khi còn Style dùng hoặc còn job train/eval/lora_files; nếu được thì `archived`, mọi bản lưu đánh dấu xóa kể cả bản chosen; bản chụp dataset giữ nguyên.
- R11 Sau eval worker khởi động lại `acestep-api` khi đã nạp adapter; tinh chỉnh: dừng `acestep-api` **trước khi chấm điểm** để trả VRAM cho máy chấm.
- R12 Style mặc định: tên = tên LoRA, độ mạnh 0.7 (mặc định trên web: độ mạnh Style hiện có nếu đã có), `prompt_template` = từ khóa, genre của catalog, model = base model.

Ruling trong lúc làm:
- Trạng thái đưa vào Style (chosen, expires_at, key, installations, approved, Style bật) chỉ đổi khi job `lora_files` hoàn tất; request chỉ kiểm tra, lưu metadata Style và xếp job. Publish lỗi thì bản trước vẫn chạy; POST lại = thử lại; cùng bản đã cài thì chỉ cập nhật Style. Giá: giao diện chỉ hiện lựa chọn sau khi node xong.
- Dấu xóa bị bỏ dở được thử lại (khi xếp job xóa và theo heartbeat mỗi ngày, kể cả LoRA archived); dọn bị 409 khi eval còn chạy.
- Mỗi LoRA tối đa một job `lora_files` đang sống: partial unique index nằm trong `0008` (chưa deploy, chỉ trên nhánh).
- Câu lệnh render đánh giá soạn ở API (test + giữ lại), worker dùng nguyên văn; câu giữ lại cắt còn 2000 ký tự thay vì chặn việc train. Eval không dựng được (vd base model tắt) thì bỏ qua kèm cảnh báo; `max_attempts` 2.
- Thời lượng thử nhanh dùng cùng khoảng kẹp theo genre như eval (60 s kẹp vào khoảng cho phép).
- Đường dẫn trong `lora_files` chỉ nhận hai dạng checkpoint của trainer, và phải nằm dưới `DATA_DIR/training`; runner đăng ký lại node sau `lora_files`.
- `eval_lora` báo `stage=running` kèm `phase` rendering/scoring (protocol chỉ cho preparing/running); tiến độ render 0..0.9; bài im lặng có điểm giai điệu 0.0, không phải null; `scorer.json` có `version` tùy chọn (mặc định '1').
- `loras.base_key` trả `COALESCE(base_key, key)`; `complete_eval` lỗi thì job vẫn `running` để worker gửi lại bản sửa.
- Thẻ các bước (train, nghe thử, ...) hiện trên mọi run có bản chụp; trạng thái `preparing_dataset` là bước 1.
- Web: bấm "Train lại" mở `/catalogs/<id>?retrain=<snapshot>`, hộp Tạo LoRA điền sẵn tùy chọn cũ (cả seed, tên "<tên> (2)") và tick lại các bài còn trong catalog.

Cái giá: bản lưu đang được đánh giá và chọn làm tăng dung lượng ổ node (xóa sau 30 ngày); máy chấm MERT/Audiobox là thêm hai venv phải cài; điểm chưa hiệu chỉnh nên dễ gây hiểu nhầm nếu coi là kết luận; mọi thứ mới test bằng mock.


## D34. Giữ Side-Step đi kèm ACE-Step; thông số train mặc định theo hướng dẫn Side-Step (2026-11-24)

Quyết định: train LoRA tiếp tục dùng Side-Step bản đi kèm ACE-Step 1.5 (`acestep/training_v2`, commit `ca1e85f`, giấy phép MIT của ACE-Step), không nâng lên Side-Step mới trên GitHub. Hộp Tạo LoRA đặt mặc định theo hướng dẫn của Side-Step: epoch theo số bài train (dưới 10 bài 300, 10–50 bài 150, trên 50 bài 80; "Kỹ" ×1,5; bỏ kiểu "Thử nhanh" 1 epoch theo yêu cầu người dùng), lưu bản để ra khoảng 10 bản, warmup ≈5% tổng bước (tối đa 100), rank 64 / alpha 128, AdamW 1e-4, giới hạn 120 phút. Base model vẫn là turbo.

Lý do: Side-Step trên GitHub (`koda-dernet/Side-Step`) từ v1.0.0-beta đổi sang CC BY-NC-SA 4.0 (thương mại phải xin phép) và đã thông báo ngừng phát triển, lưu trữ chỉ đọc từ 2027-03-01. Mặc định cũ (1/4/8 epoch, lưu mỗi epoch, warmup 2) chỉ hợp để thử quy trình: với 20 bài, 4 epoch chỉ khoảng 20 bước học. Turbo là model đang dùng để tạo nhạc; base chưa cài và cần 50 bước khi tạo nhạc.

Cái giá: không có các tính năng chỉ có ở Side-Step mới (tự lưu bản tốt nhất, loss mục tiêu, cắt đoạn 60 s ngẫu nhiên để học cả bài, lr preset 3e-4); bài dài vẫn chỉ học 240 s đầu. Thời gian train và VRAM của mặc định mới chưa đo trên rtx6000; phải chỉnh lại sau lần train thật đầu tiên. Side-Step mới khuyên train trên base, chưa thử.

## D35. Prompt tự viết từ Reference Audio; prompt gửi ACE-Step tối đa 850 ký tự (2026-11-24)

Quyết định: job mới `describe_input` cho một captioner (ưu tiên `acestep-captioner`) nghe Reference Audio và trả caption + tempo/key; web điền vào ô Prompt nếu đang trống, không thì đưa thành gợi ý. Prompt ghép gửi engine bị giới hạn 850 ký tự ở API (`fit_prompt`), rút phần người dùng gõ trước. Spec: `docs/superpowers/specs/2026-11-24-reference-auto-prompt-design.md`.

Lý do: người dùng muốn prompt sẵn từ file rồi sửa tùy ý; ACE-Step chỉ đọc 256 token đầu của `[instruction, caption, metas]` (đọc `conditioning_text.py` trên rtx6000) nên ô 2000 ký tự cũ làm mất template/tag/bpm/key mà không báo. Job riêng thay vì dùng `caption_tracks` vì file không thuộc catalog nào và kết quả chỉ là một gợi ý cho form.

Cái giá: thêm một job GPU ngắn mỗi lần tải file (priority 5, chạy trước nhạc đang chờ); nạp model mỗi lần (ACE-Step Captioner vài giây, MOSS khoảng một phút). Khi node đang train, prompt phải chờ. Vị trí trong hàng chờ chưa hiện (chỉ "đang chờ"/"đang nghe").
