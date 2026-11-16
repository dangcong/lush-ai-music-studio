# Lush Training

Workspace độc lập để fine-tune pretrained checkpoint ACE-Step cho **nhạc instrumental**: chuẩn bị dataset, caption, train có kiểm soát, đánh giá, xuất model cho Lush Studio. Không cần VPS hay website đang chạy.

Mức kiểm chứng hiện tại: phần dataset, caption, vòng đời run, checkpoint/resume và export đã có test tự động chạy không cần GPU. **Trainer trên GPU chưa chạy lần nào.** Xem `docs/STATUS.md` trước khi dùng.

## Mở bằng Claude Code

Mở riêng thư mục này (không mở cả repository Lush Studio):

```bash
cd lush-training
```

```bash
claude
```

Claude Code đọc `AGENTS.md` và làm theo. Bạn đăng nhập Claude Code theo cách chính thức của nó; workspace không có chức năng đăng nhập và không dùng API trả phí.

## Cài đặt lần đầu trên máy training

1. Cài Python 3.11 hoặc 3.12 (ACE-Step `ca1e85f` yêu cầu `>=3.11,<3.13`).
2. Tạo môi trường cho workspace: `py -m venv .venv` rồi `.venv\Scripts\python -m pip install -e ".[dev]"`.
3. Chọn một thư mục dữ liệu ngoài Git và chép `configs/project.example.yaml` thành `configs/project.yaml`, sửa `data_root`.
4. Chạy `python scripts/doctor.py`.

Cài ACE-Step, tải checkpoint và mọi bước dùng GPU đều cần bạn đồng ý từng lần; Claude Code sẽ nêu mục đích, dung lượng và giới hạn trước khi làm.

## Ví dụ yêu cầu

Gõ bằng tiếng Việt. Đây là ví dụ cách dùng, không phải lệnh đã được phép chạy sẵn:

- "Kiểm tra môi trường và model này có train được trên máy hiện tại không."
- "Chuẩn bị dataset từ thư mục này; chưa chạy training."
- "Kiểm tra caption và báo những segment cần tôi nghe lại."
- "Train theo cấu hình này, tối đa 4 giờ, dừng an toàn và lưu checkpoint."
- "Tiếp tục run này từ checkpoint hợp lệ gần nhất."
- "Đánh giá checkpoint và xuất model để dùng trong Lush Studio."
- "Soạn job autopilot cho thư mục này, train 3 epoch hoặc tới 6 giờ sáng, xong thì xuất gói cho tôi test."

Mỗi yêu cầu có một file hướng dẫn tương ứng trong `prompts/` mà Claude Code làm theo.

## Cấu trúc

```
AGENTS.md            quy tắc cho Claude Code
configs/             file cấu hình mẫu (project, dataset, training, eval prompts)
prompts/             quy trình cho từng loại yêu cầu
scripts/             điểm vào dòng lệnh (mỗi script một việc)
src/lush_training/   code
docs/                kiến trúc, hướng dẫn, tương thích model, trạng thái kiểm chứng
tests/               test không cần GPU
```

Dữ liệu (audio, dataset, tensors, checkpoint, runs, exports) nằm ở `TRAINING_DATA_ROOT`, không nằm trong repository.

## Tài liệu

- `docs/TRAINING_GUIDE.md` — từng bước và từng lệnh.
- `docs/MODEL_COMPATIBILITY.md` — model nào, method nào, đã kiểm tới đâu.
- `docs/ARCHITECTURE.md` — code và bố cục dữ liệu.
- `docs/CLAUDE_WORKFLOW.md` — Claude Code làm gì với từng yêu cầu.
- `docs/STATUS.md` — đã kiểm chứng gì, còn thiếu gì trước lần train thật.
