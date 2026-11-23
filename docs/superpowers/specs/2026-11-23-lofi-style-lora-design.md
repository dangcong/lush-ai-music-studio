# Thiết kế: Dataset Factory + Train Style LoRA cho Lo-fi (pilot)

Ngày: 2026-11-23. Trạng thái: **đã duyệt 2026-11-23** (người dùng: "Duyệt"). Tech stack (mục 5.9) là lựa chọn hiện tại, người dùng có thể đổi sau.

## 1. Mục đích và lý do

Người dùng có khoảng 250 bài Lo-fi instrumental do chính họ sản xuất (toàn quyền sử dụng), định dạng MP3, đang nằm trên máy dev Windows. Họ muốn Studio tạo nhạc mang đúng chất Lo-fi của mình bằng cách train Style LoRA trên ACE-Step 1.5.

Cách người dùng muốn làm việc: **mỗi lần train, họ chọn một nhóm bài. Nhóm đó trở thành một phiên bản dataset và cho ra đúng một LoRA**, rồi họ dùng LoRA đó để tạo nhạc. Lần đầu là một nhóm nhỏ để thử (khoảng 10–20 bài). Nếu kết quả tốt mới mở rộng.

Mô hình tham khảo là tài liệu người dùng tìm được: Dataset Factory (phân tích audio + caption AI → metadata → chia train/validation) → train LoRA → nghe đánh giá. Thông số RTX 3060 trong tài liệu đó **không** áp dụng; máy train là rtx6000.

## 2. Mục tiêu, không làm, phản mục tiêu

**Mục tiêu**
- G1. Biến một nhóm MP3 được chọn thành phiên bản dataset bất biến, có ID, checksum, nguồn gốc, metadata và caption tiếng Anh đã được người nghe duyệt.
- G2. Đo được chất lượng caption tự động trên 10–20 mẫu: tỉ lệ đúng, chi tiết bịa và thời gian sửa.
- G3. Train một Style LoRA bằng luồng Train Style đã có của Studio trên rtx6000. Đo VRAM và thời gian, trước với đoạn ngắn rồi mới với cả bài.
- G4. Nghe A/B giữa base và LoRA (cùng prompt, seed, thiết lập), chấm theo phiếu và chọn checkpoint bằng tai.

**Không làm trong task này**
- Fine-tune base model (đó là `lush-training/` backend + T-012).
- Publish LoRA cho người dùng, tạo Style dùng chung, hay đổi preset production.
- Trang Dataset trong Admin của website (sẽ làm sau, khi luồng CLI đã chứng minh được).
- Tải nhạc từ Internet, tự tách vocal.

**Phản mục tiêu** (chạy được về kỹ thuật nhưng vẫn coi là thất bại)
- Caption chứa chi tiết bịa (nhạc cụ không có, BPM/key đoán) mà vẫn lọt vào dataset vì không ai nghe lại.
- Cùng một bài có đoạn nằm ở cả train lẫn validation.
- Sửa caption nhưng trainer vẫn dùng tensor cache cũ.
- Train làm gián đoạn việc tạo nhạc của người khác khi chưa báo trước, hoặc làm hỏng adapter/LoRA đang dùng trên production.
- Chọn checkpoint cuối chỉ vì nó là checkpoint cuối, hoặc chọn theo loss.

## 3. Hiện trạng (đã kiểm ngày 2026-11-23)

