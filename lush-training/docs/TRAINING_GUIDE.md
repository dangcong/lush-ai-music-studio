# TRAINING_GUIDE — Từng bước, từng lệnh

Mọi lệnh chạy từ thư mục `lush-training/` bằng Python của `.venv`. Thêm `--help` để xem đủ tùy chọn. Lệnh có ghi **[GPU]** cần bạn phê duyệt từng lần và cần cờ `--confirm-gpu`.

Thuật ngữ dùng trong báo cáo, không dùng lẫn:

- **Track**: một file nhạc gốc. **Segment**: một đoạn cắt từ track (mặc định 60 giây).
- **Optimizer step**: một lần cập nhật trọng số = `batch_size × gradient_accumulation` segment.
- **Epoch** ở đây = một lượt mà mỗi segment train được dùng đúng một lần, theo thứ tự xáo trộn có seed. Không có random crop: segment được cắt cố định lúc khóa dataset. Vậy N epoch nghĩa là mỗi segment (không phải mỗi bài) được dùng N lần; một bài dài có nhiều segment hơn bài ngắn.

## 1. Môi trường

```bash
python scripts/doctor.py
```

Chỉ đọc. Báo Python, RAM, GPU (theo `nvidia-smi`), torch, giải mã audio, vùng dữ liệu, checkout ACE-Step. Để biết torch trong virtualenv ACE-Step có thấy GPU không, chạy doctor bằng Python đó: `<acestep python> scripts/doctor.py`.

## 2. Model

```bash
python scripts/inspect_model.py acestep-v15-base --hash --method full_decoder
```

Đọc header weights, không nạp model, không dùng GPU. In từng thành phần với số tham số và dtype, backend tương thích, và với `--method`: bao nhiêu tham số được train, thành phần nào đóng băng. `--hash` tính SHA-256 (vài phút với file nhiều GB).

Checkpoint phải nằm trong `<data_root>/pretrained/` (hoặc `acestep.checkpoints_dir`) cùng `vae/` và `Qwen3-Embedding-0.6B/`. Tải chúng là việc cần phê duyệt; workspace không tự tải.

## 3. Dataset

Chép `configs/dataset.example.yaml` thành `configs/dataset.yaml`, điền `name`, `source_dirs`, `rights`.

```bash
python scripts/prepare_dataset.py scan --config configs/dataset.yaml
```

Quét đệ quy WAV / FLAC / MP3, chỉ đọc. Mỗi file: SHA-256, giải mã thử, duration, sample rate, số kênh, im lặng, clipping. Báo file bị loại (không giải mã được, quá ngắn, trùng từng byte) và cảnh báo kỹ thuật. Kết quả cache theo (đường dẫn, kích thước, thời điểm sửa), nên quét lại chỉ xử lý file mới.

Nhóm tác phẩm (để chia tập không rò rỉ) được ghép từ: cột `work_group` trong `metadata_csv`; tên file sau khi bỏ phần trong ngoặc và các từ như remix, edit, v2, master; và cặp audio gần trùng theo fingerprint. Fingerprint bắt được cùng một bản ghi ở định dạng hay âm lượng khác; **không** bắt được remix, bản thu khác, bản đổi tông. Những trường hợp đó bạn khai trong `metadata_csv`.

**Vocal.** Workspace không kèm detector. Nếu bạn có kết quả từ một detector chạy ngoài, đưa vào `vocal_screening_csv`; file có điểm ≥ `vocal_flag_threshold` bị gắn cờ và `build` dừng cho tới khi bạn nghe và ghi quyết định:

```bash
python scripts/prepare_dataset.py review --config configs/dataset.yaml --file "sub/track 07.wav" --decision instrumental --by "Tên bạn"
```

`--decision`: `instrumental`, `vocal` (bị loại), `unsure` (vẫn chặn). Không có detector thì mọi track mang trạng thái `not_screened` và manifest ghi rõ điều đó: dataset chưa được sàng lọc vocal, chỉ dựa trên xác nhận của bạn về nguồn nhạc.

```bash
python scripts/prepare_dataset.py build --config configs/dataset.yaml --version v1
```

Yêu cầu `rights.confirmed: true` và `confirmed_by`. Chia train / validation / test theo nhóm **trước** khi cắt segment, cắt segment thành WAV 24-bit trong vùng dữ liệu (file gốc không đổi), ghi `manifest.json` bất biến kèm SHA-256 từng segment. Version đã có thì từ chối; đổi gì cũng là version mới. Tập test không được caption, không được train, chỉ dành cho đánh giá cuối.

## 4. Caption

```bash
python scripts/analyze_audio.py --dataset lofi-studio --version v1
```

