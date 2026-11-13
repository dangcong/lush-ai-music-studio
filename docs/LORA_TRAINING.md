# LORA_TRAINING — Audit và thiết kế AI Training (M0)

Ngày: 2026-11-12, cập nhật 2026-11-13. Trạng thái: **M0 xong; thí nghiệm khả thi E0 + E1 đã chạy thật (mục 5.1). Chưa có dòng code training nào trong sản phẩm.** Phạm vi đã chốt 2026-11-13: chỉ nhạc instrumental, chỉ RTX 3060 12 GB, ưu tiên Style LoRA (mục 12).
Source audit: branch `main`, HEAD `4118c6b` + thay đổi T-007 chưa commit (Reference/Cover, giữ nguyên). ACE-Step trên `render-node-01`: commit `ca1e85f` (2026-08-29), working tree sạch.

Quy ước nhãn trong tài liệu này:
- **[Đo]** số đo từ lần chạy thật, có ngày. **[Runtime]** đọc trực tiếp trên máy thật ngày audit. **[Source]** đọc từ code tại commit nêu trên. **[Tài liệu]** bên thứ ba tự công bố, ta chưa kiểm. **[Đề xuất]** thiết kế chưa làm. **[Chưa rõ]** chưa có bằng chứng.
- Đường dẫn `acestep/...`, `train.py`, `docs/...` không có tiền tố là trong `/mnt/ai-studio/AiStudio/ACE-Step-1.5` trên render node.

## 1. Kết luận chính

1. Nền tảng hiện tại (queue, lease, worker protocol, registry, RBAC) dùng lại được cho training; **không cần đổi kiến trúc**. Phần thiếu là toàn bộ nghiệp vụ training: job type, trainer adapter, vòng đời GPU, metrics, dataset, đánh giá.
2. ACE-Step `ca1e85f` có **hai trainer** [Source]: trainer tích hợp (chạy trong tiến trình `acestep-api`, có REST) và **Side-Step bản tích hợp upstream** (`train.py`, `acestep/training_v2/`, chỉ CLI, tiến trình riêng). Đề xuất dùng Side-Step CLI làm backend chính, trainer REST làm phương án dự phòng (mục 4).
3. LoRA bằng Side-Step **chạy được trên RTX 3060 12 GB** ở cấu hình smoke test E1 (segment 60 giây, rank 32, batch 1): VRAM đỉnh khoảng 5,4 GB [Đo 2026-11-13, mục 5.1]. Chưa đo với segment dài hơn, rank lớn hơn hay dataset thật. Tài liệu chính thức mâu thuẫn nhau (16 GB tối thiểu cho trainer tích hợp; ~7 GiB cho Side-Step có gradient checkpointing).
4. LoRA trước, fine-tune DiT sau. Full SFT theo hướng dẫn chính thức cần tối thiểu 8 GPU × 40 GB [Tài liệu] — ngoài tầm máy hiện tại, chỉ xét khi LoRA đã chứng minh giới hạn.
5. ~~Blocker: render node lệch driver NVIDIA.~~ Đã hết sau reboot 2026-11-13 (E0): kernel `7.0.0-43`, driver 595.99.02 ở cả hai phía, `nvidia-smi` chạy lại [Đo]. Ghi chú: trước reboot tiến trình CUDA mới vẫn khởi tạo được, chỉ NVML hỏng.

## 2. Audit source hiện tại

Mức kiểm chứng: *unit* = test tự động với HTTP/worker giả; *tích hợp* = API + worker thật, engine mock; *GPU thật* = chạy với ACE-Step trên `render-node-01`; *nghe* = người đánh giá.

| Hạng mục | Phân loại | Bằng chứng | Mức kiểm chứng |
|---|---|---|---|
| Authentication, phiên | Implemented & Verified | `apps/api/src/auth.rs:29-76` (`AuthUser`, `Admin`) | unit + production (đăng nhập thật từ 2026-11-10) |
| RBAC | Partially Implemented | Role `admin/trainer/member` trong schema (`migrations/0001_init.sql:8`); code chỉ phân biệt admin và chủ sở hữu (`auth.rs:37-41`). Role `trainer` chưa được route nào dùng | unit |
| Queue bền vững, claim nguyên tử | Implemented & Verified | `jobs` (`0001_init.sql:121-151`), claim theo capability (`routes/worker.rs:230-262`) | unit + GPU thật (job generate) |
| Lease, phục hồi | Implemented & Verified | Lease 90 s (`config.rs:54`), `recover_jobs` + reaper (`queue.rs:60-95`) | unit; chưa thử mất mạng thật giữa job |
| Retry | Implemented & Verified | Tự xếp lại khi `attempt < max_attempts` (mặc định 3, `0001_init.sql:141`); `retry_job` tạo job mới (`routes/jobs.rs:480`) | unit |
| Cancel | Implemented & Verified | Cờ `cancel_requested` (`routes/jobs.rs:454-470`), worker thấy qua heartbeat/progress (`runner.py:185-194`, `routes/worker.rs:214-222`) | unit; cancel job GPU thật chưa ghi nhận |
| Worker capabilities | Implemented & Verified cho generate | Node báo `job_types`, engines, models, loras khi register (`routes/worker.rs:73-180`); worker hardcode `JOB_TYPES = ["generate"]` (`runner.py:26`) | unit + GPU thật |
| Model Registry | Implemented & Verified | Model do node tự báo (`routes/worker.rs:117`); admin chỉ bật/tắt (`routes/registry.rs:92`) | GPU thật |
| LoRA Registry | Partially Implemented | Bảng `loras` (`0001_init.sql:60-70`) chỉ có key, name, version, genre, base model, status; **chỉ có `GET /loras`** (`lib.rs:112`), không có API tạo/sửa; không có cột dataset, config, hash, checkpoint | unit (dữ liệu chèn thẳng trong test) |
| LoRA khi tạo nhạc | Implemented but Unverified | Kiểm tra `approved` + cùng base model (`routes/jobs.rs:258-276`); worker nạp một lần, bật/tắt bằng toggle, đặt scale (`engines/acestep.py:336-362`) | unit với HTTP giả. **Chưa từng chạy với adapter thật** (node báo `loras=[]`) |
| Trạng thái LoRA dùng chung | Implemented but Unverified | `/v1/lora/unload` không dùng vì không gỡ được layer ở `ca1e85f` (`engines/acestep.py:119-120`); mỗi job tự đặt lại trạng thái | unit; kinh nghiệm từ repo cũ |
| Inference adapter | Implemented & Verified | `engines/acestep.py`: text, reference, cover; `thinking: False` cố định (`:243`) | GPU thật (text 2026-11-10 trên production; reference/cover 2026-11-12 trên dev); chưa có đánh giá nghe |
| Dataset | Missing | Trang `/datasets`, `/training`, `/lora-library` là `ComingSoonPage` (`apps/web/src/App.tsx:51-73`); thư mục `data/{datasets,training,loras}` trên node tồn tại và rỗng [Runtime] | — |
| Training | Missing | Không có job type, route, adapter nào | — |
| Storage, upload | Implemented & Verified cho artifact nhỏ | Artifact theo ID + SHA-256 (`routes/worker.rs:346`); upload audio ≤ 50 MB qua VPS (`routes/inputs.rs`, T-007 chưa deploy). D9: dataset/model/LoRA không đi qua VPS | unit + GPU thật |
| Progress | Partially Implemented | Một số `progress` 0–1, `stage`, `message`, log cắt còn 64 KB cuối (`routes/worker.rs:26,282-316`). **Không có metrics có cấu trúc, không có lịch sử theo step** | unit + GPU thật |
| Số liệu GPU | Implemented, đang hỏng trên node | `gpu.py` gọi `nvidia-smi`; hiện trả `None` vì lệch driver | [Runtime] |
| Evaluation | Missing | Chỉ có favorite/rating chung trên track | — |
| Deploy, rollback | Implemented & Verified | `scripts/deploy.sh`: rollback VPS và worker; chỉ restart `acestep-api` khi service file đổi (`:170-215`) | chạy thật 2026-11-11, 2026-11-12 |
| Test | — | API 12, worker 105, web 67 test. Không test nào dùng GPU; không có test cho training | — |

