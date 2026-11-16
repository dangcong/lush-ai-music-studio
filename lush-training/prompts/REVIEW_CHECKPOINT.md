# REVIEW_CHECKPOINT — Đánh giá một checkpoint

Mục tiêu: giúp người dùng quyết định giữ hay bỏ một checkpoint bằng cách so với model gốc. Bạn không quyết định thay, và không kết luận "nhạc hay hơn" từ loss hay số đo.

## Các bước

1. `python scripts/status.py --run <run_id>`: chọn checkpoint cùng người dùng (checkpoint đầy đủ hoặc mốc). Nêu train loss và validation loss tại đó như bối cảnh, không như kết luận.
2. Bộ prompt: `configs/eval_prompts.yaml` (chép từ file mẫu). Phải chỉ gồm prompt instrumental, có cả prompt trong style và ngoài style, ít nhất 2 seed, có `version`. Prompt viết độc lập với caption của dataset. Hỏi người dùng nếu chưa có.
3. `python scripts/evaluate.py plan --run <run_id> --checkpoint <step> --prompts configs/eval_prompts.yaml`.
4. **Render (GPU, cần phê duyệt).** Workspace chưa có script render. Cần: xuất checkpoint thành gói (`prompts/EXPORT_MODEL.md`), cho engine inference nạp model gốc rồi model ứng viên, render từng mục trong `plan.json` với cùng seed / steps / guidance vào `eval/<run_id>/<step>/parent/` và `candidate/`, lyrics `[Instrumental]`. Thỏa thuận với người dùng máy nào làm việc này. Ghi steps và guidance đã dùng vào `render_settings.json`.
5. `python scripts/evaluate.py check --run <run_id> --checkpoint <step>`: file thiếu, sai thời lượng, im lặng, clipping, quá nhỏ.
6. `python scripts/evaluate.py ab --run <run_id> --checkpoint <step>`: tạo cặp nghe mù và `ab/sheet.csv`. Hướng dẫn người dùng nghe và điền phiếu. Không mở và không đọc cho họ `ab_key.json` trước khi họ điền xong.
7. `python scripts/evaluate.py report --run <run_id> --checkpoint <step>`.
8. Tóm tắt cho người dùng:
   - vấn đề kỹ thuật ở từng phía;
   - mẫu bị gắn cờ memorization (phải nghe cạnh segment train tương ứng); nói rõ phép so chỉ bắt được lặp lại gần nguyên văn;
   - vocal: không có detector; chỉ có điều người nghe ghi trong phiếu;
   - kết quả phiếu nghe, số người nghe và số cặp; với ít người và ít cặp chỉ thấy được khác biệt lớn;
   - prompt ngoài style có bị kéo về style của dataset không (dấu hiệu quên kiến thức cũ), theo lời người nghe.
9. Hỏi người dùng quyết định: giữ, bỏ, hay thử checkpoint khác.

## Không làm

- Không tự duyệt checkpoint, không tự xuất "bản tốt nhất" theo validation loss.
- Không dùng tập test của dataset cho việc nào khác ngoài lần đánh giá cuối do người dùng yêu cầu.
- Không dùng output của model làm dữ liệu train.
- Không khởi động thêm run để "cải thiện" khi chưa được yêu cầu.
