# TRAINING_TEST_PLAN — Kiểm thử khả thi full fine-tuning trên RTX 5090

> **2026-11-13 — đã dừng (D28).** Phương án điều khiển training từ website được thay bằng workspace độc lập `lush-training/`. Tài liệu này giữ lại để tham khảo; phần supervisor, Task Scheduler, API, web và NAS trong đó không được triển khai. Nguồn hướng dẫn hiện hành: `lush-training/AGENTS.md`.

Ngày: 2026-11-13. Trạng thái: **kế hoạch, chưa chạy bước nào.** Bối cảnh và nhãn: `BASE_MODEL_TRAINING.md`. Kết quả đo ghi thẳng vào các bảng ở mục 4 của file này, kèm ngày.

## 1. Quy tắc

- Mỗi bước tải model và mỗi bước dùng GPU của máy training cần người dùng đồng ý trước. Chạy trong khung 18:00–06:00 hoặc lúc người dùng cho phép.
- Chỉ ghi số đo thật. Bước chưa chạy để trống; bước chạy với dữ liệu kỹ thuật ghi rõ là dữ liệu kỹ thuật.
- Training method cố định: full fine-tuning toàn bộ `decoder`. Không chuyển sang LoRA, không đóng băng một phần DiT để "cho vừa".
- **Điều kiện dừng:** nếu F2 cho thấy không cấu hình nào trong danh sách chạy được, dừng tại đó, viết báo cáo mục 5, không tự thử phương án khác.
- F1–F5 dùng script kiểm thử tối thiểu (vòng lặp train + lưu/nạp checkpoint), chưa phải `lush-trainer` hoàn chỉnh. Nó trả lời câu hỏi khả thi; phần đạt yêu cầu sẽ thành nền của trainer ở P2.

## 2. Dataset kiểm thử

Dataset kỹ thuật (ví dụ vài chục đoạn instrumental có quyền sử dụng do người dùng đưa, hoặc bộ tự sinh như E1) chỉ để đo tài nguyên và kiểm cơ chế. Nó không nói gì về chất lượng model. Dataset thật: Q4 của `BASE_MODEL_TRAINING.md`.

Cần ít nhất hai độ dài segment để thấy VRAM phụ thuộc độ dài thế nào: 60 giây và 240 giây (mức tối đa preprocessing của ACE-Step nhận).

## 3. Các bước