Repo nghiên cứu cũ (`research/ai_music_studio`, chỉ đọc): `worker/backends/acestep.py:536-650` train qua REST của ACE-Step theo chuỗi scan → preprocess → `/v1/training/start` → lấy `final/adapter` → `/v1/init`. Ghi nhận trong code: loss NaN làm `/v1/training/status` trả 500; sau train phải `/v1/init` vì LoRA còn nằm trong decoder. Không có ghi chép GPU nào đã chạy được, nên không tính là bằng chứng cho RTX 3060.

## 3. ACE-Step đang cài

### 3.1 Thành phần trên đĩa [Runtime]

| Thành phần | Thư mục `checkpoints/` | Dung lượng | Đang nạp |
|---|---|---|---|
| DiT 2B turbo | `acestep-v15-turbo` | 4,5 GB | Có |
| LM planner | `acestep-5Hz-lm-1.7B` | 3,5 GB | **Không** (`llm_initialized=False`; khi bỏ qua init thì lazy-load cũng bị tắt, `acestep/api/startup_llm_init.py:103`) |
| Text encoder | `Qwen3-Embedding-0.6B` | 1,2 GB | Có |
| Audio VAE | `vae` | 322 MB | Có |

Không có trên đĩa: `base`, `sft`, các bản `xl-*`, LM 0.6B/4B. Tải thêm cần phê duyệt.
Máy: RTX 3060 (ứng dụng báo 11,6 GB), i5-13600K, RAM 30 GB, NVMe còn 1,7 TB. Server chạy `offload_to_cpu=True`. Venv: Python 3.12, torch 2.10.0+cu128, peft 0.18.1, lycoris_lora 3.4.0, lightning 2.6.1. **Chưa cài**: `bitsandbytes`, `prodigyopt`.

Giấy phép: code ACE-Step MIT (`LICENSE`); weights ghi MIT và cho phép dùng nhạc tạo ra cho mục đích thương mại (`checkpoints/README.md:3,31`); Qwen3-Embedding Apache-2.0 [Tài liệu]. Giấy phép riêng của LM và VAE: [Chưa rõ] ngoài thẻ MIT chung.

### 3.2 Hai trainer

| | Trainer tích hợp (`acestep/training/`) | Side-Step tích hợp (`train.py`, `acestep/training_v2/`) |
|---|---|---|
| Cách gọi | REST trên chính `acestep-api`: `/v1/training/{start,start_lokr,stop,status,export}`, `/v1/dataset/*` (`acestep/api/route_setup.py:112`); Gradio | CLI: `python train.py fixed\|vanilla\|estimate`; preprocess bằng `--preprocess` (`training_v2/cli/args.py:318-321`). Không có REST |
| Tiến trình | Chung với inference, dùng model đang nạp | Riêng, tự nạp model |
| Thành phần được train | Chỉ DiT decoder (`trainer.py:406`, `lora_injection.py:216-218`) | Chỉ DiT decoder |
| Timestep | Luôn lấy từ 8 giá trị cứng `TURBO_SHIFT3_TIMESTEPS` (`trainer.py:61-70`); tham số `training_shift` của REST **không có tác dụng** | `fixed`: logit-normal liên tục + CFG dropout 15% (`timestep_sampling.py:53-71`); `vanilla`: như trainer tích hợp |
| Variant | Thực chất chỉ đúng cho turbo | `--model-variant turbo\|base\|sft\|xl_*` |
| Tham số qua giao diện gọi | rank, alpha, dropout, lr, epochs, batch, grad accumulation, save mỗi N epoch, seed, fp8, gradient checkpointing (`train_api_models.py:16-29`) | Thêm: optimizer (adamw/adamw8bit/adafactor/prodigy), scheduler, precision, `--offload-encoder`, target modules, attention type, `--max-duration`, `--save-every` |
| Không có | optimizer/scheduler, target modules, validation split, **resume**, max steps (qua REST) | max steps, validation, early stopping, kiểm tra NaN/Inf; `--sample-every-n-epochs` có trong CLI nhưng trainer không dùng [Source] |
| Resume | Có trong code (`trainer.py:856-930`) nhưng REST không truyền | `--resume-from`: weights + optimizer + scheduler + epoch/step, **không có RNG** (mục 5.1). Checkpoint hỏng hoặc đường dẫn sai → in `[WARN] … starting fresh`, train lại từ 0, exit 0 [Source] |
| Gradient checkpointing | Mặc định tắt | Mặc định bật |
| VRAM công bố | Tối thiểu 16 GB, thường ~17 GB (`docs/en/LoRA_Training_Tutorial.md:7-8`) | ~7 GiB ở batch 1 (`docs/sidestep/Getting Started.md:93`); preset `vram_12gb`: rank 32, alpha 64, adamw8bit, offload encoder |
| Giới hạn đường dẫn | Mọi đường dẫn phải nằm dưới thư mục checkout (`acestep/training/path_safety.py:35`); `data/` của ta bị từ chối | `training_v2/` không gọi `safe_path`, **nhưng** nạp tensor bằng `PreprocessedDataModule` chung (`trainer_fixed.py:29` → `acestep/training/data_module.py:57`), nên thư mục tensor phải nằm dưới thư mục làm việc lúc khởi động. Chạy với cwd = `/mnt/ai-studio/AiStudio` thì `data/tensors/` được nhận [Đo]. Preprocess không bị giới hạn này |
| Tiến độ | `GET /v1/training/status`: step, loss, epoch (tách bằng regex), 1000 loss gần nhất; TensorBoard | stdout từng dòng khi không phải TTY: `Epoch 1/4, Step 3, Loss: 0.8997`, `[OK] Epoch 2/4 in 5.7s, Loss: 0.8244`, `[OK] Checkpoint saved at epoch 2` [Đo]. `Step` là **optimizer step** (`trainer_fixed.py`). TensorBoard: `train/loss`, `train/lr`, `train/epoch_loss` [Đo]. Bảng tổng kết cuối in `Epochs 0 / N` (sai, bỏ qua) |
| Dừng | `/v1/training/stop`, kiểm tra mỗi batch, **không lưu checkpoint khi dừng** (`trainer.py:944-952`) | Không có signal handler: SIGINT thoát mã 130, SIGTERM chết ngay, **không lưu** [Source; chưa thử thật]. Checkpoint chỉ ghi cuối epoch theo `--save-every` |
| Đầu ra | `final/adapter/` (thư mục PEFT); checkpoint `epoch_N_loss_x/adapter/` + `training_state.pt` | `adapter_config.json` + `adapter_model.safetensors` |
| Giấy phép | MIT | Nằm trong repo MIT, không có ghi chú giấy phép riêng [Source]. Bản **standalone** trên GitHub là CC BY-NC-SA 4.0 và đã bị đánh dấu deprecated [Tài liệu] — không dùng bản đó |

