# MODEL_COMPATIBILITY — Model nào, method nào, kiểm tới đâu

Cập nhật: 2026-11-13. Nhãn: **[Source]** đọc từ code ACE-Step commit `ca1e85f` · **[Đo]** số đo thật · **[Test]** test tự động không GPU · **[Chưa kiểm]** chưa chạy thật.

## Engine được ghim

ACE-Step 1.5, commit `ca1e85fe9430179831e6bc6be790c332190a3866` (cùng bản với Render Node của Lush Studio). `scripts/doctor.py` và trainer từ chối checkout khác commit hoặc đã bị sửa. Đổi commit là việc riêng: phải đọc lại các hàm trong `src/lush_training/acestep_probe.py` và kiểm thử lại.

ACE-Step tự ghim torch trong `pyproject.toml` của nó [Source]: `torch==2.7.1+cu128` trên Windows, `torch==2.10.0+cu128` trên Linux x86_64, Python `>=3.11,<3.13`. Trainer chạy bằng Python của virtualenv ACE-Step (`acestep.python` trong `project.yaml`), không cài torch riêng. Bản torch này có hỗ trợ RTX 5090 trên máy của bạn hay không: **[Chưa kiểm]**, `doctor.py` chạy bằng Python đó sẽ trả lời.

## Bốn training method, không gọi lẫn

| Method | Cập nhật | Đóng băng | Trong workspace |
|---|---|---|---|
| `full_decoder` | Toàn bộ DiT decoder | Condition encoder, tokenizer, detokenizer (cùng file model); VAE và text encoder (file riêng) | Có backend; [Chưa kiểm] trên GPU |
| `partial_decoder` | Các tensor decoder khớp `trainable_patterns` | Phần còn lại của decoder và mọi thứ ở trên | Có backend; [Chưa kiểm] trên GPU |
| `full_model` | Mọi thành phần | Không | Không có. Bị từ chối |
| `lora` | Adapter LoRA/PEFT | Toàn bộ model gốc | Không có ở đây. Lush Studio đã có Train Style |

`full_decoder` **không phải** fine-tuning toàn bộ model: các thành phần khác đóng băng. `run.json` và gói export ghi đúng tên method, số tham số train / tổng, và danh sách thành phần đóng băng. "Export sau khi merge adapter" không áp dụng ở đây: không có adapter nào, gói export là file model với decoder đã train.

Workspace không tự đổi method. Chọn `lora` hoặc `full_model` thì nhận lỗi, không nhận một method khác.

## Upstream hỗ trợ gì

- Upstream **không có** full fine-tuning ở commit này [Source]: `training_v2/model_loader.py` `load_decoder_for_training` đặt `requires_grad = False` cho mọi tham số, trainer chỉ mở tham số LoRA/LoKR. `docs/en/Large_Scale_SFT_Training_Guide.md` ghi "Full-parameter SFT is NOT natively supported" và chỉ đưa bản vá mẫu.
- Vì vậy `src/lush_training/acestep_backend.py` là phần **phải tự phát triển và kiểm chứng**. Nó:
  - import nguyên từ checkout: nạp model, `PreprocessedTensorDataset`, `collate_preprocessed_batch`, `sample_timesteps`, `apply_cfg_dropout`, `configure_memory_features`;
  - lặp lại đúng công thức của `FixedLoRAModule.training_step` (flow matching: `x_t = t·x1 + (1−t)·x0`, đích `x1 − x0`, MSE), không inject adapter;
  - tự làm: mở `requires_grad` cho decoder, optimizer step, validation loss, lưu/nạp đủ trạng thái.
- Preprocessing dùng CLI upstream `train.py fixed --preprocess`. Dạng lệnh đã chạy thật trên Render Node với variant turbo trên Linux [Đo, E1 2026-11-13]; với variant base và trên Windows: [Chưa kiểm].
- Bảng tài nguyên của upstream cho full SFT ghi decoder "~3,5B" và 60–80 GB mỗi GPU [Source: tài liệu upstream]. Con số 3,5B không khớp số đo 1,575B của bản 2B, và bảng viết cho dataset 116 nghìn bài trên nhiều GPU. Nó không chứng minh cũng không bác bỏ trường hợp bản 2B, batch 1, một GPU 32 GB.

## Checkpoint tương thích

`python scripts/inspect_model.py <tên>` đọc header `model.safetensors` và `config.json`, không cần GPU. Một checkpoint được coi là tương thích backend `acestep15-dit` khi:

- `config.json` khai báo kiến trúc ACE-Step và có file `modeling_*.py`;
- weights có thành phần `decoder` kiểu số thực (không lượng tử hóa).

Số tham số, dtype, dung lượng từng thành phần được đọc từ chính file đó. Không có con số nào của một variant được gán cứng cho variant khác. Đổi đường dẫn không làm một model bất kỳ train được: model khác kiến trúc bị báo không tương thích.

| Checkpoint | Trạng thái |
|---|---|
| `acestep-v15-base` (2B) | Mục tiêu chính. Chưa có trên máy nào của ta; cấu trúc file **[Chưa kiểm]** |
| `acestep-v15-turbo` (2B) | Header đã đọc trên Render Node: tổng 2.393.872.518 tham số, `decoder` 1.575.458.880, lưu bf16 [Đo 2026-11-13]. Là model distilled; full fine-tuning trên nó chưa có bằng chứng, `inspect_model` cảnh báo |
| `acestep-v15-sft`, các bản `xl-*` | [Chưa kiểm] |

"Tương thích" ở đây là về cấu trúc file. Có vừa VRAM hay không phải đo.

## VRAM: chưa có số đo

Chưa có lần chạy full fine-tuning nào. Phép tính sơ bộ cho decoder 1,575 tỷ tham số, trọng số fp32 (chưa tính activation): trọng số ~6,3 GB + gradient ~6,3 GB + trạng thái AdamW ~12,6 GB ≈ 25 GB; AdamW 8-bit hoặc Adafactor nhẹ hơn ở phần optimizer. Đây là **ước tính**, không phải kết quả. Lần đo đầu tiên (50 step, có phê duyệt) mới trả lời được, và kết quả ghi vào `docs/STATUS.md`.

Optimizer là lựa chọn ghi trong cấu hình (`adamw`, `adamw8bit`, `adafactor`); trainer không tự chuyển khi OOM.

## Nhiều máy, nhiều GPU

Hai máy RTX 5090 dùng được theo ba cách, theo thứ tự nên làm:

1. Mỗi máy một run độc lập (mỗi máy một bản workspace, một `TRAINING_DATA_ROOT`). Dùng được ngay khi một máy chạy được.
2. Một máy train, máy kia render mẫu đánh giá.
3. Một model train phân tán trên hai máy (DDP/FSDP). **Chưa làm và chưa nên làm**: VRAM hai GPU không cộng lại cho một model; cần benchmark mạng, trainer và checkpoint/resume trước, và cần một nhu cầu cụ thể.

Hệ điều hành, RAM, VRAM thực và kết nối mạng của hai máy chưa được xác minh; `doctor.py` trên từng máy là bước đầu tiên.