```bash
python scripts/validate_captions.py init --dataset lofi-studio --version v1
```

`init` tạo một file nháp cho mỗi segment train/validation (`captions/work/<segment_id>.json`), mọi trường metadata là `null`. Số đo có sẵn: loudness RMS, peak, dynamic range, phổ thô; tempo ước lượng và LUFS chỉ khi cài `.[analysis]`. **Không có công cụ nào ở đây nhận ra thể loại, nhạc cụ, mood, hòa âm, cấu trúc.** Những trường đó do bạn nghe rồi mô tả; Claude Code giúp viết thành caption và kiểm tra nhất quán, nhưng không nghe được audio.

Caption tập trung vào: genre/subgenre, instruments, mood, tempo và rhythm, harmony, texture, dynamics, structure khi có bằng chứng. Không mô tả giọng hát. Ghi một caption:

```bash
python scripts/validate_captions.py set --dataset lofi-studio --version v1 --segment <segment_id> --by claude --metadata "{\"genre\": \"lo-fi\", \"instruments\": [\"electric piano\", \"drums\"]}" --caption "lo-fi instrumental, mellow electric piano, soft drums"
```

Duyệt là quyết định của bạn (`--human`), ghi vào `human_edits`:

```bash
python scripts/validate_captions.py set --dataset lofi-studio --version v1 --segment <segment_id> --by "Tên bạn" --human --status approved
```

```bash
python scripts/validate_captions.py validate --dataset lofi-studio --version v1
```

Kiểm: schema, segment có trong manifest, SHA-256 của segment khớp, đúng dataset version, không nhắc tới vocal, độ dài, BPM trong caption khớp metadata, caption `approved` có bản ghi người duyệt. Cảnh báo khi một trường có giá trị mà không có evidence.

```bash
python scripts/validate_captions.py lock --dataset lofi-studio --version v1 --caption-version c1
```

Chỉ khóa khi mọi segment train/validation đã `approved` và không lỗi. Sinh JSON cho engine theo đúng định dạng ACE-Step đọc: `caption`, `lyrics: "[Instrumental]"`, `is_instrumental: true`; `bpm`, `keyscale`, `timesignature` chỉ có khi đã biết (thiếu thì engine tự ghi `N/A`).

## 5. Tạo run

Chép `configs/training.example.yaml` thành `configs/training.yaml` và sửa. Giá trị trong file mẫu chỉ minh họa cú pháp.

```bash
python scripts/train.py create --config configs/training.yaml
```

Kiểm dataset và caption đã khóa, tính SHA-256 checkpoint gốc, ghi `run.json` bất biến. In method, số tham số train / tổng, thành phần đóng băng. Chưa dùng GPU.

```bash
python scripts/train.py preflight --run <run_id>
```

Kiểm: dataset và caption còn khớp `run.json`, SHA-256 checkpoint gốc, số tensor, commit ACE-Step, Python của trainer có đủ gói, đĩa trống, VRAM trống (so với ngưỡng nếu bạn đã đặt), không có trainer khác, checkpoint resume hợp lệ. Preflight đạt **không** có nghĩa cấu hình vừa VRAM.

## 6. Preprocess [GPU]

```bash
python scripts/train.py preprocess --run <run_id> --confirm-gpu
```

Gọi `train.py fixed --preprocess` của ACE-Step cho tập train và validation; tensor ra `tensors/…/{train,validation}/`. Engine đưa audio về 48 kHz stereo. Thời gian preprocess được ghi vào `events.jsonl`, tách khỏi thời gian train.

## 7. Train [GPU]

```bash
python scripts/train.py start --run <run_id> --session-hours 4 --confirm-gpu
```

Trainer chạy nền và lệnh trả về ngay. `--session-hours` là giới hạn của phiên này; `--stop-at 06:00` dừng ở một giờ cụ thể. Các giới hạn trong `run.json` (`max_steps`, `max_epochs`, `max_hours` cộng dồn) luôn áp dụng. Dừng ở giới hạn đầu tiên đạt tới, sau khi ghi checkpoint. `--foreground` chạy ngay trong terminal.

```bash
python scripts/status.py --run <run_id>
```

```bash
python scripts/status.py --run <run_id> --log 50
```

Hiện trạng thái, step, epoch, train/validation loss, learning rate, giây/step, VRAM đỉnh, checkpoint. ETA chỉ xuất hiện sau khi phiên hiện tại đã đo ít nhất 20 step. Số liệu từng step: `runs/<run_id>/metrics.jsonl`.