### 3.3 Dataset và preprocessing của ACE-Step [Source]

- Dataset JSON: `metadata` (name, `custom_tag`, `tag_position`, `all_instrumental`, `genre_ratio`) + `samples` (audio_path, caption, genre, lyrics mặc định `[Instrumental]`, bpm, keyscale, timesignature, duration, language, is_instrumental, labeled…) — `acestep/training/dataset_builder_modules/models.py:18-34,103-109`. Nhận file phụ `<tên>.caption.txt`, CSV cho bpm/key.
- Chỉ mẫu `labeled=True` được preprocess. Audio đưa về 48 kHz stereo, **cắt lấy 240 giây đầu**, không random crop (`preprocess_audio.py:17-19`). Caption tối đa 256 token.
- Tensor `.pt` chứa latent VAE, embedding text **và đầu ra condition encoder của DiT đang nạp** → tensor gắn với đúng variant DiT lúc preprocess.
- Auto-label cần LM 5Hz; LM đang tắt nên REST auto-label hiện trả 500. Tài liệu chính thức tự cảnh báo LM bịa BPM/key.

### 3.4 Nạp LoRA khi inference [Source]

`/v1/lora/{load,unload,toggle,scale,status}` (`acestep/api/http/lora_routes.py:60-132`). Trạng thái LoRA là toàn cục của server; `/release_task` không có trường LoRA. Nhận thư mục PEFT có `adapter_config.json` hoặc LoKR safetensors. **Không kiểm tra adapter khớp variant nào**. Từ chối trên model lượng tử hóa. Lần nạp đầu sao lưu toàn bộ decoder vào RAM.

## 4. Chọn backend và compatibility matrix

**Đề xuất: Side-Step tích hợp (`train.py fixed`) chạy như tiến trình con của worker.** Lý do:
- Tiến trình riêng → giải phóng VRAM bằng cách kết thúc tiến trình, không phụ thuộc việc server tự dọn. Trainer tích hợp để lại LoRA trong decoder và cần `/v1/init`.
- Có resume, gradient checkpointing mặc định, offload encoder, không bị giới hạn đường dẫn.
- Không sửa `ACE-Step-1.5/`; cùng venv, cùng commit với inference.

Cái giá: phải dừng `acestep-api` khi train (không tạo nhạc được trong lúc đó); không có validation loss; tiến độ lấy từ stdout (định dạng đã xác định ở E1) hoặc TensorBoard event file.

**Dự phòng**: trainer REST với `gradient_checkpointing=true`, dataset đặt dưới `ACE-Step-1.5/` — chỉ dùng nếu Side-Step không chạy được trên máy này.

| Mục | Giá trị | Mức kiểm chứng |
|---|---|---|
| ACE-Step commit | `ca1e85f` | [Runtime] |
| Checkpoint | `acestep-v15-turbo`; SHA-256 `model.safetensors` bắt đầu bằng `3f6e0797` | [Runtime] |
| Trainer | `training_v2` tại `ca1e85f`, chế độ `fixed` | [Đo] E1 |
| Định dạng adapter | PEFT LoRA: `adapter_config.json` + `adapter_model.safetensors` (rank 32: 44 MB) | [Đo] E1 |
| Target modules mặc định | Cả hai: `q_proj,k_proj,v_proj,o_proj`; Side-Step `--attention-type both`, rank 64, alpha 128 | [Source] |
| Precision | `--precision auto\|bf16\|fp16\|fp32`; không train trên DiT lượng tử hóa | [Source] |
| Preprocessing | `.pt` hai pass, gắn với variant. 8 bài × 60 s: 17 giây, VRAM đỉnh ~4,9 GB, 3,7 MB mỗi tensor | [Đo] E1 |
| Inference loader | `/v1/lora/load` nạp được thư mục `final/` của Side-Step; bật/tắt bằng `/v1/lora/toggle` | [Đo] E1 |
| Resume | `--resume-from` | [Đo] E1, có giới hạn (mục 5.1) |
| Validation / early stopping | Không có | [Source] |
| LoRA train trên turbo bằng timestep liên tục, dùng với inference 8 bước | Nạp và render được, output đổi | [Đo] E1 về mặt kỹ thuật; **chất lượng: chưa benchmark (E3)** |
| GPU | 12 GB | [Đo] E1 ở segment 60 s, rank 32, batch 1; cấu hình khác chưa benchmark |
| LoKR/DoRA/LoHA/OFT | Có trong code | Experimental cho tới khi train + inference đều chạy |

## 5. RTX 3060 12 GB: đã có gì, còn phải thử gì

Đã có bằng chứng [GPU thật]: inference turbo text/reference/cover (20–120 s audio); từ 2026-11-13 có thêm smoke test training E1 (mục 5.1).

Thí nghiệm (chạy theo thứ tự, mỗi cái cần người dùng đồng ý vì dừng dịch vụ tạo nhạc):

