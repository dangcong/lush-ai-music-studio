# STATUS — Đã kiểm chứng gì, còn thiếu gì

Cập nhật: 2026-11-13. Đọc file này đầu mỗi phiên. Khi một hạng mục được chạy thật, ghi kết quả vào đây kèm ngày, máy và cấu hình.

Mức kiểm chứng: **Thật** = đã chạy với dữ liệu/phần cứng thật · **Test** = test tự động không GPU (`pytest`, dữ liệu tổng hợp, backend giả) · **Chưa** = đã viết, chưa chạy · **Thiếu** = chưa viết.

## Tóm tắt

Phần không cần GPU (cấu hình, doctor, soi checkpoint, dataset, caption, vòng đời run, checkpoint/resume, export, đánh giá sau render, autopilot) đã viết và có test: 59 test đạt, 1 bỏ qua (so sánh làm tròn bf16 với torch; cần torch). Chạy trên máy dev Windows 11, Python 3.14, ngày 2026-11-13.

**Chưa có lần training nào trên GPU.** Backend ACE-Step được viết theo source thật của commit `ca1e85f` nhưng chưa chạy một dòng nào với checkpoint thật. Chưa biết full fine-tuning có vừa 32 GB VRAM không.

## Từng hạng mục

| Hạng mục | Mức | Ghi chú |
|---|---|---|
| `doctor.py` | Thật (máy dev) | Đã chạy trên máy dev: đọc được GPU qua `nvidia-smi`, RAM, đĩa, định dạng audio. Chưa chạy trên máy RTX 5090 |
| Kiểm checkout ACE-Step (`acestep_probe`) | Thật (chỉ source) | Đạt với checkout `ca1e85f` lấy từ GitHub (chỉ code, không có weights) |
| `inspect_model.py` | Test | Chỉ với file safetensors tổng hợp. Chưa đọc checkpoint ACE-Step thật bằng công cụ này |
| Dataset: scan, trùng lặp, gần trùng, nhóm, split, segment, manifest | Test | Audio tổng hợp (WAV, FLAC, MP3). Chưa chạy trên thư viện nhạc thật; ngưỡng gần trùng 0,97 chưa được hiệu chỉnh trên nhạc thật |
| Vocal: gắn cờ từ CSV, ghi quyết định người nghe | Test | Không có detector nào được tích hợp hay tải |
| Caption: nháp, kiểm tra, khóa, JSON cho engine | Test | Định dạng JSON theo source upstream; engine thật chưa đọc file do workspace sinh ra |
| Phân tích audio: số đo kỹ thuật | Test | Tempo (librosa) và LUFS (pyloudnorm) là tùy chọn, chưa cài, chưa chạy |
| Vòng đời run: giới hạn, dừng, resume, checkpoint hỏng, NaN, OOM, retention, early stopping | Test | Với backend giả. Resume cho chuỗi loss trùng từng số với lần chạy liền |
| Trainer chạy nền, sống sau khi lệnh khởi động kết thúc, dừng bằng `stop` | Test (tiến trình thật) | Windows, backend giả. Chưa kiểm: logout, reboot, Linux/WSL2, trainer thật |
| Backend `acestep_backend.py`: nạp model, mở decoder, train step, validation, lưu/nạp trạng thái | **Chưa** | Chỉ biên dịch được. Mọi thứ bên dưới phụ thuộc lần chạy đầu tiên |
| Preprocess qua CLI upstream | **Chưa** | Dạng lệnh đã chạy thật ở Lush Studio với turbo trên Linux; chưa chạy từ workspace này, chưa với base, chưa trên Windows |
| Export: ghép decoder, kiểm byte phần đóng băng, manifest, kiểm gói | Test | File tổng hợp F32. Làm tròn bf16 kiểm bằng giá trị tính tay; chưa đối chiếu với torch; chưa có model thật nào được engine nạp |
| Đánh giá: kế hoạch, kiểm kỹ thuật, phiếu A/B, memorization, báo cáo | Test | Trên audio tổng hợp |
| Autopilot (`autopilot.py`): thư mục nhạc → dataset → caption theo thư mục → train → chọn checkpoint theo validation loss → gói | Test | Chạy trọn với backend giả và audio tổng hợp, kể cả chạy lại và dừng khi trainer lỗi. Bước preprocess và train thật chưa chạy |
| Báo cáo tiến độ `PROGRESS.md` do trainer tự ghi; giới hạn dừng của autopilot (epochs, tổng giờ train, dừng sớm theo validation, giờ của phiên) | Test | Backend giả. Chưa thấy báo cáo của một run thật |
| Render mẫu đánh giá | **Thiếu** | Cần engine inference trên GPU; chưa có script. Hiện render tay theo `plan.json` |
| Nạp model xuất ra vào `acestep-api`, tạo nhạc thật | **Chưa** | Biến cấu hình để `acestep-api` nạp model tùy chỉnh chưa được xác minh (ghi trong `IMPORT.md` của gói) |
| Nhiều máy / DDP | **Thiếu** | Cố ý chưa làm (`MODEL_COMPATIBILITY.md`) |
| Sao lưu checkpoint ra NAS, lịch chạy theo khung giờ | **Thiếu** | Có `--stop-at`; không có lịch tự khởi động |

