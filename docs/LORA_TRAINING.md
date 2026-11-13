# LORA_TRAINING — Audit và thiết kế AI Training (M0)

Ngày: 2026-11-12. Trạng thái: **M0 — audit + thiết kế đề xuất. Chưa có dòng code training nào trong sản phẩm, chưa chạy training nào.**
Source audit: branch `main`, HEAD `4118c6b` + thay đổi T-007 chưa commit (Reference/Cover, giữ nguyên). ACE-Step trên `render-node-01`: commit `ca1e85f` (2026-08-29), working tree sạch.

Quy ước nhãn trong tài liệu này:
- **[Runtime]** đọc/đo trực tiếp trên máy thật hôm nay. **[Source]** đọc từ code tại commit nêu trên. **[Tài liệu]** bên thứ ba tự công bố, ta chưa kiểm. **[Đề xuất]** thiết kế chưa làm. **[Chưa rõ]** chưa có bằng chứng.
- Đường dẫn `acestep/...`, `train.py`, `docs/...` không có tiền tố là trong `/mnt/ai-studio/AiStudio/ACE-Step-1.5` trên render node.

## 1. Kết luận chính

1. Nền tảng hiện tại (queue, lease, worker protocol, registry, RBAC) dùng lại được cho training; **không cần đổi kiến trúc**. Phần thiếu là toàn bộ nghiệp vụ training: job type, trainer adapter, vòng đời GPU, metrics, dataset, đánh giá.
2. ACE-Step `ca1e85f` có **hai trainer** [Source]: trainer tích hợp (chạy trong tiến trình `acestep-api`, có REST) và **Side-Step bản tích hợp upstream** (`train.py`, `acestep/training_v2/`, chỉ CLI, tiến trình riêng). Đề xuất dùng Side-Step CLI làm backend chính, trainer REST làm phương án dự phòng (mục 4).
3. **Chưa có bằng chứng nào** cho việc train trên RTX 3060 12 GB. Tài liệu chính thức mâu thuẫn nhau (16 GB tối thiểu cho trainer tích hợp; ~7 GiB cho Side-Step có gradient checkpointing). Phải đo bằng thí nghiệm khả thi E1 trước khi viết UI.
4. LoRA trước, fine-tune DiT sau. Full SFT theo hướng dẫn chính thức cần tối thiểu 8 GPU × 40 GB [Tài liệu] — ngoài tầm máy hiện tại, chỉ xét khi LoRA đã chứng minh giới hạn.
5. **Blocker trước mọi thí nghiệm GPU**: render node lệch driver NVIDIA (userspace 595.99, kernel module đang nạp 595.91, có `/var/run/reboot-required`) [Runtime]. Tiến trình CUDA mới nhiều khả năng không khởi tạo được cho tới khi reboot. Reboot cần người dùng đồng ý.

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
| Không có | optimizer/scheduler, target modules, validation split, **resume**, max steps (qua REST) | max steps, validation, early stopping |
| Resume | Có trong code (`trainer.py:856-930`) nhưng REST không truyền | `--resume-from` (weights + optimizer + scheduler) |
| Gradient checkpointing | Mặc định tắt | Mặc định bật |
| VRAM công bố | Tối thiểu 16 GB, thường ~17 GB (`docs/en/LoRA_Training_Tutorial.md:7-8`) | ~7 GiB ở batch 1 (`docs/sidestep/Getting Started.md:93`); preset `vram_12gb`: rank 32, alpha 64, adamw8bit, offload encoder |
| Giới hạn đường dẫn | Mọi đường dẫn phải nằm dưới thư mục checkout (`acestep/training/path_safety.py:35`); `data/` của ta bị từ chối | Không dùng `safe_path` (0 lần gọi trong `training_v2/`) |
| Tiến độ | `GET /v1/training/status`: step, loss, epoch (tách bằng regex), 1000 loss gần nhất; TensorBoard | Generator `TrainingUpdate(step, loss, msg, kind)`; TensorBoard. Định dạng stdout khi chạy không tương tác: [Chưa rõ] |
| Dừng | `/v1/training/stop`, kiểm tra mỗi batch, **không lưu checkpoint khi dừng** (`trainer.py:944-952`) | Tín hiệu tiến trình; hành vi lưu khi dừng: [Chưa rõ] |
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

Cái giá: phải dừng `acestep-api` khi train (không tạo nhạc được trong lúc đó); không có validation loss; tiến độ phải lấy từ stdout hoặc TensorBoard event file (cần E1 xác định).

**Dự phòng**: trainer REST với `gradient_checkpointing=true`, dataset đặt dưới `ACE-Step-1.5/` — chỉ dùng nếu Side-Step không chạy được trên máy này.