| # | Thí nghiệm | Đo | Tiêu chí qua |
|---|---|---|---|
| E0 | Reboot render node, xác nhận `nvidia-smi`, NVMe tự mount, hai dịch vụ tự chạy | — | **Xong 2026-11-13** |
| E1 | Khả thi: 8–10 bài có quyền, caption tay; preprocess → `train.py fixed` (preset gần `vram_12gb`, AdamW thường vì chưa có bitsandbytes) vài epoch → dừng → resume → nạp adapter bằng `/v1/lora/load` → tạo nhạc bật/tắt adapter cùng seed | VRAM/RAM đỉnh, dung lượng đĩa, giây/step, định dạng stdout, `step` đếm micro-batch hay optimizer step, kích thước adapter, NaN | **Xong 2026-11-13**, kết quả ở mục 5.1. Dataset thực dùng là 8 bài tự sinh, chỉ để đo kỹ thuật |
| E2 | Baseline trước training (mục 8): cấu hình production hiện tại trên bộ prompt chuẩn | Latency, VRAM, điểm nghe | Có bảng baseline để so |
| E3 | Rank (16/32/64), số epoch, độ dài segment; so `fixed` với `vanilla` | Loss, nghe mù | Chọn preset để công bố |
| E4 | (Cần tải ~4,7 GB, cần phê duyệt) train trên `base` hoặc `sft`, so với turbo | Nghe mù, thời gian | Quyết định base cho train |
| E5 | LM bật: VRAM, latency, chất lượng; auto-caption có dùng được cho nhạc không lời không | — | Quyết định có đưa LM vào pipeline |

### 5.1 Kết quả E0 + E1 (chạy thật 2026-11-13)

E1 là **smoke test kỹ thuật**. Nó không nói gì về chất lượng Style LoRA, và không chứng minh partial/full fine-tuning khả thi.

**E0 — reboot.** Kernel `7.0.0-43`, driver NVIDIA 595.99.02 khớp hai phía, `nvidia-smi` chạy, NVMe tự mount, `acestep-api` và `lush-worker` tự lên, worker tự đăng ký lại, tạo được một bài 30 giây (6 giây render). Màn hình desktop không tự đăng nhập sau reboot (dừng ở màn hình đăng nhập GDM).

**Môi trường E1.** RTX 3060 12 GB, i5-13600K, RAM 30 GB; Python 3.12, torch 2.10.0+cu128, peft 0.18.1; ACE-Step `ca1e85f`, checkpoint `acestep-v15-turbo` (SHA-256 `3f6e0797…`). `acestep-api` dừng trong lúc preprocess và train (khoảng 2,5 phút).

**Dataset kỹ thuật.** `data/datasets/e1-feasibility/v1`: 8 bài lo-fi instrumental × 60 giây (tổng 8 phút), do chính ACE-Step của studio tạo, không cắt segment (1 bài = 1 mẫu). Caption là prompt đã dùng để tạo bài; **chưa ai nghe để xác nhận caption đúng với audio**. Không dùng làm dataset thật.

**Conditioning instrumental.** `lyrics = "[Instrumental]"`; caption đặt trong khuôn `SFT_GEN_PROMPT` kèm `bpm`, `timesignature`, `keyscale`, `duration`; trường trống thành `N/A` (`training_v2/preprocess_prompt.py`). Inference dùng cùng quy ước: `is_instrumental()` coi lyrics rỗng, `[inst]`, `[instrumental]` là instrumental (`acestep/api/server_utils.py:135`), meta thiếu cũng thành `N/A` (`core/generation/handler/metadata_utils.py`). Worker của ta gửi đúng `lyrics: "[Instrumental]"`. Không có bằng chứng rằng quy ước này luôn ngăn được vocal.

**Cấu hình.** `train.py fixed`, LoRA rank 32 / alpha 64 / dropout 0,1, AdamW, lr 1e-4, cosine, warmup 2 step, batch 1, gradient accumulation 4, bf16, gradient checkpointing bật, `--offload-encoder`, seed 42, `--save-every 2`. 22.020.096 tham số được train.

| Hạng mục | Kết quả [Đo] |
|---|---|
| Preprocess | 8/8 bài, 17 giây (tính cả nạp model); VRAM đỉnh ~4,9 GB; RAM đỉnh 6,7 GB; 3,7 MB mỗi tensor |
| VRAM nền khi `acestep-api` tắt | 88 MiB, **đo lúc desktop chưa đăng nhập**. Mức nền khi desktop đang đăng nhập chưa đo trực tiếp |
| VRAM đỉnh khi train | 5.391 MiB toàn thiết bị (lấy mẫu `torch.cuda.mem_get_info()` mỗi 2 giây, gồm nền và tiến trình đo); trainer tự báo 4,9 GiB. Lấy mẫu 2 giây có thể sót đỉnh ngắn |
| RAM đỉnh khi train | 5,7 GB |
| Tốc độ | ~5,7 giây mỗi epoch = 8 micro-batch (mẫu 60 giây) = 2 optimizer step → ~0,7 giây/micro-batch, ~2,9 giây/optimizer step |
| Đã hoàn thành | Lần 1: 4 epoch, 8 optimizer step. Lần 2 (resume): epoch 5–8, step 9–16 |
| Loss | Hữu hạn ở cả 16 step (0,73–0,94). Không có xu hướng; với 8 mẫu và 16 step thì không kết luận được gì |
| Checkpoint | Ghi ở epoch 2, 4, 6, 8: adapter 44 MB + `training_state.pt` 88 MB. Trainer tự kiểm adapter: 100% tham số khác 0 |
| Resume | `[OK] Resumed from epoch 4, step 8, optimizer OK, scheduler OK`; tiếp tục đúng từ epoch 5, step 9 |
| Adapter vào inference | `/v1/lora/load` trả 200 với thư mục `final/`; render được |
| Bật/tắt adapter, cùng prompt, seed, 8 bước, 30 giây | Hai lần tắt: giống hệt nhau (sai khác 0). Prompt trong style: tương quan 0,922, RMS sai khác 0,054. Prompt ngoài style (orchestral): tương quan 0,992, RMS sai khác 0,018. Tắt lại bằng toggle: trở về giống hệt baseline |
| Khôi phục inference | `systemctl --user start` → `/health` sau ~10 giây. Cuối E1 restart `acestep-api` để gỡ adapter thử: `lora_loaded=false`, bài kiểm tra **trùng từng byte** với bài render trước E1 |
| Thời gian training đã dùng | 55 giây trên trần 1.800 giây (2 lần chạy, không retry) |
| Dung lượng | Thư mục E1 634 MB, tensor 29 MB |