- **Train Style (M1, T-008) đã chạy trên production.** Server website có migration `0004_training`. Node `rtx6000` báo job types `["generate","train_lora"]`. Worker `training.py` làm trọn chuỗi: dừng `acestep-api` → preprocess bằng Side-Step (`train.py fixed --preprocess`, `--max-duration 240`) → train → kiểm adapter → bật lại inference → nạp thử adapter → LoRA vào Registry ở trạng thái `evaluation`. Chuỗi này mới chạy thật trên máy 3060 cũ với 8 bài tự sinh, **chưa từng chạy với nhạc thật và chưa chạy trên rtx6000**.
- Định dạng dataset worker đọc: `data/datasets/<tên>/<version>/dataset.json` + `audio/` (theo `acestep/training/dataset_builder_modules/models.py`: `metadata` + `samples[]` với `audio_path`, `caption`, `lyrics="[Instrumental]"`, `bpm`, `keyscale`, `timesignature`, `duration`, `is_instrumental`, `labeled`…). Tensor cache: `data/tensors/<dataset_key>-<model_key>`.
- Worker đang cố định một số tham số: batch 1, AdamW, gradient checkpointing, `--offload-encoder`, dropout 0,1. Rank, alpha, lr, epochs, warmup, grad accumulation, save_every và seed lấy từ run.
- **`lush-training/` đã có công cụ dataset tái dùng được** (mới test trên audio tổng hợp): quét WAV/FLAC/MP3, trùng lặp và gần trùng, `work_group`, chia train/validation/test theo nhóm trước khi cắt đoạn, cắt segment, manifest, caption nháp/kiểm/khóa, JSON cho engine. Có `analyze_audio.py` (librosa và pyloudnorm là tùy chọn, chưa cài).
- rtx6000: AI_DATA đã mount đúng (`/dev/nvme1n1p1`, ext4, còn trống 3,4 TB). RTX PRO 6000 96 GB (inference đang chiếm ~8 GB), RAM 123 GB, 32 nhân. `data/{datasets,import,tensors,training,loras}` đều rỗng.
- Chỉ có **một** node GPU, và nó đang phục vụ production.

## 4. Quyết định đã chốt với người dùng

| # | Quyết định |
|---|---|
| U1 | Pilot với Lo-fi, nhạc của chính người dùng, toàn quyền sử dụng, MP3, nằm trên máy dev |
| U2 | Thử trước với một nhóm nhỏ do người dùng chọn và tải lên; 250 bài làm sau |
| U3 | Mỗi lần train = một nhóm bài = một phiên bản dataset = một LoRA |
| U4 | Từ prompt task: không tích hợp Music Flamingo mặc định; chỉ dùng công cụ caption được phép dùng thương mại; không đoán BPM/key khi thiếu căn cứ; chỉ thêm Essentia nếu cần và đã kiểm giấy phép |
| U7 | Train cả bài (không cắt đoạn 60 s), tận dụng GPU 96 GB |
| U6 | Bài có vocal chop / tiếng nói / tiếng ngân: loại khỏi dataset |
| U5 | Từ prompt task: một job GPU tại một thời điểm; không publish hay đổi production trong pilot; mọi bước cài, tải model, chạy GPU, deploy đều cần duyệt |

## 5. Thiết kế

### 5.1 Hai luồng, tách bạch

- **Dataset Factory** = công cụ CLI trong `lush-training/` (tái dùng module dataset/caption đã có), cài thành một bản checkout riêng trên rtx6000 với venv riêng. Đầu ra cuối cùng là một thư mục dataset đúng định dạng worker đọc. *(Lựa chọn của tôi: không viết lại từ đầu, không đặt vào `workers/` để không ảnh hưởng worker đang chạy production.)*
- **Train Style** = luồng đã có của Studio (`train_lora`, trang Training trên website). Dataset Factory **không** gọi trainer. Trainer trong `lush-training/` (fine-tune decoder) không dùng cho task này.

### 5.2 Cây thư mục trên rtx6000

```
/media/ai-rtx6000/AI_DATA/AiStudio/
  ACE-Step-1.5/                 có sẵn, không sửa
  lush-worker/                  có sẵn
  factory/                      MỚI: bản checkout lush-training + .venv riêng (librosa, soundfile, transformers…)
  models/captioner/             MỚI, chỉ khi người dùng duyệt tải (mục 5.4)
  data/
    import/lofi/<batch-YYYYMMDD>/*.mp3      bản gốc tải từ máy dev, chmod chỉ đọc, không bao giờ sửa
    factory/lofi/
      sources.jsonl              mỗi bài gốc: source_id, đường dẫn, sha256, duration, batch, work_group, quyền
      samples.jsonl              mỗi mẫu: sample_id, source_id, start_s, end_s, split
      analysis/<sample_id>.json  BPM/key/energy/loudness + căn cứ + công cụ/phiên bản
      captions/<sample_id>.json  lịch sử: nháp máy → bản sửa của người → trạng thái duyệt
      review/log.jsonl           ai duyệt, lúc nào, quyết định, số giây sửa, các chi tiết bịa đã gạch
      pilot-report.md            thống kê caption (G2)
    datasets/lofi-<tên-nhóm>/v<N>/   XUẤT cho Train Style; bất biến sau khi xuất
      dataset.json
      audio/<sample_id>.wav           48 kHz stereo 16-bit, giải mã từ MP3 bằng ffmpeg
      manifest.json                   checksum từng file, nguồn, phiên bản caption, split của các mẫu không train
    tensors/ training/ loras/    worker quản lý, không đụng
    eval/lofi-<tên-nhóm>/<run>/  render A/B, phiếu chấm, kết quả
```

