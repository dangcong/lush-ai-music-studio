# ARCHITECTURE — Lush Training

## Ranh giới

`lush-training/` là một gói Python độc lập nằm trong repository Lush AI Music Studio. Nó không import gì từ `apps/` hay `workers/`, không gọi API của website, không đụng database, không sửa checkout ACE-Step. Thứ duy nhất nó trao cho Lush Studio là một **gói inference** do người dùng tự chép sang Render Node.

Ba vai trò tách rời:

| Vai trò | Ai | Làm gì |
|---|---|---|
| Điều phối | Claude Code (hoặc người dùng gõ lệnh) | Chạy script, đọc trạng thái, giải thích, đề xuất |
| Vòng đời run | `session.py`, `checkpoint.py`, `run.py` | Giới hạn, dừng an toàn, checkpoint, resume, ghi số liệu. Không phụ thuộc model |
| Tính toán | `acestep_backend.py` trong tiến trình trainer | Loss, backward, cập nhật trọng số, validation |

Caption và dataset được khóa **trước** khi train. Trainer không gọi Claude; Claude hết quota hay đóng phiên không ảnh hưởng một run đang chạy.

## Module

| File | Trách nhiệm |
|---|---|
| `config.py` | `TRAINING_DATA_ROOT` / `configs/project.yaml`, bố cục thư mục dữ liệu |
| `util.py` | SHA-256, ghi nguyên tử, manifest file |
| `doctor.py` | Kiểm tra môi trường, chỉ đọc |
| `safetensors_io.py` | Đọc/ghi safetensors bằng stdlib + numpy (không cần torch) |
| `model.py` | Soi checkpoint, xác định backend tương thích, tính phần train / đóng băng |
| `acestep_probe.py` | Kiểm checkout ACE-Step đúng commit ghim và còn đủ hàm backend dùng (phân tích source, không import) |
| `audio.py` | Giải mã, số đo kỹ thuật, fingerprint thô |
| `dataset.py` | Scan, nhóm tác phẩm, vocal review, split, segment, manifest bất biến |
| `captions.py` | Phân tích segment, caption nháp, kiểm tra, khóa caption version, JSON cho engine |
| `run.py` | Cấu hình training, `run.json`, status, events |
| `checkpoint.py` | Checkpoint nguyên tử, kiểm manifest, retention |
| `session.py` | Vòng lặp một session quanh một backend; mã thoát |
| `acestep_backend.py` | Backend ACE-Step DiT + lệnh preprocess (chưa chạy trên GPU) |
| `evaluate.py` | Kế hoạch render, kiểm kỹ thuật, phiếu A/B mù, sàng lọc memorization, báo cáo |
| `export.py` | Gói inference, kiểm gói, hướng dẫn import |
| `cli.py` | Mọi lệnh; `scripts/*.py` là điểm vào mỏng |

## Bố cục `TRAINING_DATA_ROOT`

```
<TRAINING_DATA_ROOT>/
  originals/                      audio gốc nếu bạn muốn để ở đây (workspace chỉ đọc)
  pretrained/                     thư mục `checkpoints` của engine: <model>/, vae/, Qwen3-Embedding-0.6B/
  datasets/<name>/
    _work/scan.json               kết quả scan gần nhất
    _work/vocal_review.json       quyết định của người nghe
    <version>/                    BẤT BIẾN sau khi khóa
      manifest.json  manifest.sha256
      segments/{train,validation,test}/<segment_id>.wav
      analysis/<segment_id>.json  số đo, cache theo SHA-256 của segment
      captions/work/<segment_id>.json     caption đang soạn
      captions/<caption_version>/         BẤT BIẾN: captions.json, engine.train.json, engine.validation.json, LOCK.json
  tensors/<dataset>-<version>-<caption_version>-<model>/{train,validation}/   *.pt do engine preprocess
  runs/<run_id>/
    run.json                      bất biến: config, hash dataset + caption, SHA-256 checkpoint gốc, method,
                                  thành phần train / đóng băng, số tham số, commit code
    status.json  metrics.jsonl  events.jsonl  trainer.log  trainer.pid  STOP  LATEST
    PROGRESS.md                   báo cáo tiến độ cho người đọc, trainer ghi lại khoảng mỗi phút
    sessions/session-NNNN.json    môi trường từng session: dependencies, GPU, số tham số đếm lúc chạy
    checkpoints/step-NNNNNNNN/    decoder.safetensors, optimizer.pt, scheduler.pt, rng.pt, progress.json, MANIFEST.json
  eval/<run_id>/<checkpoint>/     plan.json, parent/, candidate/, ab/, ab_key.json, EVALUATION.md
  exports/<name>/                 model/<name>/, MANIFEST.json, PROVENANCE.json, EVALUATION.md, IMPORT.md, verify_package.py
  cache/                          cache scan (xóa được)
```