Giới hạn và phát hiện:
- **RNG không được khôi phục khi resume** [Đo]: loss ở step 9 và 10 (0,8372; 0,7895) lặp lại loss ở step 1 và 2 (0,8372; 0,7896), tức chuỗi noise/timestep chạy lại từ đầu sau resume.
- **Resume với `--epochs` lớn hơn làm hỏng lịch learning rate** [Đo]: lần 1 LR giảm cosine về 1e-6 ở step 8; sau resume LR **tăng lại** từ 5,9e-6 lên 1e-4 ở step 15. Scheduler được khôi phục với tổng step của lần chạy cũ. Muốn resume đúng thì giữ nguyên `--epochs` của run gốc; "train thêm" phải là run mới.
- Output bật/tắt khác nhau chỉ chứng minh adapter có tác động. Chưa có đánh giá nghe; **chưa kiểm tra vocal contamination** (chưa ai nghe, chưa cài detector). File để nghe: `data/training/e1/eval/{in_off_a,in_on,out_off,out_on}.wav` trên node.
- Khi `acestep-api` tắt, worker đăng ký với `models=[]`; sau khi inference lên, model xuất hiện lại ở lần đăng ký định kỳ kế tiếp (tới ~60 giây). Thiết kế GPU lifecycle nên chủ động đăng ký lại ngay.
- `train.py fixed --preprocess` vẫn đòi `--dataset-dir` và `--output-dir` dù không dùng.
- Chưa thử: dừng giữa chừng, OOM, NaN, hết đĩa, mất mạng, segment dài hơn 60 giây, `adamw8bit`.
- Script và log: `data/training/e1/` trên node (`e1.sh`, `gen.py`, `train-a1.log`, `train-a2.log`, `vram-*.log`).

## 6. LoRA trước hay fine-tune DiT trước

LoRA trước. Căn cứ: (a) hai trainer có sẵn đều là LoRA/LoKR trên DiT decoder; (b) không có trainer full fine-tune chính thức nào đã merge — chỉ có hướng dẫn SFT quy mô lớn yêu cầu tối thiểu 8 × 40 GB (`docs/en/Large_Scale_SFT_Training_Guide.md:25-28`) và script bên thứ ba (DiffSynth-Studio) chưa kiểm; (c) dataset nội bộ hiện ở mức chục–trăm bài. "Train Model" trong Admin vì vậy chỉ bật phương án A (LoRA); B/C/D hiển thị điều kiện cần. Merge LoRA vào checkpoint không được gọi là fine-tune.

LM, text encoder, VAE giữ nguyên.

## 7. Thiết kế đề xuất

Tất cả mục 7–10 là **[Đề xuất]**.

### 7.1 Dataset và conditioning

- Lưu trên render node: `data/datasets/<dataset_key>/v<N>/` gồm `audio/` (bản gốc, không sửa), `manifest.json` (từng file: SHA-256, duration, nguồn, quyền sử dụng, nhóm tác phẩm, split), `captions/`. Tensor: `data/tensors/<dataset_key>-v<N>/<khóa cấu hình>/` với khóa = hash của (dataset version, SHA-256 checkpoint DiT/VAE/text encoder, commit ACE-Step, `max_duration`, chế độ preprocess).
- Metadata tách trường: `genre`, `instrumentation[]`, `mood[]`, `texture[]`, `bpm`, `key`, `time_signature`, mỗi trường có `source` (`human` / tên công cụ) và `unknown` khi không có bằng chứng. Không lưu confidence nếu công cụ không trả.
- Caption sinh từ metadata + chỉnh tay, đúng định dạng ACE-Step nhận: `caption` ≤ 256 token, `lyrics = [Instrumental]`, `custom_tag` (trigger) nếu dùng. Bộ từ vựng caption trùng với cách `compose_prompt` ghép prompt lúc tạo nhạc (`routes/jobs.rs:84`) để điều kiện lúc train và lúc dùng khớp nhau.
- Split theo nhóm tác phẩm/bản thu trước khi cắt segment; remaster/alternate mix cùng nhóm. ACE-Step chỉ lấy 240 giây đầu, nên bài dài phải cắt segment ở bước chuẩn bị và ghi overlap.
- Version đã dùng cho một lần train là bất biến; sửa dữ liệu hoặc caption tạo version mới.
- Phân tích: FFmpeg (định dạng, loudness, clipping, im lặng), trùng lặp theo SHA-256 và fingerprint; BPM/key bằng công cụ phân tích audio, không lấy từ LM. Lựa chọn công cụ cụ thể để tới M2.
- Vận chuyển: xem Q3 (mục 11). M1 dùng thư mục đã chuẩn bị sẵn trên node.

### 7.2 Hợp đồng và schema (chỉ phần M1)

- Job type mới `train_lora` trên bảng `jobs` hiện có (D6). `max_attempts = 1`: training không tự chạy lại; resume là thao tác chủ động, tạo attempt mới.
- Worker báo `job_types` theo cấu hình thật (`train_lora` chỉ khi trainer được cấu hình) và báo danh sách dataset version có trên node khi register, giống cách báo LoRA.
- `POST /jobs/{id}/progress` thêm trường tùy chọn `metrics` (mảng sự kiện). Heartbeat giữ nguyên, tách khỏi metrics.
- Migration `0004_training.sql`: bảng `training_metrics` (job_id, attempt, sequence, global_step, epoch, train_loss, learning_rate, elapsed_seconds, gpu_memory_used, recorded_at); bảng `node_datasets`; thêm cột cho `loras` (job_id, dataset_key + version, trainer, config JSON, adapter SHA-256, created_by).
- `complete` của `train_lora` tạo dòng `loras` ở trạng thái `evaluation`. Không tự `approved`, không đổi preset nào.

### 7.3 Sự kiện tiến độ

Trainer adapter phát: `run_id` (= job id), `attempt_id`, `sequence`, `timestamp`, `stage`, `epoch_current/total`, `global_step`, `max_steps`, `train_loss`, `validation_loss`, `learning_rate`, `elapsed_seconds`, `estimated_remaining_seconds`, `gpu_memory_used/peak`, `checkpoint_id`, `error_code/message`. Trường backend không có để `null`; với Side-Step `validation_loss` và `max_steps` luôn `null` → web hiện "Không hỗ trợ".

- `global_step` là micro-batch hay optimizer step: ghi theo kết quả E1, hiển thị đúng tên.
- Loss chỉ lấy từ sự kiện có cấu trúc của trainer (TensorBoard scalar hoặc dòng stdout đã xác định định dạng), kèm đúng step.
- Lưu trữ: worker gộp sự kiện, gửi tối đa một lô mỗi ~10 giây; API giữ tối đa N điểm mỗi attempt (downsample khi vượt), log đầy đủ nằm trên node và tải lên như artifact khi kết thúc.
- Web poll như hiện tại. Quá 2 chu kỳ heartbeat không có cập nhật → "Mất cập nhật từ …", không đổi trạng thái run.
- Checkpoint chỉ được ghi nhận sau khi file ghi xong và kiểm SHA-256; thư mục checkpoint theo `attempt` để attempt sau không ghi đè.

### 7.4 Trạng thái

