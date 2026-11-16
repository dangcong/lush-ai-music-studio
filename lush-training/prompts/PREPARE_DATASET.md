# PREPARE_DATASET — Chuẩn bị dataset instrumental

Dùng khi người dùng đưa một thư mục nhạc và muốn có dataset version. Không dùng GPU. Không chạy training.

## Cần biết trước khi làm

- Thư mục nhạc (một hoặc nhiều). Tên dataset.
- Quyền sử dụng: ai xác nhận, giấy phép / nguồn. Nếu người dùng chưa nói, hỏi; không tự điền `rights.confirmed: true`.
- Có file nào là các bản khác nhau của cùng một tác phẩm (remix, bản thu lại, stem) không.

## Các bước

1. `python scripts/doctor.py` nếu chưa chạy trong phiên này. Vùng dữ liệu phải sẵn sàng.
2. Tạo `configs/dataset.yaml` từ `configs/dataset.example.yaml`: `name`, `source_dirs`. Để `rights.confirmed: false` cho tới khi người dùng xác nhận.
3. `python scripts/prepare_dataset.py scan --config configs/dataset.yaml`.
4. Báo cho người dùng, bằng số lấy từ kết quả scan:
   - số file, số track dùng được, số giờ, số nhóm tác phẩm;
   - file bị loại và lý do (không giải mã được, quá ngắn, trùng từng byte);
   - cảnh báo kỹ thuật (clipping, im lặng nhiều, sample rate thấp, mono) để họ quyết định giữ hay loại;
   - cặp nghi gần trùng;
   - tình trạng vocal: nói rõ nếu chưa có detector nào chạy (`not_screened`).
5. Nếu người dùng cho biết các bản liên quan hoặc muốn loại file: ghi vào một `metadata_csv` (cột `path`, `work_group`, `exclude`, `rights_note`), khai trong config, scan lại.
6. File bị gắn cờ vocal (`needs_listening`): liệt kê để người dùng nghe. Ghi quyết định của họ bằng `prepare_dataset.py review … --by "<tên họ>"`. Không tự quyết định thay.
7. Khi người dùng xác nhận quyền sử dụng: điền `rights` theo lời họ.
8. Xác nhận lại với người dùng độ dài segment và tỉ lệ split, rồi `python scripts/prepare_dataset.py build --config configs/dataset.yaml --version v1`.
9. Báo kết quả: số track, nhóm, segment, giờ của từng tập; số track bị loại; tình trạng sàng lọc vocal ghi trong manifest.

## Không làm

- Không sửa, chuyển, đổi tên hay ghi đè audio gốc.
- Không tách vocal. Không kết luận một file "không có vocal" từ tên file hay số đo.
- Không sửa một dataset version đã khóa: cần thay đổi thì tạo version mới (`v2`).
- Không chạy `train.py preprocess` hay bất cứ lệnh GPU nào trong quy trình này.

## Dừng và hỏi khi

- Dataset có ít nhóm tác phẩm tới mức không chia được ba tập.
- Phần lớn file có cảnh báo kỹ thuật, hoặc nhiều cặp gần trùng bất thường.
- Người dùng chưa xác nhận quyền sử dụng.