ID: `source_id` = 12 ký tự đầu SHA-256 của file MP3 gốc. `sample_id` = `<source_id>-<start_s>-<end_s>`, hoặc `<source_id>-full` khi không cắt.

### 5.3 Phân tích audio (tách khỏi caption)

- `ffprobe`/`soundfile`: định dạng, sample rate, duration, bitrate MP3. Phát hiện clipping, im lặng và loudness (pyloudnorm, MIT).
- BPM: librosa (ISC), có thể thêm `beat_this` (MIT) nếu librosa sai nhiều. Key: profile chroma (Krumhansl) trên librosa. **Mỗi giá trị kèm độ tin cậy và công cụ. Dưới ngưỡng thì ghi `unknown`.** Nghi nhân đôi/chia đôi tempo (ví dụ 70 hay 140) thì đánh dấu để người nghe chọn.
- Không dùng Essentia (AGPL, model NC) hay madmom (model NC).
- Chỉ đưa BPM/key vào `dataset.json` khi người duyệt đã xác nhận, hoặc khi độ tin cậy vượt ngưỡng do pilot hiệu chỉnh. Ngược lại để trống, và upstream tự ghi `N/A`.
- Trùng lặp: SHA-256 + gần trùng (chức năng có sẵn, ngưỡng 0,97 chưa hiệu chỉnh). Kết quả gần trùng phải được người xem trước khi khóa.

### 5.4 Caption AI

- Ứng viên chính: **`ACE-Step/acestep-captioner`**, mô hình chính ACE-Step dùng để gắn nhãn dữ liệu train 1.5. Thẻ model ghi giấy phép MIT. **Cảnh báo:** repo weights không có file LICENSE riêng, và chưa rõ nó fine-tune từ biến thể Qwen2.5-Omni nào. Một bản mirror ghi khoảng 11B tham số (chưa đối chiếu với repo chính thức), nên bản bf16 có thể nặng cỡ 20+ GB. Trước khi tải, tôi sẽ báo dung lượng thật, giấy phép và phiên bản transformers cần, rồi chờ người dùng duyệt.
- Chạy trong venv của `factory/`, không đụng venv ACE-Step. Với 96 GB VRAM, captioner (ước ~25 GB) **chạy được cùng lúc với `acestep-api` (~8 GB)**, nên bước caption không cần dừng việc tạo nhạc. Mức VRAM này phải đo lại khi chạy thật.
- Đầu ra: genre/subgenre, mood, nhạc cụ, vocal/instrumental và một caption tiếng Anh ≤ 256 token. Caption không chứa BPM/key (hai trường này đi riêng theo mục 5.3).
- Phương án dự phòng nếu không duyệt tải, hoặc caption tệ: nháp từ nhãn thư mục cộng metadata, rồi người viết tay. Mỗi caption ghi rõ `source`: `acestep-captioner@<rev>` / `folder-label` / `human`.
- Music Flamingo: không dùng (giấy phép không cho thương mại).

### 5.5 Nghe, sửa và duyệt

- Một trang web nhỏ do `factory` phục vụ, chỉ nghe trên `127.0.0.1` của rtx6000. Người dùng mở từ máy dev qua SSH tunnel (`ssh -L`). Trang không nằm trên website công khai.
- Mỗi mẫu hiển thị: trình phát audio, số đo phân tích, caption nháp và ô sửa. Có các nút **Đúng / Sửa / Sai / Loại mẫu**, ô tick các chi tiết bịa (nhạc cụ không có, mood sai, BPM/key sai), và cờ "nghe thấy giọng hát/lời nói". Trang tự đo thời gian từ lúc mở tới lúc duyệt.
- Chỉ mẫu ở trạng thái `approved` mới được xuất. Mẫu có cờ vocal phải được người nghe xác nhận; máy không tự kết luận.
- **Bài có vocal chop, câu nói lấy mẫu, tiếng ngân hay bất kỳ giọng người nào (người nghe đã xác nhận) bị loại khỏi dataset** (quyết định U6). Cả bài bị loại, không cắt bỏ đoạn có giọng. Lý do loại ghi vào `review/log.jsonl`. `pilot-report.md` thống kê số bài bị loại vì vocal, để biết 250 bài thực còn bao nhiêu bài dùng được.