Run: `queued → preparing → training → saving → evaluating → completed | failed | cancelled` (map vào `status` + `stage` của job; không thêm state machine thứ hai). Lưu ý: API hiện chỉ nhận `stage` là `preparing`/`running` (`routes/worker.rs:293-297`), stage khác bị bỏ qua — phải mở rộng khi làm M1.
Candidate (dòng `loras`): `evaluation → approved | archived` (đã có trong schema), độc lập với run.
Triển khai: LoRA nào đang là mặc định của preset — thuộc `style_presets`.

### 7.5 Vòng đời GPU

Một tiến trình GPU tại một thời điểm, tính cả `acestep-api`:

1. Worker nhận `train_lora` (node chỉ giữ một job nên không còn job generate nào chạy).
2. Dừng `acestep-api` (`systemctl --user stop`), chờ tiến trình thoát, xác nhận không còn tiến trình Python nào giữ GPU. Dùng `nvidia-smi --query-compute-apps` khi NVML chạy; dự phòng khi NVML hỏng: quét `/proc/<pid>/fd` tìm `/dev/nvidia*` và `torch.cuda.mem_get_info()` (cả hai đã thử thật). VRAM nền không về 0 vì màn hình cũng dùng GPU (mục 5.1).
3. Preflight: dataset version + checksum, SHA-256 checkpoint, dung lượng đĩa, VRAM trống.
4. Preprocess (bỏ qua nếu cache tensor khớp khóa) → train → lưu.
5. Kết thúc tiến trình trainer, xác nhận VRAM trống.
6. Khởi động lại `acestep-api`, chờ `/health`, nạp thử adapter, rồi mới `complete`.

Lỗi: OOM / NaN / trainer crash → `fail` không retry, kèm `error_code`, vẫn chạy bước 5–6. Cancel → gửi tín hiệu, chờ, bước 5–6. Worker bị restart khi đang train → khi khởi động lại đọc pidfile của run, kết thúc trainer mồ côi rồi mới register. Bước 6 thất bại → node báo engine không khỏe, không nhận job, hiện cảnh báo cho Admin; worker **không** tự reboot hay đổi driver. Mất mạng → trainer vẫn chạy, worker gửi lại khi có mạng; lease hết thì job thành `failed` phía VPS nhưng checkpoint còn trên node để resume.

Người dùng đã duyệt hướng này ngày 2026-11-13 (D21): khi train thì dừng tạo nhạc; job generate gửi trong lúc đó nằm chờ trong queue. UI phải báo trước khi tạo run và trong lúc train.

### 7.6 Admin UI

`Admin → AI Training`: Tổng quan · Datasets · Train Style LoRA · Train Model · Experiments · Đánh giá A/B · Models & LoRA. Dùng lại `ui.tsx`, `Modal`, `JobCard`, bảng của các trang admin hiện có; thay ba `ComingSoonPage`. Mỗi màn có đủ trạng thái empty / loading / queued / running / mất cập nhật / success / failed / cancelled / unsupported. Wizard Train Style LoRA 10 bước như yêu cầu; phần Advanced chỉ hiện tham số trainer adapter khai báo. Không công bố preset "Quick Test / Balanced / Low VRAM" trước E3. Repo chưa có `docs/UI_SYSTEM.md`.

### 7.7 Quyền và storage

Dùng role sẵn có: `admin` và `trainer` tạo dataset/run; `admin` duyệt, đặt mặc định, archive, xóa. Kiểm tra ở backend. Adapter chỉ nạp định dạng safetensors/PEFT do chính hệ thống train ra; không nhận upload pickle.
Trên NVMe: `data/datasets/` (gốc), `data/tensors/`, `data/training/<run>/attempt-<n>/` (checkpoint, optimizer state, log), `data/loras/<key>/` (adapter đã đăng ký), `data/eval/`. VPS chỉ giữ metadata, metrics đã rút gọn, log và audio đánh giá.

## 8. Baseline và evaluation

Baseline (E2) trước mọi lần train: bộ prompt chuẩn có version, độc lập với dataset train, phủ Jazz, Lo-fi, Piano, Ambient, Cinematic và vài prompt ngoài style; mỗi prompt ≥ 3 seed cố định; 30 s và 120 s; ghi latency, VRAM, steps/guidance thực gửi. Lưu ý turbo chạy 8 bước và không dùng CFG [Tài liệu] — phải xác nhận `guidance_scale` có tác dụng hay không trước khi đưa vào ma trận.

Đánh giá chung cho Model/LoRA:
- **Kỹ thuật** (tự động): toàn vẹn file, im lặng, clipping, loudness, duration.
- **Âm nhạc + con người**: nghe mù A/B (adapter bật/tắt, cùng checkpoint, cùng seed, thứ tự ngẫu nhiên), rubric: đúng style, giai điệu, hòa âm, nhịp, phối khí, cấu trúc, lặp, chất âm. Mỗi người chấm riêng; báo cáo số mẫu, số người, tỷ lệ thắng kèm khoảng tin cậy. Với 2–4 người và vài chục cặp, kết quả chỉ đủ để phát hiện khác biệt lớn.
- **Regression**: prompt ngoài style với adapter bật.
- **Ghi nhớ dữ liệu train**: so output với bài train (fingerprint / nghe) khi khả thi.
- Chọn checkpoint: Side-Step không có validation loss → không có "best validation checkpoint" tự động; chọn bằng nghe mù trên các checkpoint theo epoch. Loss thấp nhất không phải tiêu chí chọn.
- Tham chiếu Suno chỉ dùng khi có quyền, ghi model/ngày/prompt/cách chọn. Không cam kết đạt chất lượng Suno.

## 9. Cloud và chi phí

Cần cloud khi: E1 cho thấy 12 GB không train được ở cấu hình chấp nhận được; hoặc thời gian train chặn việc tạo nhạc hằng ngày quá lâu; hoặc muốn thử fine-tune DiT. Cách làm: Admin thuê tay, cài worker, node tự đăng ký như render node thứ hai (scheduler đã theo capability). Chưa làm provision/billing tự động.

Ước lượng (giả định ghi rõ, chưa đo):
- Lưu trữ: WAV 48 kHz stereo 16-bit ≈ 11,5 MB/phút → 100 bài × 3 phút ≈ 3,5 GB; 1.000 bài ≈ 35 GB. NVMe còn 1,7 TB nên chưa là giới hạn. Kích thước tensor, adapter, checkpoint: đo ở E1.
- Thời gian train trên 3060: **không ước lượng** trước E1.
- Giá thuê (RunPod, đọc ngày 2026-11-12): RTX 4090 24 GB 0,34–0,89 USD/giờ; A6000 48 GB 0,33–0,59; A100 80 GB 1,19–1,79. Một lần train LoRA 10 giờ trên 4090 ≈ 3,4–8,9 USD. Full SFT theo cấu hình tối thiểu chính thức (8 GPU): cỡ 10–14 USD/giờ với A100 80 GB; số giờ chưa có nguồn.