**Báo cáo tiến độ.** Trainer tự ghi `runs/<run_id>/PROGRESS.md` khoảng mỗi phút, mỗi lần lưu checkpoint và khi dừng. Mở file này bất cứ lúc nào để biết: đã train bao nhiêu phần trăm, epoch thứ mấy, còn bao lâu, giới hạn nào sẽ dừng run, bảng validation loss theo epoch, checkpoint đang có, sự kiện gần đây. Trainer ghi file này mà không cần Claude Code.

```bash
python scripts/train.py stop --run <run_id>
```

Yêu cầu dừng an toàn: xong step đang chạy, ghi checkpoint, thoát. Không kill tiến trình.

```bash
python scripts/train.py resume --run <run_id> --session-hours 4 --confirm-gpu
```

Tiếp tục từ checkpoint hợp lệ mới nhất. Checkpoint hỏng bị bỏ qua và được ghi lại; không còn cái nào thì dừng với lỗi, không train lại từ đầu. `start` từ chối run đã có checkpoint; `resume` từ chối run chưa có.

| Trạng thái sau khi dừng | Ý nghĩa | Làm gì |
|---|---|---|
| `completed` (`max_steps`, `max_epochs`, `max_hours`, `early_stopping`) | Run đã tới giới hạn của nó | Đánh giá. Muốn train thêm: run mới |
| `stopped` (`session_time_limit`, `stop_at`, `stop_requested`) | Dừng an toàn, đã lưu | `resume` |
| `interrupted` | Tiến trình chết (crash, reboot, logout) | `resume`; mất phần sau checkpoint gần nhất |
| `failed` `out_of_memory` | Hết VRAM | Xem VRAM đỉnh, quyết định cấu hình mới (run mới). Workspace không tự đổi |
| `failed` `non_finite_loss` | Loss NaN/Inf | Không có checkpoint nào ghi sau lỗi. Xem `metrics.jsonl`, thường phải giảm learning rate (run mới) |
| `failed` `no_valid_checkpoint` | Mọi checkpoint hỏng | Cần người xem; không tự train lại |
| `failed` `mismatch` | Dataset, checkpoint gốc hoặc ACE-Step đã đổi | Khôi phục đúng dữ liệu của run |

Đĩa đầy khi ghi checkpoint: checkpoint dở nằm trong `.partial` và bị bỏ; checkpoint trước còn nguyên; session kết thúc `failed` kèm lỗi.

## 8. Đánh giá

```bash
python scripts/evaluate.py plan --run <run_id> --checkpoint step-00001000 --prompts configs/eval_prompts.yaml
```

Sinh `plan.json`: mỗi prompt instrumental × mỗi seed, với tên file phải render. Bộ prompt có version, viết độc lập với caption train, gồm cả prompt ngoài style.

**Render [GPU]** chưa được tự động hóa: cần engine inference nạp model gốc và model ứng viên (xuất ứng viên trước bằng `export_model.py`), render từng mục cùng seed, steps, guidance vào `parent/` và `candidate/`. Xem `docs/STATUS.md`.

```bash
python scripts/evaluate.py check --run <run_id> --checkpoint step-00001000
```

```bash
python scripts/evaluate.py ab --run <run_id> --checkpoint step-00001000
```

```bash
python scripts/evaluate.py report --run <run_id> --checkpoint step-00001000
```

`check`: toàn vẹn file, thời lượng, im lặng, clipping, mức âm. `ab`: chép từng cặp thành `pair-NNN-A.wav` / `-B.wav` theo thứ tự ngẫu nhiên, tạo `sheet.csv` để người nghe chấm melody, harmony, rhythm, arrangement, timbre, structure, lặp/đa dạng, bám prompt, chất âm, có nghe thấy vocal không; khóa giải nằm riêng ở `ab_key.json`. `report`: gộp kiểm tra kỹ thuật, sàng lọc memorization (độ giống với segment train, kèm mức tham chiếu với validation; chỉ bắt được lặp lại gần nguyên văn) và kết quả phiếu nghe.

Loss và các số tự động chỉ để sàng lọc. Chọn checkpoint là quyết định của bạn sau khi nghe. Workspace không tự duyệt model.

## 9. Export

```bash
python scripts/export_model.py export --run <run_id> --checkpoint step-00001000 --name lush-lofi-base-v1
```

```bash
python scripts/export_model.py verify --package <data_root>/exports/lush-lofi-base-v1
```

Hai thứ khác nhau:

1. **Checkpoint resume** (`runs/<run_id>/checkpoints/`): để train tiếp. Có optimizer, scheduler, RNG; không dùng để inference.
2. **Gói inference** (`exports/<name>/`): `model/<name>/` là thư mục model ACE-Step đầy đủ (weights = file gốc với decoder đã train, cùng dtype với gốc; `config.json`, file modeling, `silence_latent.pt` chép từ gốc), kèm `MANIFEST.json`, `PROVENANCE.json` (checkpoint gốc + SHA-256, method, tham số train / tổng, dataset, config, commit), `EVALUATION.md`, `IMPORT.md`, `verify_package.py`.