| # | Việc | Đo / kiểm | Tiêu chí đạt |
|---|---|---|---|
| **F0** | Khảo sát máy, chỉ đọc: bản Windows 11, driver NVIDIA, ổ đĩa và dung lượng trống, RAM; WSL2 đã bật chưa, các distro và Docker Desktop đang có, `.wslconfig` hiện tại; Tailscale; ứng dụng dùng GPU lúc máy nghỉ | VRAM nền và GPU utilization khi desktop đang đăng nhập, lấy mẫu 10 phút | Có bảng 4.1; biết đặt `.vhdx` của distro training ở ổ nào |
| **F0b** | Tạo distro Ubuntu riêng cho training (cần đồng ý; nếu WSL2 chưa bật thì người dùng bật và tự khởi động lại máy). Trong distro: `nvidia-smi`, RAM VM nhìn thấy, đường tới VPS qua Tailscale, đồng hồ; chạy thử một task rỗng của Task Scheduler ở chế độ "chưa đăng nhập" | `nvidia-smi` trong WSL2 báo VRAM/utilization của cả máy có khớp số đo từ Windows không; RAM cấp cho VM; task chạy được khi chưa đăng nhập không | GPU thấy được từ WSL2; các mục [Chưa rõ] ở `TRAINING_SCHEDULER.md` mục 2 có câu trả lời; không distro hay ứng dụng nào khác của người dùng bị ảnh hưởng |
| **F1** | Cài môi trường vào `train_root` trong distro: ACE-Step ghim commit, venv, torch bản hỗ trợ RTX 5090; tải `acestep-v15-base`, VAE, text encoder (cần duyệt). Nạp model, không train | SHA-256 các checkpoint; số tham số từng thành phần đọc từ header; `torch.cuda.is_available()`; một forward pass | `decoder` của base có đúng 1.575.458.880 tham số (nếu khác: ghi lại và tính lại mục 3 của tài liệu thiết kế); forward chạy không lỗi |
| **F2** | **Khả thi.** Preprocess dataset kỹ thuật bằng variant `base`. Mở `requires_grad` cho toàn bộ `decoder`. Chạy 50 optimizer step, batch 1, gradient checkpointing bật, trọng số fp32 + autocast bf16, lần lượt: (A) AdamW chuẩn → (B) AdamW 8-bit → (C) Adafactor; mỗi cấu hình ở segment 60 giây và 240 giây. Cấu hình trước chạy được vẫn đo tiếp cấu hình sau để người dùng có đủ số so sánh | VRAM đỉnh (`torch.cuda.max_memory_allocated`, `max_memory_reserved`, và `nvidia-smi` lấy mẫu 1 giây), RAM đỉnh của tiến trình, giây/optimizer step (trung vị, p95, bỏ 5 step đầu), loss hữu hạn, số tham số `requires_grad` | Ít nhất một cấu hình chạy hết 50 step ở segment 240 giây, không OOM, VRAM đỉnh ≤ 90 % VRAM tổng, loss hữu hạn. Không đạt → mục 5 |
| **F3** | **Trọng số thật sự được cập nhật.** Sau N step ở cấu hình đã chọn, so từng tensor của `decoder` với checkpoint gốc; so các thành phần đóng băng | Số tensor thay đổi / tổng; chuẩn L2 của sai khác theo từng nhóm lớp; tensor nào không đổi thì nêu tên | 100 % tensor có `requires_grad` trong `decoder` khác bản gốc; `encoder`, `tokenizer`, `detokenizer` trùng từng byte với bản gốc |
| **F4** | **Thời gian checkpoint và dừng.** Ghi checkpoint đầy đủ (decoder, optimizer, scheduler, RNG, manifest + SHA-256) 5 lần; đo từ lúc yêu cầu dừng tới lúc GPU trống | Thời gian ghi, thời gian tính SHA-256, dung lượng từng file; thời gian step → thoát tiến trình → VRAM về mức nền; thời gian nạp lại checkpoint | Có đủ số để tính `stop_margin` theo công thức ở `TRAINING_SCHEDULER.md` mục 5; VRAM về mức nền F0 (± 500 MiB) trong 60 giây sau khi tiến trình thoát |
| **F5** | **Resume chính xác.** Lần 1: chạy liền 2K step, ghi loss từng step. Lần 2: chạy K step → checkpoint → thoát tiến trình → tiến trình mới resume → chạy tiếp K step. Cùng seed, cùng dữ liệu | Step và learning rate ngay sau resume; loss step K+1…2K của hai lần; thứ tự mẫu; trạng thái optimizer (so tensor) sau khi nạp | Bắt buộc: step tiếp tục đúng K+1; learning rate trùng lịch; thứ tự mẫu trùng; trạng thái optimizer và trọng số sau khi nạp trùng từng bit với lúc lưu; loss **không** lặp lại chuỗi step 1…K (lỗi RNG đã thấy ở Side-Step, E1). Mong muốn: loss hai lần trùng nhau; nếu chỉ gần bằng (GPU không tất định) thì báo sai lệch lớn nhất, không tuyên bố bit-exact |
| **F6** | **Mất điện giả lập.** Kết thúc cứng tiến trình trainer: (a) giữa một step, (b) **đang ghi checkpoint**. Rồi resume. (c) `wsl --terminate` distro training giữa lúc ghi checkpoint — gần với mất điện hơn là kill tiến trình, nhưng vẫn **không** phải mất điện thật. Thêm: làm hỏng một file của checkpoint mới nhất rồi resume | Checkpoint nào được chọn; số step mất; có thư mục `.partial` sót không | (a) resume từ checkpoint gần nhất; (b), (c) checkpoint dở bị bỏ, resume từ cái trước; file hỏng → bị phát hiện qua SHA-256, lùi về cái trước; không trường hợp nào train lại từ step 0 mà không báo lỗi |
| **F7** | **Diễn tập cả hệ thống** (sau khi có supervisor, P2): 3 đêm liên tiếp tự chạy bằng Task Scheduler; một đêm máy bận lúc 18:00; một lần "Dừng ngay"; một lần reboot giữa đêm; supervisor bị kết thúc giữa session | Giờ bắt đầu / dừng / GPU trống từng đêm; báo cáo đêm; step nối tiếp giữa các đêm | GPU trống trước 06:00 cả 3 đêm; step đêm sau nối đúng đêm trước; đêm máy bận không train và không đụng tiến trình nào; sáng hôm sau distro training đã tắt và RAM Windows trở về mức trước 18:00; báo cáo khớp log |