## 10. Milestones

| | Phạm vi | Điều kiện xong |
|---|---|---|
| M0 | Tài liệu này | Người dùng chọn milestone |
| M1 | LoRA vertical slice (chi tiết dưới) | Train thật trên 3060 hoặc kết luận có bằng chứng là không thể |
| M2 | Dataset platform: import, validate, caption review, split, version bất biến, cache tensor | Tạo được dataset version từ web và train từ đó |
| M3 | Vận hành: metrics đầy đủ, log, cancel, resume, phục hồi lỗi mục 7.5 | Diễn tập từng lỗi trên node thật |
| M4 | Đánh giá mù A/B, chọn checkpoint, version, rollback | Một LoRA đi hết vòng tới `approved` |
| M5 | Fine-tune DiT, cloud worker thủ công | Có trainer đã kiểm + compute |
| M6 | LM conditioning, long-form | Khi có bằng chứng cần |

### M1 — file sẽ sửa

Điều kiện trước: E0 và E1 đã xong (2026-11-13). Danh sách dưới đây là bản M0; phần bổ sung sau rà soát nằm ở mục 12.4. Chưa bắt đầu code: chờ người dùng duyệt.

| Lớp | File | Việc |
|---|---|---|
| API | `apps/api/migrations/0004_training.sql` (mới) | Mục 7.2 |
| | `apps/api/src/routes/training.rs` (mới), `routes/mod.rs`, `lib.rs` | `POST /training-runs`, `GET /training-runs[/{id}]`, `GET /training-runs/{id}/metrics`, `GET /datasets` |
| | `apps/api/src/routes/worker.rs` | register nhận datasets; progress nhận `metrics`; `complete` cho `train_lora` |
| | `apps/api/src/routes/registry.rs` | `PATCH /loras/{id}` (status, name) — admin |
| | `apps/api/src/routes/jobs.rs` | `insert_job` nhận `max_attempts` |
| | `apps/api/tests/api.rs` | Test với worker giả |
| Worker | `src/lush_worker/trainers/{__init__,base,sidestep}.py` (mới) | Trainer adapter + khai báo capability, parser tiến độ |
| | `src/lush_worker/gpu_lifecycle.py` (mới) | Mục 7.5 |
| | `runner.py`, `config.py`, `api_client.py`, `gpu.py` | Dispatch theo job type; `DATASET_DIR`, `TRAINING_DIR`, `ACESTEP_DIR`; gửi metrics |
| | `tests/test_sidestep_trainer.py`, `tests/test_gpu_lifecycle.py` (mới), `tests/test_runner.py` | Unit với tiến trình giả |
| Web | `src/api/{types,api}.ts`, `src/pages/admin/training/*` (mới), `App.tsx`, `components/Sidebar.tsx` | Form tạo run, danh sách, chi tiết + biểu đồ loss, Models & LoRA tối thiểu |
| Infra | `infrastructure/render-node/install.sh` | Tạo thư mục `data/tensors`, `data/eval` |
| Docs | `API_CONTRACTS.md`, `WORKER_PROTOCOL.md`, `DATABASE.md`, `TESTING.md`, `DECISIONS.md`, file này | Cùng thay đổi |

Tiêu chí M1: Admin chọn dataset có sẵn trên node → tạo run → train thật → dừng và resume → adapter vào registry ở `evaluation` → tạo nhạc bật/tắt adapter → so với baseline E2. Rollback: tắt job type `train_lora` trong cấu hình worker; migration chỉ thêm bảng/cột.

## 11. Giới hạn và câu hỏi mở

Chưa đủ bằng chứng (sau E1):
- VRAM và tốc độ với segment dài hơn 60 s, rank lớn hơn, dataset thật; thời gian của một run có ý nghĩa.
- LoRA có học được style hay không, và có hợp với turbo 8 bước về chất lượng không (E3).
- Hành vi thật khi gửi tín hiệu dừng, khi OOM, khi loss NaN (mới đọc source).
- Mở run mới chỉ từ adapter weights (`--resume-from` vào thư mục không có `training_state.pt`).
- Chất lượng: chưa có đánh giá nghe nào, kể cả cho model gốc. Không có cơ sở để hứa mức chất lượng sau fine-tune.
- LoRA trên segment ≤ 240 s không chứng minh được việc học cấu trúc bài dài.
- `guidance_scale` trên turbo, tác dụng của LM với nhạc không lời.
- Giấy phép riêng của LM/VAE.

Cần người dùng quyết định:
- **Q1.** ~~Cho phép worker tự dừng/khởi động `acestep-api` trong lúc train?~~ **Đã chốt 2026-11-13: có, tạm thời thiết kế như vậy (D21).** Chạy tay E1 vẫn cần đồng ý riêng.
- **Q2.** ~~Reboot render node lúc nào (E0)?~~ **Xong 2026-11-13.**
- **Q3.** **Đã chốt 2026-11-13: thư mục import trên node (a) cho giai đoạn đầu, không cài thêm dịch vụ chia sẻ; sản phẩm chính thức có upload/import và tiến trình trong Admin (thiết kế sau).** Các phương án đã xét: (a) chép vào thư mục import trên node/NAS rồi bấm Import trong Admin; (b) upload từ trình duyệt qua VPS (trái D9, tốn đĩa VPS); (c) upload thẳng tới node qua Tailscale (máy văn phòng phải cài Tailscale).
- **Q4.** **Đã chốt:** dataset E1 chỉ để kiểm tra kỹ thuật, không đăng ký làm dataset thật. Dataset thật do người dùng cung cấp kèm nguồn và quyền; không tự tải nhạc trên mạng; dữ liệu tự sinh không mặc định là có quyền train.
- **Q5.** **Hoãn.** Chỉ đề xuất lại khi có mục tiêu so sánh cụ thể, kèm dung lượng, tài nguyên và license.

## 12. Phạm vi đã chốt và thiết kế bổ sung (2026-11-13)

Mục này ghi kết quả rà soát sau M0 và các quyết định người dùng đã chốt. Chỗ nào khác với mục 7–10 thì theo mục này. Tất cả là **[Đề xuất]** trừ khi ghi nhãn khác; chưa có code.

### 12.1 Phạm vi (D22)

- Chỉ tạo và train nhạc **instrumental**. Không làm lyrics editor, voice identity, chọn ca sĩ hay tham số train vocal trong Training UI. Không xoá hay sửa tính năng vocal có sẵn ngoài phạm vi training.
- Chỉ RTX 3060 12 GB; không cloud. Ưu tiên Style LoRA.
- Bốn mức, không gọi lẫn: LoRA/LoKR adapter (có trainer, E1 đã chạy) · partial fine-tuning (không có trainer cho `ca1e85f`) · full fine-tuning DiT (upstream ghi "NOT natively supported", tối thiểu 8 × 40 GB [Tài liệu]) · train toàn hệ thống (không có). Chỉ đề xuất partial/full khi có trainer tương thích **và** bằng chứng tài nguyên.

