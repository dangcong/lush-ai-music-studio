# AUTOPILOT — Chạy tự động từ thư mục nhạc tới gói model

Dùng khi người dùng muốn một quá trình không dừng lại hỏi: tự chuẩn bị dataset, tự duyệt caption, tự train tới khi xong hoặc tới giờ chỉ định, tự chọn checkpoint và xuất gói để họ test.

Ủy quyền nằm trong job file do **người dùng tự điền**. Đó là thứ thay cho việc hỏi từng bước; không có nó thì không chạy.

## Trước khi chạy

1. `python scripts/doctor.py`. ACE-Step phải được cài đúng commit và checkpoint gốc phải có trong vùng `pretrained`. Hai việc đó không thuộc autopilot: thiếu thì dừng, nêu rõ cần cài hay tải gì và xin phép riêng.
2. Đọc `docs/STATUS.md`. Nếu backend chưa từng chạy trên máy này: nói thẳng rằng lần đầu nên là một job ngắn (`stop.epochs: 1` trên một thư mục nhỏ, hoặc `stop.hours` nhỏ) để đo VRAM và tốc độ trước khi chạy cả kho nhạc.
3. Soạn `configs/autopilot.yaml` từ file mẫu cùng người dùng:
   - `music_dir`, `model`, `method`, `training` theo lời họ;
   - `stop`: bắt buộc có `epochs` hoặc `total_train_hours` để job có điểm kết thúc hẳn. Đề nghị thêm `patience_epochs` (dừng khi validation loss thôi cải thiện) và `hours` / `at` nếu họ chỉ cho máy chạy trong một khung giờ. Đọc lại cho họ bằng lời: job dừng khi nào, tối đa chiếm GPU bao lâu;
   - `captions`: hỏi họ mô tả kho nhạc, tốt nhất theo từng thư mục con. Viết lại thành mô tả tiếng Anh ngắn và đọc lại cho họ. Không tự bịa mô tả từ tên thư mục;
   - **để trống** `authorized_by` và để `false` ba dòng `authorization`. Giải thích từng dòng nghĩa là gì và yêu cầu họ tự sửa file. Họ trả lời đồng ý trong chat thì ghi đúng lời họ vào; không tự suy ra.
4. Nói rõ ba điều autopilot tự quyết và giới hạn của chúng:
   - caption = mô tả theo thư mục, không ai nghe từng đoạn;
   - không có sàng lọc vocal;
   - checkpoint được chọn theo validation loss, chưa ai nghe.

## Chạy

```bash
python scripts/autopilot.py run --job configs/autopilot.yaml --detach
```

`--detach` chạy nền, tách khỏi terminal và khỏi Claude Code. Sau khoảng một phút kiểm tra:

```bash
python scripts/autopilot.py status --job configs/autopilot.yaml
```

Báo người dùng: đang ở bước nào, khi nào dừng, log ở đâu. Không ngồi chờ.

Trong lúc train, trainer tự cập nhật `runs/<run_id>/PROGRESS.md` khoảng mỗi phút. Khi người dùng hỏi tiến độ: chạy lệnh `status` ở trên và tóm tắt từ file đó (phần trăm, epoch, thời gian còn lại, giới hạn nào tới trước, validation loss). Chỉ nêu thời gian còn lại khi báo cáo có số; không tự ước lượng.

## Khi xong

Đọc `autopilot/<name>/REPORT.md` trong vùng dữ liệu và tóm tắt: số bài dùng được và bị loại, số segment, dừng vì lý do gì ở step nào, VRAM đỉnh, checkpoint được chọn và các ứng viên, đường dẫn gói. Nhắc lại gói mang nhãn chưa đánh giá và hướng dẫn test nằm trong `IMPORT.md` của gói.

- Dừng theo giờ mà chưa hết epochs: chạy lại đúng lệnh trên để train tiếp từ checkpoint. Lần chạy lại vẫn nằm trong ủy quyền của job file đó.
- Người dùng muốn nghe checkpoint khác: `export_model.py export` với step khác; các mốc theo từng epoch còn trong thư mục run.

## Khi lỗi

Autopilot dừng ngay và không tự sửa: OOM, loss NaN, preprocess lỗi, dataset không chia được. Đọc `autopilot.log` và `status.py --run`, báo nguyên nhân và các lựa chọn. Đổi optimizer, learning rate, độ dài segment hay method là quyết định của người dùng và là một job mới (`name` mới).

## Vẫn phải hỏi riêng, kể cả khi có job file

Cài ACE-Step, tải model, cài hay đổi driver, chép gói sang máy khác, đưa model vào Lush Studio, deploy, push, xóa dữ liệu.
