# ACE-Step XL SFT là model duy nhất; thêm LM 4B để thử

Ngày: 2026-11-25. Người dùng chốt: "dùng ACE-Step XL làm nền tảng duy nhất cho AI Music Studio, cả khi tạo nhạc và khi train LoRA. Gỡ bỏ hoàn toàn việc hỗ trợ ACE-Step Turbo", rồi duyệt các phương án khuyên dùng (câu 1–4 và A–D trong phiên). Quyết định ghi thành **D36** trong `docs/DECISIONS.md`.

## Mục đích

- Mọi bài nhạc và mọi LoRA dùng một model: **`acestep-v15-xl-sft`** (DiT 4B, `is_turbo: false`). LoRA train trên đúng model sẽ dùng nó.
- Turbo biến khỏi ứng dụng: code, test, tài liệu, cấu hình, checkpoint, database.
- Website "làm mới": xóa sạch lịch sử tạo nhạc cũ (đều làm bằng Turbo).
- Thêm **LM `acestep-5Hz-lm-4B`** để thử ở hai chỗ, mỗi chỗ có công tắc riêng: gắn nhãn dataset và "Thinking" khi tạo nhạc.

Không thuộc phạm vi: XL Turbo, XL Base, đổi kiến trúc (website nhận yêu cầu và theo dõi job; rtx6000 train và tạo nhạc; dữ liệu ở `/media/ai-rtx6000/AI_DATA/AiStudio`), đổi trainer, đổi thông số train mặc định (D34), lời bài hát (vẫn chỉ instrumental).

Thất bại dù "chạy được": LoRA train xong nhưng không nạp được vào XL khi tạo nhạc; bật LM làm hỏng tạo nhạc khi công tắc tắt; xóa nhầm tài khoản, thể loại, node hay model captioner.

## Hiện trạng (đã kiểm 2026-11-25)

- rtx6000: chỉ có `checkpoints/acestep-v15-turbo` (4,5 GB), `acestep-5Hz-lm-1.7B` (3,5 GB), `Qwen3-Embedding-0.6B`, `vae`. `config/acestep.env`: `ACESTEP_CONFIG_PATH=acestep-v15-turbo`, `ACESTEP_INIT_LLM=false`. `data/{datasets,import,loras,tensors,training}` trống.
- Database website: model Turbo (generator duy nhất), 4 job, 3 track, 7 artifact, 35 audit log, 10 session; 0 LoRA, 0 Style Preset, 0 catalog.
- Mã ACE-Step `ca1e85f` (bằng `main`). Trainer đi kèm nhận `--model-variant xl_sft` (`acestep/training_v2/cli/args.py:182`); worker đổi `acestep-v15-xl-sft` → `xl_sft` (`workers/music-worker/src/lush_worker/training.py:302`), đúng.
- Web và API không ghi cứng Turbo; model lấy từ dữ liệu (worker báo khi register).
- ACE-Step API có sẵn chế độ phân tích bằng LM: `/release_task` với `full_analysis_only` + `src_audio` → DiT mã hóa bài thành audio codes → LM `understand_audio_from_codes` → `{prompt (caption), bpm, keyscale, timesignature, duration, genre, lyrics, language}` (`acestep/api/job_analysis_runtime.py`). LM chỉ nạp khi `ACESTEP_INIT_LLM=true`, model chọn bằng `ACESTEP_LM_MODEL_PATH`.

## Hành vi sau khi làm

### Tạo nhạc
- Model duy nhất trong mọi form là XL SFT. Bước mặc định **50** (thay cho 8 của Turbo / 32), `guidance_scale` mặc định giữ 7.
- Nâng cao có công tắc **"Thinking (LM)"**, mặc định tắt. Bật: ACE-Step để LM lên dàn ý (metadata + audio codes) trước khi XL tạo nhạc; chậm hơn. Tắt: request giống hệt hôm nay (`thinking: false`, CoT tắt). Bài tạo ra lưu `params.thinking` để biết bài nào dùng LM. Style Preset lưu được giá trị này như các params khác.
- Không có LM trên máy (LM chưa nạp): công tắc ẩn hoặc vô hiệu kèm giải thích; API từ chối `thinking: true` với 422 rõ ràng thay vì để job lỗi trên GPU.

### Chuẩn bị dataset (gắn nhãn catalog)
- Ô chọn model mô tả hiện có thêm **"ACE-Step LM 4B"** (key `acestep-lm`) bên cạnh `acestep-captioner`. `acestep-captioner` vẫn là mặc định.
- Chọn LM: mỗi bài được gửi qua chế độ phân tích của ACE-Step; caption lấy từ `prompt`, kèm BPM/key/time signature LM đoán. Kết quả lưu như một caption revision bình thường (nguồn ghi rõ là LM) để so với caption của captioner trên cùng bài.
- Reference Audio ở trang Tạo nhạc vẫn ưu tiên `acestep-captioner` (D35), không đổi.