| Mục | Giá trị | Mức kiểm chứng |
|---|---|---|
| ACE-Step commit | `ca1e85f` | [Runtime] |
| Checkpoint | `acestep-v15-turbo`; hash file chưa tính | ghi SHA-256 ở E1 |
| Trainer | `training_v2` tại `ca1e85f`, chế độ `fixed` | [Source], chưa chạy |
| Định dạng adapter | PEFT LoRA (safetensors) | [Source] |
| Target modules mặc định | Trainer tích hợp: `q_proj,k_proj,v_proj,o_proj`; Side-Step: theo `--target-modules`/`--attention-type`, mặc định chưa đọc | [Chưa rõ] một phần |
| Precision | `--precision auto\|bf16\|fp16\|fp32`; không train trên DiT lượng tử hóa | [Source] |
| Preprocessing | `.pt` hai pass (~3 GB, ~6 GB VRAM), gắn với variant | [Tài liệu], chưa đo |
| Inference loader | `/v1/lora/load` nhận thư mục PEFT; adapter Side-Step nạp được hay không | **[Chưa rõ] — E1 phải chứng minh** |
| Resume | `--resume-from` | [Source], chưa chạy |
| Validation / early stopping | Không có | [Source] |
| LoRA train trên turbo bằng timestep liên tục, dùng với inference 8 bước | — | **[Chưa rõ] — giả thuyết cần E1/E3** |
| GPU | 12 GB | **[Chưa rõ]** |
| LoKR/DoRA/LoHA/OFT | Có trong code | Experimental cho tới khi train + inference đều chạy |

## 5. RTX 3060 12 GB: đã có gì, còn phải thử gì

Đã có bằng chứng [GPU thật]: inference turbo text/reference/cover (20–120 s audio). **Chưa có** bằng chứng cho bất kỳ bước training nào, kể cả preprocess.

Thí nghiệm (chạy theo thứ tự, mỗi cái cần người dùng đồng ý vì dừng dịch vụ tạo nhạc):

| # | Thí nghiệm | Đo | Tiêu chí qua |
|---|---|---|---|
| E0 | Reboot render node, xác nhận `nvidia-smi`, NVMe tự mount, hai dịch vụ tự chạy | — | Điều kiện tiên quyết |
| E1 | Khả thi: 8–10 bài có quyền, caption tay; preprocess → `train.py fixed` (preset gần `vram_12gb`, AdamW thường vì chưa có bitsandbytes) vài epoch → dừng → resume → nạp adapter bằng `/v1/lora/load` → tạo nhạc bật/tắt adapter cùng seed | VRAM/RAM đỉnh, dung lượng đĩa, giây/step, định dạng stdout, `step` đếm micro-batch hay optimizer step, kích thước adapter, NaN | Train không OOM, resume đúng step, adapter nạp được, output bật/tắt khác nhau |
| E2 | Baseline trước training (mục 11): cấu hình production hiện tại trên bộ prompt chuẩn | Latency, VRAM, điểm nghe | Có bảng baseline để so |
| E3 | Rank (16/32/64), số epoch, độ dài segment; so `fixed` với `vanilla` | Loss, nghe mù | Chọn preset để công bố |
| E4 | (Cần tải ~4,7 GB, cần phê duyệt) train trên `base` hoặc `sft`, so với turbo | Nghe mù, thời gian | Quyết định base cho train |
| E5 | LM bật: VRAM, latency, chất lượng; auto-caption có dùng được cho nhạc không lời không | — | Quyết định có đưa LM vào pipeline |

## 6. LoRA trước hay fine-tune DiT trước

LoRA trước. Căn cứ: (a) hai trainer có sẵn đều là LoRA/LoKR trên DiT decoder; (b) không có trainer full fine-tune chính thức nào đã merge — chỉ có hướng dẫn SFT quy mô lớn yêu cầu tối thiểu 8 × 40 GB (`docs/en/Large_Scale_SFT_Training_Guide.md:25-28`) và script bên thứ ba (DiffSynth-Studio) chưa kiểm; (c) dataset nội bộ hiện ở mức chục–trăm bài. "Train Model" trong Admin vì vậy chỉ bật phương án A (LoRA); B/C/D hiển thị điều kiện cần. Merge LoRA vào checkpoint không được gọi là fine-tune.

LM, text encoder, VAE giữ nguyên.

## 7. Thiết kế đề xuất

Tất cả mục 7–12 là **[Đề xuất]**.

### 7.1 Dataset và conditioning