### 12.2 Dataset instrumental

- Chỉ nhạc instrumental có quyền sử dụng do người dùng xác nhận. Giữ audio gốc.
- File bị detector nghi có singing, speech, humming hoặc vocal chops: **gắn cờ, người nghe xác nhận**. Detector không tự kết luận có hay không có vocal.
- Không tự tách vocal rồi coi phần còn lại là dữ liệu đạt chuẩn.
- Split train / validation / test theo `work_group` + `source_group` trước khi cắt segment; kiểm gần trùng chéo split (fingerprint) trước khi khoá version. Test set chỉ dùng cho lần đánh giá cuối.
- Caption tập trung: genre/subgenre, instruments, mood/energy/texture, tempo/rhythm, harmony, dynamics, structure. Trường chưa có bằng chứng lưu `unknown`; khi sinh conditioning text thì **bỏ trường đó khỏi caption**, và để `bpm`/`keyscale`/`timesignature` trống cho upstream tự ghi `N/A` (mục 5.1). Không sinh lyrics hay mô tả giọng hát.
- Ba loại văn bản tách riêng: training caption (mô tả audio) · benchmark prompt (viết độc lập, có version) · chỉ dẫn cho coding agent (`AGENTS.md`, không vào dataset).
- Công cụ dự kiến (chưa tải, chưa tích hợp; license theo trang gốc đọc 2026-11-13 [Tài liệu]): `ffmpeg`/`ffprobe` (LGPL), `pyloudnorm` (MIT), `librosa` (ISC), `beat_this` (MIT), Chromaprint `fpcalc` (LGPL), PANNs Cnn14 (code MIT, weights CC BY 4.0) hoặc AST (BSD-3) cho cờ vocal/speech. Tránh: Essentia (AGPL, model NC), madmom (model NC), Demucs (repo đã archive, license weights chưa xác minh).
- Caption tự động (giai đoạn sau): ứng viên `ACE-Step/acestep-captioner` (thẻ MIT; chỉ có bản GGUF cộng đồng vừa 12 GB) — phải báo nguồn, checkpoint, license, dung lượng, runtime và khả năng chạy trước khi tải. Mọi caption tự động phải sửa và duyệt được.

### 12.3 Đánh giá instrumental

- Bộ prompt benchmark chỉ gồm yêu cầu instrumental, có cả prompt trong style và ngoài style.
- Tiêu chí: melody, harmony, rhythm, arrangement, timbre, structure và long-range consistency, repetition và diversity, prompt/style adherence, audio fidelity, **vocal contamination ngoài yêu cầu**. Không có tiêu chí chất lượng giọng hát. Output nghi có vocal phải được nghe lại.
- Vai trò model chấm (chưa tải): CLAP đo text–audio alignment; Audiobox Aesthetics cho tín hiệu chất lượng trong phạm vi nó hỗ trợ; PANNs gắn cờ vocal/speech. Không cái nào là bằng chứng nhạc hay. Điểm máy chỉ để sàng lọc và sắp thứ tự nghe, và phải đối chiếu với A/B mù của nhóm.
- Không có validation loss (mục 3.2) nên chọn checkpoint bằng: render bộ prompt validation cố định nhiều seed cho từng checkpoint → chấm kỹ thuật tự động → điểm theo từng tiêu chí → sàng lọc memorization (fingerprint + khoảng cách embedding tới train so với validation) → nghe A/B mù so baseline. Admin nghe cả mẫu kém trước khi duyệt.

### 12.4 Bổ sung so với danh sách M1 (mục 10)

- **Worker:** chạy trainer với cwd là `LUSH_ROOT` (tránh `safe_path`); preprocess và train là hai lệnh; bắt loss không hữu hạn; kiểm dòng `Resumed from epoch`, coi `starting fresh` là lỗi; chỉ đăng ký checkpoint sau dòng `Checkpoint saved`, đủ file, có SHA-256; đồng hồ ngân sách; resume giữ nguyên `--epochs`; đăng ký lại ngay sau khi inference lên; `run.json` bất biến cho mỗi run.
- **API:** lưu `stage` tự do cho job train; `gpu_activity` trong queue summary; job train chỉ được claim khi không còn job generate chờ; khung giờ / lịch train do Admin đặt để train không chờ vô hạn; sổ ngân sách cộng dồn qua resume/retry; generate nhận LoRA `evaluation` cho job đánh giá; so hash base khi dùng LoRA.
- **Database:** `training_runs`, `training_checkpoints`, hash base / commit ACE-Step / provenance trên `loras`; `loras.key` đang `UNIQUE` nên mỗi phiên bản dùng key riêng.
- **Web:** banner "node đang training" và câu chờ trên job generate (không hứa giờ hoạt động lại từ trần training, vì còn thời gian lưu checkpoint và bật lại inference); khối giải thích bài / epoch / optimizer step / micro-batch; trường trainer không cấp hiện "Không hỗ trợ" hoặc "Chưa có dữ liệu"; trạng thái "Mất cập nhật". Polling như hiện tại (web không có SSE/WebSocket).
- **Hạ tầng:** `scripts/deploy.sh` từ chối khi node đang train (hiện deploy luôn restart `lush-worker`, trainer con sẽ bị giết theo).
- **Lease cho training** (chưa làm, xem phần đề xuất trong `WORKER_PROTOCOL.md`): tách heartbeat, lease và training progress; ngăn hai worker chạy trùng một run bằng fencing token theo attempt, không chỉ tăng timeout.
- **Experiment tuần tự tối thiểu:** Admin duyệt trước danh sách cấu hình, đặt số lần thử, tổng GPU-hours, steps/epochs, trần dung lượng checkpoint, điều kiện dừng khi lỗi; ngân sách cộng dồn; không tự dùng output làm dataset, không tự approve/deploy. Chỉ làm sau khi một run và một evaluation đơn lẻ chạy ổn.
- **RBAC:** `trainer` quản lý dataset, tạo run, xem kết quả; chỉ `admin` duyệt adapter dùng chung. Thực thi ở backend.

### 12.5 Các quyết định khác đã chốt

- `bitsandbytes`: chỉ cài khi thiếu VRAM, trong môi trường training riêng, không đổi môi trường inference.
- Desktop trên node: giữ nguyên; chỉ đề xuất tắt khi số đo cho thấy cần.
- Tải `base`/`sft`, captioner, model chấm điểm: chưa; mỗi lần tải cần báo trước và được duyệt.