### Train LoRA
- Form tạo LoRA chỉ còn XL SFT; train `--model-variant xl_sft`, tensor cache theo `{dataset}-acestep-v15-xl-sft`. Thông số train giữ D34.
- LoRA lưu và gắn vào Style Preset như cũ; API vẫn chặn LoRA đi với model khác (`apps/api/src/routes/jobs.rs:383`).

## Phần cần sửa

**Worker** (`workers/music-worker/src/lush_worker/`)
- `engines/acestep.py`: bỏ `TURBO_STEPS` và nhánh `"turbo" in model_key`; mặc định 50 bước. Comment dòng 18, 287. `build_release_body` đọc `params.thinking` (bool, mặc định false): bật thì `thinking: true` (CoT caption/language giữ tắt, vì prompt là của người dùng).
- Báo khả năng LM: worker hỏi ACE-Step (inventory/health) xem LM đã nạp chưa; có thì model XL báo thêm capability `thinking`, và worker báo captioner built-in `acestep-lm` (engine `captioner`, chạy qua ACE-Step API chứ không qua script trong `CAPTIONER_DIR`).
- `caption_tracks` / `describe_input` với key `acestep-lm`: gọi `/release_task` `full_analysis_only` với file bài, đổi kết quả về cùng dạng trường như `parse_output` (caption, mood, instruments… lấy từ caption bằng `extract_fields`; tempo/key từ LM).
- Train: việc dừng `acestep-api` trước khi train giữ nguyên; kiểm rằng sau train service khởi động lại có LM.

**API** (`apps/api/src/`)
- `params.thinking` (bool) hợp lệ trong generate và Style Preset; `true` mà model không có capability `thinking` → 422 "LM chưa bật trên máy render".
- Test `apps/api/tests/api.rs`: đổi fixture key Turbo sang `acestep-v15-xl-sft`; thêm test cho `thinking`.

**Web** (`apps/web/src/`)
- `features/create/formState.ts` + form Nâng cao: công tắc "Thinking (LM)", hiện khi model có capability `thinking`. `pages/admin/StylesPage.tsx`: cùng công tắc.
- Catalog: không cần sửa ô chọn (đọc captioner từ API); chỉ kiểm hiển thị tên "ACE-Step LM 4B".

**lush-training**: `src/lush_training/cli.py:602` mặc định `acestep-v15-xl-sft`; `render_ab.py:25` bỏ `TURBO_STEPS`, dùng 50; sửa test kèm theo. Cảnh báo `is_turbo` trong `model.py:93` giữ (chung).

**Tài liệu**: `MUSIC_GENERATION.md`, `RENDER_NODE_DEPLOYMENT.md`, `STYLE_TRAINING.md`, `WORKER_PROTOCOL.md` (ví dụ base_model, captioner `acestep-lm`, `params.thinking`), `API_CONTRACTS.md` (`params.thinking`), `PROJECT_BRIEF.md`, `ROADMAP.md`, `lush-training/docs/MODEL_COMPATIBILITY.md` + `STATUS.md`, D36 trong `DECISIONS.md`, `CHANGELOG.md`. Số đo Turbo cũ trong `TESTING.md`, `CHANGELOG.md`, các spec/plan cũ giữ nguyên làm lịch sử.

**rtx6000** (cần duyệt từng bước, theo AGENTS.md)
- Tải `ACE-Step/acestep-v15-xl-sft` (~20 GB) và `ACE-Step/acestep-5Hz-lm-4B` (~8,4 GB) vào `ACE-Step-1.5/checkpoints/`, kiểm đủ file. Trước khi ghi: kiểm `/media/ai-rtx6000/AI_DATA` đúng mount `/dev/nvme1n1p1`.
- `config/acestep.env`: `ACESTEP_CONFIG_PATH=acestep-v15-xl-sft`, `ACESTEP_INIT_LLM=true`, `ACESTEP_LM_MODEL_PATH=acestep-5Hz-lm-4B`. Khởi động lại `acestep-api` và worker.
- Xóa `checkpoints/acestep-v15-turbo` và `checkpoints/acestep-5Hz-lm-1.7B` sau khi XL và LM 4B khởi động được.

