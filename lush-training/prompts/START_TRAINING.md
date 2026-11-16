# START_TRAINING — Bắt đầu, theo dõi, dừng, tiếp tục một run

Lệnh `preprocess`, `start`, `resume` dùng GPU. Chỉ chạy sau khi người dùng đồng ý cho đúng lần chạy đó.

## Trước khi xin phép

1. Đọc `docs/STATUS.md`. Nếu backend chưa từng chạy trên máy này, nói thẳng: lần đầu phải là một lần chạy thử kỹ thuật ngắn (ví dụ `limits.max_steps: 50`), không phải run thật.
2. Kiểm cấu hình training với người dùng. Nhắc lại bằng lời:
   - method và ý nghĩa của nó (`full_decoder` không phải toàn bộ model);
   - dataset version, caption version, checkpoint gốc;
   - optimizer, learning rate, batch × gradient accumulation;
   - giới hạn: `max_steps`, `max_epochs`, `max_hours`, và giới hạn của phiên (`--session-hours` hoặc `--stop-at`);
   - chu kỳ checkpoint và retention (cái gì sẽ bị xóa).
3. `python scripts/train.py create --config configs/training.yaml`. Đọc lại cho người dùng số tham số train / tổng và thành phần đóng băng mà lệnh in ra.
4. `python scripts/train.py preflight --run <run_id>`. Mục nào FAIL thì dừng và báo.

## Xin phép

Nêu trong một tin nhắn: lệnh sẽ chạy; việc đó chiếm GPU trong bao lâu là tối đa; đĩa cần (checkpoint × `keep_resumable` + mốc); điều gì làm nó dừng; điều chưa biết (ví dụ chưa có số đo VRAM cho cấu hình này). Chờ câu trả lời.

## Chạy

1. Nếu tensor chưa có: `python scripts/train.py preprocess --run <run_id> --confirm-gpu`. Báo số tensor và thời gian.
2. `python scripts/train.py start --run <run_id> --session-hours <N> --confirm-gpu` (hoặc `--stop-at HH:MM`).
3. Sau khoảng một phút: `python scripts/status.py --run <run_id>`. Xác nhận trainer đã vào vòng lặp (step tăng, loss hữu hạn). Nếu `failed`: `status.py --run <run_id> --log 80`.
4. Báo người dùng: run id, step hiện tại, loss, VRAM đỉnh, giây/step, khi nào phiên tự dừng, cách dừng sớm (`train.py stop`). Nói rõ trainer chạy độc lập với phiên Claude Code, không tự chạy lại sau reboot, và việc sống qua logout chưa được kiểm chứng.

## Theo dõi

`status.py` khi người dùng hỏi. ETA chỉ nêu khi `status.py` in ra. Phân biệt optimizer step, epoch (lượt qua các segment train), số bài và số giờ audio.

## Dừng

`python scripts/train.py stop --run <run_id>`: trainer xong step đang chạy, ghi checkpoint, thoát. Kiểm lại bằng `status.py` rằng trạng thái là `stopped` và có checkpoint đúng step đó. Không kill tiến trình; nếu trainer không phản hồi, báo người dùng và để họ quyết định.

## Tiếp tục

1. `python scripts/status.py --run <run_id>`: xem trạng thái, checkpoint hợp lệ mới nhất, checkpoint bị loại (nếu có, nói cho người dùng cái nào hỏng và vì sao).
2. `train.py preflight`, xin phép như trên, rồi `python scripts/train.py resume --run <run_id> --session-hours <N> --confirm-gpu`.
3. Kiểm `events.jsonl` có sự kiện `resumed` với đúng checkpoint, và step tiếp tục từ đó.

Run `completed` không resume được: muốn train thêm thì tạo run mới. Run `failed` vì `no_valid_checkpoint`: không train lại từ đầu; báo người dùng.

## Khi lỗi

| `stop_reason` | Việc của bạn |
|---|---|
| `out_of_memory` | Báo VRAM đỉnh đã ghi, cấu hình đang dùng. Nêu các lựa chọn (optimizer khác, segment ngắn hơn, `partial_decoder`) và hệ quả của từng cái. Không tự chọn |
| `non_finite_loss` | Báo step xảy ra, loss và grad norm các step trước đó. Checkpoint tốt gần nhất còn nguyên. Đề xuất, không tự đổi learning rate |
| `mismatch` | Nêu cái gì không khớp `run.json`. Không sửa `run.json` |
| `crash` | Đọc log, nêu lỗi nguyên văn |

Kết quả đo thật (VRAM đỉnh, giây/step, thời gian ghi checkpoint) ghi vào bảng "Nhật ký số đo thật" trong `docs/STATUS.md`.
