# Prompt tự viết từ Reference Audio, và giới hạn prompt ACE-Step đọc

Ngày: 2026-11-24. Người dùng duyệt mockup `docs/superpowers/mockups/reference-auto-prompt.html` ("làm theo đề xuất của bạn, ren xong thì tôi sửa tùy ý").

## Mục đích

Khi tải file Reference Audio ở trang Tạo nhạc, hệ thống cho một model mô tả nghe file và điền sẵn prompt; người dùng sửa tùy ý trước khi Generate. Đồng thời prompt gửi vào ACE-Step không được vượt phần ACE-Step thật sự đọc: ACE-Step chỉ đọc 256 token đầu của `[instruction, caption, metas]` (đã đọc `conditioning_text.py` trên rtx6000), tức khoảng 850 ký tự caption tiếng Anh; quá thì template/nhạc cụ/mood ở cuối caption và cả bpm/key/duration bị bỏ mà không báo.

## Hành vi

1. Tải file vào Reference Audio, ô Prompt đang trống → web gọi `POST /audio-inputs/{id}/prompt-jobs`, hiện "Đang nghe file để viết prompt…" (vị trí trong hàng chờ). Vẫn gõ được.
2. Job xong:
   - Prompt vẫn trống → điền caption vào ô Prompt, kèm nhãn "Tự tạo từ <file> · sửa thoải mái", nút **Viết lại** (chạy job mới, thay prompt; lấy lại bản trước bằng **Hoàn tác**) và **Xóa prompt**.
   - Người dùng đã gõ → không ghi đè; hiện thẻ gợi ý với **Thay prompt / Thêm vào cuối / Bỏ**.
   - Tempo/Key đo được hiện thành chip; chỉ khi bấm **Dùng Tempo/Key này** mới điền vào BPM/Key (Nâng cao).
3. Lỗi, máy render không có captioner, hoặc job bị hủy → "Không viết được prompt · Thử lại"; form vẫn dùng bình thường. Bấm Generate lúc đang nghe thì tạo nhạc với prompt hiện có.
4. Gỡ file reference: prompt đã điền giữ nguyên; job đang chạy bị bỏ qua (không hủy).

Giới hạn prompt:
- Ô Prompt đếm **độ dài caption sau khi ghép** (prompt + template genre/Style + nhạc cụ + mood + "instrumental, no vocals", như `compose_prompt`) trên **850**; vàng từ 90%, đỏ khi vượt, kèm câu "ACE-Step chỉ đọc khoảng 850 ký tự; phần sau bị bỏ". Ô vẫn nhận tối đa 2000 ký tự.
- API, khi ghép: nếu vượt 850 thì **rút ngắn phần prompt của người dùng** (cắt ở cuối câu, rồi dấu phẩy, rồi khoảng trắng, chỉ nhận điểm cắt ở nửa sau) để template, nhạc cụ, mood vẫn còn; nếu riêng phần còn lại đã vượt thì cắt cả chuỗi. `prompt_final` (trang chi tiết bài hiện sẵn) là đúng chuỗi đã gửi.

## Thiết kế

- **Job mới `describe_input`** (worker protocol): payload `{model: {key}, inputs: {audio: <audio_input>}}`, priority 5 (trước tạo nhạc đang chờ, sau `lora_files`), `max_attempts` 2, tiêu đề "Viết prompt: <file>". Kết quả `{caption, mood, instruments, vocals, source_ref, tempo_bpm|null, key|null}`; API kiểm caption không rỗng (≤ 4000), chuẩn hóa key (`normalize_key`), tempo ngoài 20–300 → null.
- **Chọn model:** `acestep-captioner` nếu đang bật và có trên node đang bật; không thì captioner đầu tiên như vậy; không có → 422 "Chưa có model mô tả trên máy render". Một input chỉ có một job `describe_input` sống cho mỗi người: gọi lại khi đang có thì trả job đó (200), xong rồi thì tạo mới (201, dùng cho "Viết lại").
- **Worker:** báo `describe_input` khi có captioner. Chạy script captioner một lần (như `caption_tracks`, một bài), tách trường bằng `parse_output`; nếu có extra `analysis` thì đo tempo/key bằng `analyze_file`.
- **Web:** đọc job bằng `GET /jobs/{id}` mỗi 2 giây tới khi xong; hàm `composePrompt` (TS) giống `compose_prompt` (Rust) để đếm.

Cái giá: thêm một job GPU ngắn mỗi lần tải file (ACE-Step Captioner khoảng 5–10 s, chưa đo trên website); khi node đang train thì phải chờ. Cắt prompt ở API làm mất phần đuôi người dùng gõ quá dài (đúng với những gì ACE-Step vốn đã bỏ, nay nhìn thấy được ở `prompt_final`).