- Lưu trên render node: `data/datasets/<dataset_key>/v<N>/` gồm `audio/` (bản gốc, không sửa), `manifest.json` (từng file: SHA-256, duration, nguồn, quyền sử dụng, nhóm tác phẩm, split), `captions/`. Tensor: `data/tensors/<dataset_key>-v<N>/<khóa cấu hình>/` với khóa = hash của (dataset version, SHA-256 checkpoint DiT/VAE/text encoder, commit ACE-Step, `max_duration`, chế độ preprocess).
- Metadata tách trường: `genre`, `instrumentation[]`, `mood[]`, `texture[]`, `bpm`, `key`, `time_signature`, mỗi trường có `source` (`human` / tên công cụ) và `unknown` khi không có bằng chứng. Không lưu confidence nếu công cụ không trả.
- Caption sinh từ metadata + chỉnh tay, đúng định dạng ACE-Step nhận: `caption` ≤ 256 token, `lyrics = [Instrumental]`, `custom_tag` (trigger) nếu dùng. Bộ từ vựng caption trùng với cách `compose_prompt` ghép prompt lúc tạo nhạc (`routes/jobs.rs:84`) để điều kiện lúc train và lúc dùng khớp nhau.
- Split theo nhóm tác phẩm/bản thu trước khi cắt segment; remaster/alternate mix cùng nhóm. ACE-Step chỉ lấy 240 giây đầu, nên bài dài phải cắt segment ở bước chuẩn bị và ghi overlap.
- Version đã dùng cho một lần train là bất biến; sửa dữ liệu hoặc caption tạo version mới.
- Phân tích: FFmpeg (định dạng, loudness, clipping, im lặng), trùng lặp theo SHA-256 và fingerprint; BPM/key bằng công cụ phân tích audio, không lấy từ LM. Lựa chọn công cụ cụ thể để tới M2.
- Vận chuyển: xem câu hỏi mở Q3 (mục 14). M1 dùng thư mục đã chuẩn bị sẵn trên node.

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

Run: `queued → preparing → training → saving → evaluating → completed | failed | cancelled` (map vào `status` + `stage` hiện có của job; không thêm state machine thứ hai).
Candidate (dòng `loras`): `evaluation → approved | archived` (đã có trong schema), độc lập với run.
Triển khai: LoRA nào đang là mặc định của preset — thuộc `style_presets`.

### 7.5 Vòng đời GPU

Một tiến trình GPU tại một thời điểm, tính cả `acestep-api`:

1. Worker nhận `train_lora` (node chỉ giữ một job nên không còn job generate nào chạy).
2. Dừng `acestep-api` (`systemctl --user stop`), chờ tiến trình thoát, xác nhận bằng `nvidia-smi` không còn compute process.
3. Preflight: dataset version + checksum, SHA-256 checkpoint, dung lượng đĩa, VRAM trống.
4. Preprocess (bỏ qua nếu cache tensor khớp khóa) → train → lưu.
5. Kết thúc tiến trình trainer, xác nhận VRAM trống.
6. Khởi động lại `acestep-api`, chờ `/health`, nạp thử adapter, rồi mới `complete`.

Lỗi: OOM / NaN / trainer crash → `fail` không retry, kèm `error_code`, vẫn chạy bước 5–6. Cancel → gửi tín hiệu, chờ, bước 5–6. Worker bị restart khi đang train → khi khởi động lại đọc pidfile của run, kết thúc trainer mồ côi rồi mới register. Bước 6 thất bại → node báo engine không khỏe, không nhận job, hiện cảnh báo cho Admin; worker **không** tự reboot hay đổi driver. Mất mạng → trainer vẫn chạy, worker gửi lại khi có mạng; lease hết thì job thành `failed` phía VPS nhưng checkpoint còn trên node để resume.

Việc worker được phép dừng/khởi động `acestep-api` là thay đổi vận hành production → cần người dùng duyệt thiết kế này (Q1).

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

Điều kiện trước: E0 (reboot) và Q1–Q2 được trả lời. Thứ tự: E1 chạy tay trên node trước, rồi mới viết code theo kết quả.

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

Chưa đủ bằng chứng:
- Train được trên 12 GB hay không, tốc độ, kích thước file.
- Adapter Side-Step có nạp được qua `/v1/lora/load` không; LoRA train bằng timestep liên tục có hợp với turbo 8 bước không.
- Định dạng tiến độ của Side-Step khi chạy không tương tác; `step` nghĩa là gì; hành vi khi nhận tín hiệu dừng.
- Chất lượng: chưa có đánh giá nghe nào, kể cả cho model gốc. Không có cơ sở để hứa mức chất lượng sau fine-tune.
- LoRA trên segment ≤ 240 s không chứng minh được việc học cấu trúc bài dài.
- `guidance_scale` trên turbo, tác dụng của LM với nhạc không lời.
- Giấy phép riêng của LM/VAE.

Cần người dùng quyết định:
- **Q1.** Cho phép worker tự dừng/khởi động `acestep-api` trong lúc train (website không tạo nhạc được trong thời gian đó)?
- **Q2.** Reboot render node lúc nào (E0)?
- **Q3.** Đưa dataset lên node bằng cách nào: (a) chép vào thư mục import trên node/NAS rồi bấm Import trong Admin; (b) upload từ trình duyệt qua VPS (trái D9, tốn đĩa VPS); (c) upload thẳng tới node qua Tailscale (máy văn phòng phải cài Tailscale).
- **Q4.** Nguồn và quyền sử dụng của dataset đầu tiên (8–10 bài cho E1).
- **Q5.** Có tải thêm `acestep-v15-base`/`sft` để so sánh (E4) không.