### 5.6 Chia tập và phiên bản

- Chia train / validation theo `source_id`/`work_group` **trước khi cắt đoạn** (dùng chức năng có sẵn). Pilot 10–20 bài: khoảng 80/20, tối thiểu 2 bài validation. Bài validation **không** vào `dataset.json`. Chúng dùng để viết prompt đánh giá và kiểm "học thuộc" (mục 5.8).
- Mỗi lần xuất tạo `v<N>` mới. Thư mục đã xuất không bao giờ được ghi đè: công cụ từ chối nếu thư mục đã tồn tại, và đặt chế độ chỉ đọc. Muốn sửa caption thì xuất `v<N+1>`.
- Cache tensor: khóa của worker là `tensors/<dataset_key có version>-<model>` (`training.py:261`), và worker bỏ qua preprocess khi số `.pt` ≥ số mẫu (`:276`). Vì vậy đổi version là preprocess chạy lại. Cache cũ chỉ bị dùng nhầm khi ai đó sửa tay bên trong một version đã xuất, và công cụ xuất đã chặn việc này bằng thư mục bất biến, chỉ đọc. **Pilot không sửa worker** *(lựa chọn của tôi: tránh deploy worker lên node production duy nhất giữa chừng)*. Thêm hash `dataset.json` vào khóa cache để dành làm việc sau. Nghiệm thu bằng một lần chạy thật: hai version chỉ khác caption → log worker phải cho thấy preprocess chạy lại.
- **Mỗi lần xuất cần worker đăng ký lại.** Node chỉ báo danh sách dataset khi register (`runner.py:154`), và API từ chối run với dataset node chưa báo (`routes/training.rs:105-117`). Vì vậy sau mỗi lần xuất phải restart `lush-worker` (lúc node không có job) thì dataset mới hiện trên trang Training. Việc restart nằm trong quy trình khung giờ train (mục 5.7) và được xin duyệt cùng lúc.
- **Train cả bài (quyết định U7):** mỗi mẫu là một bài nguyên vẹn, không cắt đoạn 60 s. Giới hạn kỹ thuật cần biết: worker gọi preprocess với `--max-duration 240` (`training.py:205`), và ACE-Step chỉ lấy **240 giây đầu** của mỗi file. Bài ≤ 240 s (đa số Lo-fi 2–3 phút) được dùng trọn. Bài dài hơn sẽ được công cụ báo trước khi xuất, để người dùng chọn: chấp nhận cắt còn 240 s đầu, hoặc tách làm hai mẫu có ghi mốc thời gian. Muốn nâng giới hạn 240 s thì phải sửa worker, và việc đó nằm ngoài pilot.
- **Đặt tên version:** `lofi-<nhóm>/v1`, rồi `v2`, `v3`… cho mỗi lần sửa caption hoặc đổi danh sách bài. *(Lựa chọn của tôi.)*
- **Trigger tag:** pilot **không** dùng `custom_tag`. Caption dùng từ vựng khớp cách `compose_prompt` (`apps/api/src/routes/jobs.rs:84`) ghép prompt lúc tạo nhạc, và prompt A/B được ghép theo đúng cách đó. LoRA được bật qua Style Preset nên không cần từ kích hoạt. Nếu A/B cho thấy LoRA yếu thì thử trigger tag ở version sau. *(Lựa chọn của tôi.)*

### 5.7 Train (Giai đoạn 2), một job tại một thời điểm

Tạo run qua trang Training của website (đúng luồng production). Node chỉ có một, và khi train, `acestep-api` bị dừng nên **website không tạo nhạc được trong lúc đó** (job generate nằm chờ). Vì vậy mỗi run chỉ chạy trong khung giờ người dùng chọn.