**Database website** (lushvietnix2, cần duyệt; chạy một lần bằng script SQL, không đưa vào migration)
- Xóa: 4 job, 3 track, 7 artifact và file thật của chúng, model `acestep-v15-turbo` cùng `model_installations`, 35 audit log.
- Giữ: user, session, 23 genre, node rtx6000, các model captioner/scorer.
- Trước khi xóa: sao lưu `lush.db` ra file riêng trên VPS; xóa bản sao lưu sau 7 ngày nếu không cần (**tôi tự quyết**, để còn đường lùi nếu xóa nhầm).

## Kiểm thử từ đầu đến cuối (thật, trên rtx6000 và website)

1. `acestep-api` khởi động với XL SFT + LM 4B; worker register báo XL (có capability `thinking`) và captioner `acestep-lm`; website chỉ còn XL.
2. Tạo bài 30 s không LoRA, Thinking tắt: ghi thời gian, VRAM, `prompt_final`. Lặp lại với Thinking bật.
3. Catalog 5–10 bài Lo-fi: gắn nhãn bằng `acestep-captioner` và bằng `acestep-lm`, so caption hai bên.
4. Tạo LoRA từ dataset đó, epoch ngắn: theo dõi preprocess → train → lưu checkpoint trong `data/loras`.
5. Eval, duyệt, gắn vào Style Preset.
6. Cùng prompt, cùng seed: có Style và không Style; `/v1/lora/status` cho thấy LoRA đã nạp; hai bài nghe khác nhau.
7. Tự viết prompt từ Reference Audio vẫn chạy.

Báo rõ bước nào là thật, bước nào mock (AGENTS.md).

## Chưa xác minh (rủi ro)

- Nạp LoRA vào XL qua `/v1/lora/load` khi tạo nhạc.
- XL SFT có tự dùng shift 1.0 khi worker không gửi `shift` không; nếu API nhận `shift`, gửi tường minh 1.0.
- Tốc độ, VRAM của XL + LM 4B cùng lúc; LM backend có chiếm trước nhiều VRAM (ảnh hưởng captioner chạy cùng máy) không.
- `full_analysis_only` chạy đúng với XL (audio codes do tokenizer của DiT XL tạo).
- Thinking với bài instrumental (lyrics `[Instrumental]`) cho kết quả hợp lý.

## Quyết định của Claude (người dùng có thể bác)

- **Thứ tự triển khai:** (1) kiểm mount, tải XL SFT + LM 4B; (2) đổi `config/acestep.env`, khởi động lại `acestep-api` và worker, để worker register XL; (3) **chạy thử sớm nạp LoRA trên XL** (train một LoRA rất nhỏ, nạp, tạo một bài) trước khi làm phần giao diện LM; nạp không được thì dừng và hỏi người dùng; (4) xóa database cũ và checkpoint Turbo/LM 1.7B sau khi worker đã không còn báo Turbo, để Turbo không bị register lại; (5) deploy code; (6) kiểm thử đầy đủ ở trên.
- **LM khi gắn nhãn đi qua job `caption_tracks` / `describe_input` có sẵn** (key `acestep-lm`), không thêm loại job: cùng hàng chờ, lease, hủy, tiến độ như captioner khác. Một job GPU mỗi node nên không chạy cùng lúc train (train vẫn dừng `acestep-api`).
- **Nguồn caption LM:** `caption_revisions.model_key = 'acestep-lm'`, `source = 'model'`, `source_ref` ghi tên checkpoint LM. Không cần migration.
- **Capability `thinking`:** worker kiểm LM mỗi lần register (mỗi lần khởi động). API kiểm khi nhận yêu cầu (422). Nếu LM mất sau đó, job lỗi rõ "LM chưa nạp trên máy render", không tự tắt Thinking.
- **Không chặn số bước thấp:** mặc định 50; người dùng hoặc Style Preset vẫn đặt được số khác (hiện không có preset nào lưu 8).
- **Ngưỡng bỏ LM:** nếu XL + LM 4B không khởi động được trên card 96 GB, hoặc captioner không còn đủ VRAM để chạy, thì đặt `ACESTEP_INIT_LLM=false`. Hai công tắc LM tự ẩn theo capability, phần còn lại vẫn chạy. Báo người dùng trước khi quyết LM 1.7B thay thế.
- **LoRA Turbo cũ:** không có (0 trong database, `data/loras` trống), không có gì để chuyển.

## Phần để người làm tự quyết

Cách worker phát hiện LM đã nạp; tên/nhãn hiển thị cụ thể; bản ghi caption revision ghi nguồn LM thế nào; thứ tự commit. Mặc định 50 bước, guidance 7, sao lưu DB 7 ngày là quyết định của Claude, người dùng có thể bác.