Gói không chứa VAE và text encoder; `PROVENANCE.json` khai báo chúng là thứ máy inference phải có sẵn. Export không cần GPU. Tensor của thành phần đóng băng được chép nguyên byte và kiểm lại; số tensor decoder thực sự khác bản gốc được ghi vào provenance như một dữ kiện, không phải tiêu chí đạt.

Đưa vào Lush Studio: làm theo `IMPORT.md` trong gói. Model đặt trên **Render Node** (máy GPU), không phải VPS. Workspace không tự chép, không đổi model mặc định, không deploy.

## 10. Autopilot: một job, không dừng lại hỏi

Thay cho các bước 3–9 khi bạn muốn giao cả quy trình. Chép `configs/autopilot.example.yaml` thành `configs/autopilot.yaml`, điền thư mục nhạc, mô tả nhạc, model, cấu hình training, điều kiện dừng, rồi **tự tay** điền `authorized_by` và ba dòng `authorization: true`.

```bash
python scripts/autopilot.py run --job configs/autopilot.yaml --detach
```

```bash
python scripts/autopilot.py status --job configs/autopilot.yaml
```

Job chạy lần lượt: scan và khóa dataset → caption → tạo run → preprocess [GPU] → train [GPU] → chọn checkpoint → xuất gói → viết `autopilot/<name>/REPORT.md`.

Kho nhạc xếp theo thể loại (`lofi/`, `jazz/`, `darkambient/`…) dùng thẳng được: mỗi thư mục một dòng trong `captions.by_folder`. Mỗi job tạo đúng một run; autopilot không tự mở thêm run, không tự đổi cấu hình, không train quá các giới hạn trong `stop`.

| Việc | Autopilot làm thế nào | Hệ quả |
|---|---|---|
| Quyền sử dụng | Ghi theo xác nhận của bạn trong job file | Trách nhiệm xác nhận là của bạn |
| Vocal | Không sàng lọc | Bài có giọng hát lẫn trong kho sẽ được train như nhạc không lời |
| Caption | Mọi segment nhận mô tả của thư mục chứa nó (`captions.by_folder`, không khớp thì `captions.default`), đánh dấu duyệt tự động | Model chỉ học được sự phân biệt mà mô tả của bạn nêu ra. Một mô tả chung cho cả kho thì model không phân biệt được các phong cách trong kho |
| Chia tập | Validation theo `split.validation`, không giữ tập test | Bạn tự test bằng tai |
| Kết thúc hẳn | `stop.epochs` hoặc `stop.total_train_hours` (bắt buộc ít nhất một) | Tới đây job xong; chạy lại không train thêm. Job không có điểm kết thúc bị từ chối |
| Dừng sớm | `stop.patience_epochs`: validation loss không cải thiện N epoch liên tiếp | Tránh train tiếp khi model đã bắt đầu học thuộc |
| Tạm dừng theo giờ | `stop.hours`, `stop.at` | Lưu checkpoint rồi dừng; chạy lại cùng job để train tiếp |
| Báo cáo | `PROGRESS.md` trong thư mục run, cập nhật khoảng mỗi phút; `REPORT.md` khi job dừng | `autopilot.py status` in cả hai |
| Checkpoint, validation | Mỗi epoch một checkpoint mốc và một lần tính validation loss | Các mốc còn lại trong thư mục run để bạn xuất cái khác |
| Chọn checkpoint | Validation loss thấp nhất; không có validation thì lấy cái mới nhất | Không phải đánh giá chất lượng âm nhạc. Chưa ai nghe |
| Gói xuất ra | `<name>-step<N>`, nhãn "chưa đánh giá" | Test theo `IMPORT.md` trong gói |

Autopilot dừng ngay và không tự sửa khi: OOM, loss NaN, preprocess lỗi, kho nhạc không đủ nhóm để chia tập. Nó cũng từ chối chạy tiếp một job nếu kho nhạc hoặc mô tả đã đổi sau khi job bắt đầu: dataset và caption đã khóa là bất biến, nên thay đổi nghĩa là job mới với `name` mới.

Không thuộc autopilot, vẫn cần bạn đồng ý riêng: cài ACE-Step, tải checkpoint, chép gói sang máy khác, đưa vào Lush Studio.

Chạy lại cùng một job sau khi đã xong không train thêm: muốn train thêm thì tăng `stop.epochs` trong một job mới.