`segment_id` = 16 ký tự đầu SHA-256 của file gốc + vị trí bắt đầu tính bằng mili giây.

## Bất biến

- Dataset version và caption version: ghi vào thư mục `.partial` rồi đổi tên; đã tồn tại thì từ chối. Sửa manifest hay caption sau khi khóa bị phát hiện qua SHA-256.
- `run.json` không đổi sau khi tạo. Lịch learning rate tính trên `limits.max_steps` cố định trong đó, nên resume không bẻ được lịch. "Train thêm" là một run mới.
- Run ghi hash của dataset, caption và checkpoint gốc; preflight và trainer so lại trước mỗi session.

## Một session

1. Xóa thư mục checkpoint `.partial` còn sót.
2. Backend `setup`: kiểm commit ACE-Step, SHA-256 checkpoint gốc, số tensor; nạp model; đếm tham số `requires_grad` và so với `run.json`.
3. Tìm checkpoint hợp lệ mới nhất (manifest + SHA-256 từng file). Cái hỏng bị bỏ qua và ghi vào `events.jsonl`. Run đã từng có checkpoint mà không còn cái nào hợp lệ → thoát mã 22, không train.
4. Lặp: kiểm giới hạn → một optimizer step → ghi metrics → validation / checkpoint theo chu kỳ.
5. Dừng ở giới hạn đầu tiên: `max_steps`, `max_epochs`, `max_hours` (cộng dồn), early stopping, file `STOP` hoặc SIGTERM/SIGINT, `--session-hours`, `--stop-at`. Trước khi thoát luôn ghi checkpoint, trừ khi lỗi.

Mã thoát: `0` xong · `10` hết giờ (đã lưu) · `11` được yêu cầu dừng (đã lưu) · `20` OOM · `21` loss không hữu hạn · `22` không còn checkpoint hợp lệ · `23` dữ liệu không khớp `run.json` · `24` đã có trainer chạy run này · `1` crash.

Loss không hữu hạn, OOM, crash: **không** ghi checkpoint mới; checkpoint tốt gần nhất còn nguyên.

## Checkpoint

Ghi vào `step-N.partial/`, fsync từng file, ghi `MANIFEST.json` sau cùng, đổi tên thành `step-N/`, rồi cập nhật `LATEST`. Mất điện ở bất kỳ điểm nào để lại checkpoint cũ nguyên vẹn. Checkpoint không bao giờ bị ghi đè.

Mức resume công bố: trọng số decoder, optimizer, scheduler, RNG (Python, numpy, torch CPU và CUDA), thứ tự mẫu và vị trí trong lượt. Với backend giả, chuỗi loss sau resume trùng từng số với lần chạy liền [Test]. Trên GPU thật chưa đo; phép tính GPU có thể không tất định, nên chưa tuyên bố bit-exact.

`optimizer.pt`, `scheduler.pt`, `rng.pt` là pickle: chỉ nạp sau khi checkpoint qua kiểm SHA-256, và chỉ nạp file do chính workspace ghi.

Retention (nơi duy nhất workspace xóa đầu ra training, theo `checkpoint` trong `run.json`): giữ `keep_resumable` checkpoint đầy đủ mới nhất; checkpoint cũ ở mốc `milestone_every_steps` giữ lại riêng weights; cái khác bị xóa. Checkpoint hỏng không bị xóa.

## Tiến trình trainer

`train.py start|resume` khởi động trainer thành tiến trình tách rời (Windows: `DETACHED_PROCESS`; Linux: session riêng), ghi `trainer.log`, rồi trả về ngay.

| Tình huống | Hành vi |
|---|---|
| Claude Code kết thúc, đóng terminal | Trainer tiếp tục. Đã kiểm trên Windows với backend giả [Test]; chưa kiểm với trainer thật |
| Logout khỏi máy | **Chưa kiểm.** Windows thường kết thúc tiến trình của phiên đăng nhập; đừng dựa vào |
| Reboot, mất điện | Trainer chết, mất phần sau checkpoint gần nhất. Không tự chạy lại; dùng `train.py resume` |
| `train.py stop` | Tạo file `STOP`; trainer xong step đang chạy, ghi checkpoint, thoát mã 11. Không kill |
| Trainer treo | Workspace không tự kill. Người dùng quyết định |

`status.py` so `trainer.pid` với tiến trình thật: status ghi "running" mà tiến trình không còn thì báo `interrupted`.