| Bước | Dataset | Cấu hình khởi đầu | Mục đích |
|---|---|---|---|
| T1 smoke | `v1`: nhóm pilot, cả bài | rank 32, alpha 64, lr 1e-4, 1 epoch, save_every 1 | Chạy ngắn (vài phút) để xác nhận cả bài chạy được trên rtx6000; đo VRAM, giây/step, thời gian preprocess riêng với train. Tensor cache dùng lại được cho T2. LoRA ra từ T1 chuyển `archived` |
| T2 thật | `v1` (cùng tensor cache) | như T1, 5 epoch, save_every 1 | LoRA đầu tiên để nghe |

Quy trình mỗi khung giờ train: báo trước người dùng → chờ node rảnh → restart `lush-worker` để đăng ký dataset mới → tạo run trên trang Training → theo dõi → sau khi run xong, tạo một bài thường để kiểm tra việc tạo nhạc đã chạy lại.

Số cụ thể của T2 được chỉnh theo số đo của T1, không lấy từ ảnh RTX 3060. Mỗi run ghim commit ACE-Step `ca1e85f`, checkpoint `acestep-v15-turbo` (SHA-256), dataset key + version và toàn bộ config. Phần ghim này worker đã ghi vào `run.json`.

### 5.8 Đánh giá

- Bộ prompt đánh giá có version, gồm Lo-fi trong style (viết từ caption của bài **validation**, không từ bài train) và vài prompt ngoài style để kiểm regression. Mỗi prompt 3 seed cố định, 60–120 s.
- Render base (tắt LoRA) và từng checkpoint đã lưu (không chỉ checkpoint cuối), cùng prompt/seed/steps. Phần render chạy trong khung giờ đã đặt, khi worker tạm dừng nhận job. Sau đó restart `acestep-api` để gỡ LoRA thử, như cách E1 đã làm. LoRA khác vẫn ở `evaluation`; không tạo Style.
- Phiếu nghe A/B mù (thứ tự ngẫu nhiên): đúng phong cách, chất lượng âm thanh, lỗi nghe được, có lẫn giọng hát không, kèm ghi chú. Kiểm kỹ thuật tự động: im lặng, clipping, loudness, duration. Kiểm học thuộc: so fingerprint output với bài train.
- Người dùng chọn checkpoint. Báo cáo ghi rõ số mẫu và số người nghe; với 1 người và vài chục cặp thì chỉ phát hiện được khác biệt lớn.

### 5.9 Tech stack

| Thành phần | Công nghệ | Nhiệm vụ |
|---|---|---|
| Base Model | ACE-Step 1.5 `acestep-v15-turbo`, commit `ca1e85f` (đang chạy trên rtx6000) | Sinh nhạc; LoRA gắn lên DiT decoder |
| Training | Side-Step tích hợp trong ACE-Step (`train.py fixed`, bản MIT, không dùng bản standalone CC BY-NC-SA) + PyTorch 2.10 cu128 + PEFT; chạy qua job `train_lora` của Studio worker | Train Style LoRA |
| Music Understanding | `acestep-captioner` (mô hình gắn nhãn ACE-Step dùng cho dữ liệu 1.5, gốc Qwen2.5-Omni, thẻ MIT); chạy bằng transformers trong venv riêng. Không dùng Music Flamingo | Nghe audio, nhận ra nhạc cụ, genre, mood, có giọng người hay không |
| Audio Analysis | librosa (BPM, key bằng chroma profile) + pyloudnorm (LUFS); `beat_this` nếu librosa sai nhiều. Không dùng Essentia/madmom | BPM, key, energy, loudness, clipping, im lặng, kèm độ tin cậy |
| Caption Generator | `acestep-captioner` (cùng mô hình trên) + người duyệt trên trang review. Dự phòng: nhãn thư mục + viết tay | Caption tiếng Anh ≤ 256 token |
| Metadata Builder | Python + Pydantic (schema mới cho factory) | Gộp phân tích + caption + quyết định duyệt thành bản ghi chuẩn, kiểm hợp lệ |
| Audio Processing | FFmpeg/ffprobe + SoundFile | Đọc MP3, giải mã sang WAV 48 kHz stereo cho trainer, kiểm định dạng |
| Dataset Manager | Module dataset có sẵn của `lush-training` (trùng lặp, nhóm, split, version) + manifest JSONL + bộ xuất `dataset.json` | Quản lý version bất biến, chia train/validation theo bài, xuất cho Train Style |
| GPU | RTX PRO 6000 Blackwell 96 GB trên `rtx6000` | Caption (chạy song song được với inference), preprocess, train (dừng inference), render A/B |