Thứ tự bắt buộc: F0 → F0b → F1 → F2 → (người dùng chọn cấu hình) → F3, F4, F5 → F6 → viết code P2 → F7.

Chưa nằm trong kế hoạch này: chất lượng âm nhạc, chọn learning rate / số step, đánh giá nghe, sao lưu NAS (kiểm ở P4: khôi phục một run từ NAS sang thư mục trống rồi resume).

## 4. Bảng kết quả (điền khi chạy)

### 4.1 F0 — máy training

| Mục | Giá trị | Ngày |
|---|---|---|
| Windows 11 bản dựng; phiên bản WSL; kernel và bản Ubuntu của distro training | | |
| Distro / Docker Desktop khác của người dùng (để không đụng vào) | | |
| RAM cấp cho VM WSL2 (GB); `.wslconfig` hiện tại | | |
| `nvidia-smi` trong WSL2 so với Windows: VRAM dùng, utilization, danh sách tiến trình | | |
| Đường từ distro tới VPS qua Tailscale | | |
| Driver NVIDIA / phiên bản CUDA của torch | | |
| VRAM tổng (MiB) | | |
| VRAM nền khi desktop đăng nhập, máy nghỉ (MiB: thấp nhất / cao nhất) | | |
| GPU utilization khi nghỉ (%) | | |
| RAM tổng / trống (GB) | | |
| Ổ đĩa cho `train_root`, dung lượng trống (GB) | | |
| Tự bật lại sau mất điện; task chạy khi chưa đăng nhập | | |

### 4.2 F2 — khả thi

| Cấu hình | Segment | Chạy hết 50 step | VRAM đỉnh allocated / reserved / nvidia-smi (MiB) | RAM đỉnh (GB) | Giây/step trung vị / p95 | Loss hữu hạn |
|---|---|---|---|---|---|---|
| A. AdamW | 60 s | | | | | |
| A. AdamW | 240 s | | | | | |
| B. AdamW 8-bit | 60 s | | | | | |
| B. AdamW 8-bit | 240 s | | | | | |
| C. Adafactor | 60 s | | | | | |
| C. Adafactor | 240 s | | | | | |

Tham số `requires_grad`: ______ / ______ (kỳ vọng 1.575.458.880 trong `decoder`).

### 4.3 F3–F6

| Bước | Kết quả | Ngày |
|---|---|---|
| F3: tensor `decoder` thay đổi / tổng; thành phần đóng băng trùng byte | | |
| F4: dung lượng checkpoint đầy đủ (GB); thời gian ghi + kiểm (giây: trung vị / lớn nhất) | | |
| F4: thời gian từ yêu cầu dừng tới GPU trống (giây, lớn nhất) | | |
| F4: `stop_margin` tính ra (phút) | | |
| F5: step, learning rate, thứ tự mẫu sau resume | | |
| F5: sai lệch loss lớn nhất giữa chạy liền và chạy có resume | | |
| F6a / F6b / file hỏng | | |

## 5. Mẫu báo cáo khả thi

Viết sau F2 (và bổ sung sau F5), gửi người dùng **trước** mọi đề xuất phương án thay thế.

1. **Kết luận một dòng:** full fine-tuning toàn bộ DiT trên RTX 5090 32 GB — khả thi / khả thi có điều kiện / không khả thi.
2. **Môi trường:** bảng 4.1, commit ACE-Step, SHA-256 checkpoint gốc, phiên bản torch.
3. **Số đo:** bảng 4.2 nguyên văn, kể cả cấu hình thất bại kèm thông báo lỗi.
4. **Điều kiện kèm theo** (nếu có): optimizer nào, segment dài nhất, batch, dự phòng VRAM còn lại.
5. **Ước lượng từ số đo:** step mỗi đêm = (thời gian train thực trong khung giờ) / (giây mỗi step). Số đêm cho một run chỉ nêu khi đã có dataset thật và `total_steps`.
6. **Chưa biết:** những gì kiểm thử này không trả lời (chất lượng, nguy cơ quên kiến thức cũ, dataset thật).
7. **Nếu không khả thi:** nguyên nhân kỹ thuật cụ thể (thiếu bao nhiêu VRAM, ở phần nào). Phương án thay thế chỉ được liệt kê **sau khi người dùng đã đọc báo cáo và yêu cầu**.