## Điều chưa biết về backend (phải trả lời bằng lần chạy đầu tiên)

1. `load_decoder_for_training(..., precision="fp32")` trên checkpoint lưu bf16 có nạp đúng và vừa RAM/VRAM không.
2. Số tham số `requires_grad` đếm lúc chạy có khớp số tính từ header không (trainer dừng nếu khác).
3. `configure_memory_features` có bật được gradient checkpointing cho decoder khi không có adapter không.
4. VRAM đỉnh và giây/step với từng optimizer (`adamw`, `adamw8bit`, `adafactor`) ở segment 60 giây; có OOM không.
5. Loss có hữu hạn và nằm cùng khoảng với LoRA đã đo (0,7–0,9) ở những step đầu không.
6. Gradient có tới các nhóm lớp của decoder không; thành phần đóng băng có giữ nguyên từng byte không. (Kiểm theo thành phần; không đòi mọi tensor phải đổi sau vài step.)
7. Thời gian ghi và kiểm một checkpoint đầy đủ (ước tính 9–19 GB) và dung lượng thật.
8. Resume: step, learning rate, thứ tự mẫu có nối đúng không; loss sau resume lệch bao nhiêu so với chạy liền.
9. `PreprocessedTensorDataset` có nhận thư mục tensor dưới `TRAINING_DATA_ROOT` sau `set_safe_root` không, trên Windows và Linux.
10. Model xuất ra có được `AutoModel.from_pretrained` nạp và tạo ra audio hợp lệ không.

## Điều kiện còn thiếu trước lần train thật

1. Chọn máy và biết thông số thật: hệ điều hành, RAM, VRAM, đĩa trống (`doctor.py` trên máy đó). Quyết định chạy Windows gốc hay WSL2.
2. Cài ACE-Step đúng commit ghim kèm virtualenv của nó trên máy training; xác nhận torch trong đó thấy RTX 5090. *(Cần phê duyệt.)*
3. Tải `acestep-v15-base`, `vae`, `Qwen3-Embedding-0.6B` vào vùng `pretrained`. *(Cần phê duyệt; dung lượng base chưa kiểm, turbo cùng họ là 4,8 GB.)*
4. Dataset instrumental thật có quyền sử dụng, đã `scan`, đã nghe các file cần nghe, đã `build`.
5. Caption cho mọi segment train/validation, do bạn duyệt, đã `lock`.
6. Một lần chạy thử kỹ thuật ngắn có phê duyệt (ví dụ 50 optimizer step) để trả lời mục "Điều chưa biết" 1–9 và chọn optimizer. Chỉ sau đó mới đặt `max_steps` cho run thật.
7. Cách render mẫu đánh giá (máy nào, engine nào) và một lần smoke test nạp model xuất ra.

## Nhật ký số đo thật

| Ngày | Máy | Việc | Kết quả |
|---|---|---|---|
| | | | Chưa có |