## 6. File dự kiến sửa (ước lượng, builder chốt lại)

| Nơi | File | Việc |
|---|---|---|
| `lush-training/` | `src/lush_training/analysis.py` (mới hoặc mở rộng `audio.py`) | BPM/key/energy có độ tin cậy |
| | `src/lush_training/captioner.py` (mới) | Gọi acestep-captioner, ghi nháp kèm source |
| | `scripts/review_server.py` + trang tĩnh (mới) | Trang nghe/sửa/duyệt, log thời gian |
| | `scripts/export_studio_dataset.py` (mới) | Xuất `datasets/<k>/v<N>/` theo định dạng worker, bất biến, manifest |
| | `scripts/eval_ab.py` (mới) | Render A/B, phiếu mù, báo cáo |
| | `configs/factory.lofi.example.yaml`, `tests/…`, `docs/STATUS.md`, `AGENTS.md` | Cấu hình, test, ghi rõ đây là luồng Style LoRA, khác luồng fine-tune |
| `docs/` | `STYLE_TRAINING.md`, `tasks/CURRENT.md` (T-014), `DECISIONS.md` (D30), `RENDER_NODE_DEPLOYMENT.md` | Ghi kết quả và cách chạy |

Không sửa `ACE-Step-1.5/`, worker, API, web hay nginx. Vì vậy pilot không cần deploy gì lên production; chỉ cài `factory/` riêng trên rtx6000.

## 7. Nghiệm thu

1. Nhóm pilot nằm trong `data/import/` với checksum trùng file trên máy dev; bản gốc không bị sửa.
2. Mỗi mẫu có analysis + caption nháp. `pilot-report.md` có tỉ lệ Đúng/Sửa/Sai, số chi tiết bịa theo loại và thời gian sửa trung bình.
3. Không bài nào có đoạn ở cả hai tập (có kiểm tự động).
4. Xuất thành công, xuất đè một version đã có bị từ chối. Hai version chỉ khác caption (ví dụ `v1` → `v2`) → preprocess chạy lại (bằng chứng trong log worker).
5. T1 và T2 hoàn tất trên rtx6000, có số đo VRAM/thời gian thật. Inference bật lại; một bài tạo thường sau đó vẫn ra đúng.
6. Có bộ render A/B, phiếu chấm, và checkpoint do người dùng chọn. LoRA vẫn ở `evaluation`.
7. Test tự động của `lush-training` và worker vẫn đạt. Báo cáo bàn giao ghi diff/commit, lệnh đã chạy, kết quả, phần chưa xác minh.

## 8. Hoàn tác

- Code: revert commit. Pilot không đổi code production.
- Trên rtx6000, mọi thứ mới đều nằm trong `factory/`, `models/captioner/`, `data/factory/`, `data/import/lofi/`, `data/datasets/lofi-*`, `data/eval/`. Xóa (khi người dùng đồng ý) là về trạng thái cũ. Không đụng `ACE-Step-1.5/` hay venv của nó.
- LoRA trong Registry: chuyển `archived`. Run đang chạy: hủy từ trang Training (đã kiểm ở M1: inference bật lại, không đăng ký LoRA).

## 9. Để builder tự quyết

Cấu trúc module trong `lush-training`, công nghệ trang duyệt (ưu tiên stdlib/nhẹ), ngưỡng tin cậy BPM/key ban đầu, định dạng phiếu chấm, cách tạm dừng worker khi render A/B, cách đồng bộ MP3 lên (scp/rsync).

## 10. Câu hỏi còn mở cho người dùng

Không còn câu hỏi chặn việc lập kế hoạch. Các việc cần người dùng làm hoặc duyệt trong lúc triển khai:
1. Chọn 10–20 bài pilot trên máy dev (đường dẫn thư mục).
2. Duyệt tải `acestep-captioner` sau khi tôi báo dung lượng, giấy phép và yêu cầu chạy. Nếu không duyệt, dùng phương án caption tay.
3. Nghe và duyệt caption trên trang review.
4. Chọn khung giờ cho mỗi lần train và render A/B (lúc đó website không tạo nhạc được).
5. Nghe phiếu A/B và chọn checkpoint.
