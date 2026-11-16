# CLAUDE_WORKFLOW — Claude Code làm gì với từng yêu cầu

Dành cho Claude Code. Quy tắc chung ở `AGENTS.md`; lệnh chi tiết ở `TRAINING_GUIDE.md`. File này nối yêu cầu của người dùng với quy trình trong `prompts/`.

| Người dùng nói (ví dụ) | Làm theo | Dùng GPU |
|---|---|---|
| "Kiểm tra môi trường và model này có train được trên máy hiện tại không." | `doctor.py`, `inspect_model.py`; trả lời theo ba mức bên dưới | Không |
| "Chuẩn bị dataset từ thư mục này; chưa chạy training." | `prompts/PREPARE_DATASET.md` | Không |
| "Kiểm tra caption và báo những segment cần tôi nghe lại." | `prompts/BUILD_CAPTIONS.md` | Không |
| "Train theo cấu hình này, tối đa 4 giờ, dừng an toàn và lưu checkpoint." | `prompts/START_TRAINING.md` | Có: hỏi trước |
| "Tiếp tục run này từ checkpoint hợp lệ gần nhất." | `prompts/START_TRAINING.md` (phần resume) | Có: hỏi trước |
| "Đánh giá checkpoint…" | `prompts/REVIEW_CHECKPOINT.md` | Render: có |
| "…xuất model để dùng trong Lush Studio." | `prompts/EXPORT_MODEL.md` | Không |
| "Tự làm hết từ thư mục này, train tới khi xong hoặc tới giờ X, rồi đưa tôi checkpoint để test." | `prompts/AUTOPILOT.md` | Có: ủy quyền trong job file |

## "Model này có train được trên máy này không?"

Trả lời ba câu hỏi riêng, không gộp:

1. **Cấu trúc có tương thích không?** `inspect_model.py` trả lời chắc chắn.
2. **Môi trường có đủ không?** `doctor.py` (và doctor chạy bằng Python của ACE-Step) trả lời: GPU, driver, torch thấy CUDA, đĩa.
3. **Có vừa VRAM và chạy được không?** Chỉ một lần chạy thử có phê duyệt mới trả lời. Trước đó chỉ được nói "chưa đo", kèm phép ước tính trong `MODEL_COMPATIBILITY.md` và ghi rõ là ước tính.

## Cách làm việc

- **Trạng thái nằm trên đĩa.** Mỗi phiên mới: `docs/STATUS.md`, `status.py`, rồi `run.json` / `events.jsonl` của run liên quan. Không dựa vào trí nhớ của phiên trước.
- **Hỏi trước khi dùng GPU hay tải về.** Nêu: lệnh sẽ chạy, mục đích, tài nguyên dự kiến (thời gian, đĩa, VRAM nếu đã có số đo), giới hạn dừng. Một lần đồng ý chỉ áp cho đúng lần chạy đó.
- **Chạy nền việc dài.** `train.py start` tự tách tiến trình. Sau khi khởi động, kiểm `status.py` một lần để xác nhận trainer đã vào vòng lặp, rồi báo người dùng; không ngồi chờ hàng giờ.
- **Báo cáo bằng số đọc được từ file.** Loss, step, VRAM lấy từ `status.json` / `metrics.jsonl`. Không ước lượng thay cho số đo; chưa có thì nói "chưa có dữ liệu".
- **Khi lỗi:** đọc `status.py` và `--log`, nêu `stop_reason` và mã thoát, nêu điều đã biết và chưa biết, đề xuất lựa chọn. Không tự đổi method, optimizer, batch, learning rate. Không xóa checkpoint.
- **Giải thích kết quả** phân biệt: số bài, số giờ audio, số segment, epoch, optimizer step. Loss giảm không có nghĩa nhạc hay hơn.

## Caption: phần của Claude và phần của người dùng

Claude Code không nghe được audio. Việc Claude làm được:

- Chạy `analyze_audio.py` và đọc số đo (loudness, dynamic range, phổ thô; tempo ước lượng nếu đã cài librosa).
- Đọc metadata có sẵn: tên file, thư mục, `metadata_csv`, ghi chú của người dùng.
- Hỏi người dùng mô tả từng nhóm bài, rồi viết caption nhất quán từ mô tả đó và điền metadata có cấu trúc kèm `evidence` (`source: human` hoặc `source_metadata`).
- Phát hiện metadata thiếu hoặc mâu thuẫn (BPM trong caption khác metadata, cùng một track mà segment ghi khác thể loại, caption nhắc tới vocal).
- Đánh dấu `needs_listening` cho segment mà mô tả không đủ để viết.

Việc chỉ người dùng làm: nghe, xác nhận nội dung, đặt `approved`. Tempo do công cụ ước lượng có thể sai gấp đôi hoặc một nửa; chỉ đưa vào `metadata.tempo_bpm` khi người dùng xác nhận.

Nếu sau này có công cụ hiểu audio được cấu hình (ví dụ một captioner chạy cục bộ, cần phê duyệt tải), kết quả của nó đi vào `evidence` với `source: tool`, `tool`, `tool_version`, và caption vẫn phải được người dùng duyệt.

## Đề xuất thí nghiệm

Được đề xuất (không tự chạy): thứ tự đo optimizer, learning rate thử, số step, độ dài segment. Mỗi đề xuất nêu câu hỏi nó trả lời, chi phí GPU dự kiến, và điều kiện dừng. Mỗi thí nghiệm là một run riêng có `run.json` riêng; không sửa run cũ.
