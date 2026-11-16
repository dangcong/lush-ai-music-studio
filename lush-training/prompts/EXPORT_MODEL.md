# EXPORT_MODEL — Xuất gói inference cho Lush Studio

Export không dùng GPU. Chép gói sang máy khác, đưa vào Lush Studio hay đổi model mặc định là việc của người dùng hoặc cần họ phê duyệt từng bước.

## Các bước

1. Xác nhận với người dùng: run, checkpoint, và tên gói (chữ, số, `.`, `_`, `-`; ví dụ `lush-lofi-base-v1`). Tên này sẽ là key của model trong Lush Studio.
2. Nếu đã có `eval/<run_id>/<step>/EVALUATION.md`, gói sẽ kèm nó. Nếu chưa, nói rõ: gói vẫn xuất được (ví dụ để render mẫu đánh giá) nhưng sẽ mang nhãn "chưa đánh giá" và không nên đưa vào sản xuất.
3. `python scripts/export_model.py export --run <run_id> --checkpoint <step> --name <name>`.
4. `python scripts/export_model.py verify --package <data_root>/exports/<name>`.
5. Đọc `PROVENANCE.json` và báo người dùng:
   - checkpoint gốc và SHA-256;
   - method, số tham số train / tổng, thành phần đóng băng (nhắc lại: `full_decoder` không phải toàn bộ model);
   - `weight_merge`: bao nhiêu tensor decoder khác bản gốc, bao nhiêu tensor đóng băng đã kiểm trùng byte;
   - dataset, caption version, quyền sử dụng và giấy phép đã khai báo;
   - thứ gói **không** chứa mà máy inference phải có: `vae`, `Qwen3-Embedding-0.6B`, ACE-Step đúng commit.
6. Chỉ cho người dùng `IMPORT.md` trong gói: các bước chép sang Render Node, kiểm `verify_package.py`, smoke test, rollback. Nhấn mạnh:
   - model đặt trên Render Node (máy GPU), không phải VPS;
   - website chỉ thấy model sau khi `acestep-api` nạp nó và worker đăng ký lại;
   - bước cấu hình `acestep-api` nạp model tùy chỉnh **chưa được kiểm chứng**; phải thử trên node trước khi đổi cấu hình đang chạy.

## Không làm

- Không tự chép gói ra khỏi máy, không SSH sang máy khác, không sửa cấu hình Render Node hay VPS.
- Không đổi model mặc định của Lush Studio.
- Không ghi đè một gói đã có: tên trùng thì dùng tên mới.
- Không nói gói "sẵn sàng dùng" khi chưa có smoke test nạp model và tạo audio thật.
