# AGENTS.md — Lush Training

Hướng dẫn cho Claude Code (và người mới) khi mở **riêng** thư mục `lush-training/`. Giao tiếp và tài liệu bằng tiếng Việt; code, tên file, config key, lệnh giữ tiếng Anh.

## Workspace này là gì

Nơi chuẩn bị dataset, fine-tune, đánh giá và xuất model **nhạc instrumental** từ một pretrained checkpoint ACE-Step. Nó chạy độc lập: không cần VPS, website hay worker của Lush Studio. Không có vocal training, lyrics editor hay singer identity, và không thêm những thứ đó.

Bạn (Claude Code) điều phối script và giải thích kết quả. Trainer là tiến trình Python riêng; chính nó tính loss, backward và cập nhật trọng số. Training không cần bạn trong từng step và vẫn chạy khi phiên của bạn kết thúc.

## Dataset Factory cho Train Style Studio (D30)

Ngoài fine-tune decoder, workspace còn dùng làm **Dataset Factory** cho Train Style LoRA của Lush Studio: `prepare_dataset` → `validate_captions init` (BPM/key có độ tin cậy) → `draft_captions` → người dùng duyệt trên `review_server` → `pilot_report` → `validate_captions lock` → `export_studio_dataset` sang `data/datasets/` mà worker Studio đọc. Factory **không train**: train chạy qua job `train_lora` của Studio. `render_ab` render A/B qua `acestep-api` (cần `--confirm-gpu`). Bài có giọng người bị loại cả bài.

## Đầu mỗi phiên

1. Đọc `docs/STATUS.md`: cái gì đã kiểm chứng, cái gì chưa. Không coi phần chưa kiểm chứng là chạy được.
2. Chạy `python scripts/doctor.py` (chỉ đọc) để biết máy này có gì.
3. Chạy `python scripts/status.py` để xem các run đang có. Trạng thái nằm trên đĩa, không nằm trong lịch sử trò chuyện.
4. Chỉ mở thêm tài liệu khớp với việc đang làm:

| Việc | Đọc |
|---|---|
| Lệnh và quy trình từng bước | `docs/TRAINING_GUIDE.md` |
| Người dùng nhờ việc gì thì làm gì | `docs/CLAUDE_WORKFLOW.md`, `prompts/` |
| Model nào train được, method nào có | `docs/MODEL_COMPATIBILITY.md` |
| Cấu trúc code, nơi lưu dữ liệu | `docs/ARCHITECTURE.md` |

## Cấu hình và dữ liệu

- `configs/project.yaml` (chép từ `project.example.yaml`, không commit) hoặc biến `TRAINING_DATA_ROOT` chỉ tới vùng dữ liệu **ngoài Git**: `datasets/`, `tensors/`, `pretrained/`, `runs/`, `eval/`, `exports/`, `cache/`.
- Không ghi audio, weights, checkpoint hay dataset vào repository. Không gắn cứng đường dẫn của một máy vào code.
- Audio gốc chỉ đọc. Không sửa, không ghi đè, không chuyển chỗ.

## Quy trình

`doctor` → `inspect_model` → dataset (`scan` → người dùng nghe file bị gắn cờ → `build`) → caption (`init` → điền và duyệt → `lock`) → `train create` → `preflight` → `preprocess` → `start` → `status` → `stop` / `resume` → evaluation → `export`. Chi tiết: `docs/TRAINING_GUIDE.md`.

Một run tiếp tục được nhờ `runs/<run_id>/`: `run.json` (bất biến), `status.json`, `metrics.jsonl`, `events.jsonl`, `checkpoints/`. Muốn tiếp tục thì đọc các file đó, không đoán.

## Cần người dùng phê duyệt trước, từng lần

- Tải bất kỳ model, checkpoint hay công cụ có weights nào.
- Mọi lệnh dùng GPU: `train.py preprocess`, `train.py start`, `train.py resume` (chúng đòi `--confirm-gpu`; chỉ thêm cờ này khi người dùng đã đồng ý cho đúng lần chạy đó), và render mẫu đánh giá.
- Cài hoặc đổi driver, WSL2, firewall, cấu hình hệ điều hành; dừng dịch vụ hay tiến trình không phải trainer của workspace này.
- Xóa dữ liệu ngoài chính sách retention đã ghi trong `run.json`.
- Chép gói model sang máy khác, đưa vào Lush Studio, deploy, `git push`.
- Gọi dịch vụ trả phí. Không dùng API key, không đọc hay chép cookie, token đăng nhập.

Được làm không cần hỏi: đọc file, `doctor`, `inspect_model`, `status`, `prepare_dataset scan`, `analyze_audio`, `validate_captions init/validate`, `train create/preflight`, `evaluate check/report`, `export_model verify`, chạy `pytest`.

## Autopilot (ủy quyền thường trực theo job)

Người dùng có thể giao cả quy trình trong một job file (`configs/autopilot.yaml`, quy trình ở `prompts/AUTOPILOT.md`). Khi họ **tự điền** `authorized_by` và ba dòng `authorization: true`, job đó được phép, không hỏi lại: dùng GPU tới điều kiện `stop`, đánh dấu duyệt caption lấy từ mô tả trong job file, chọn checkpoint theo validation loss và xuất gói.

- Bạn không tự điền các dòng ủy quyền. Bạn soạn phần còn lại và giải thích từng dòng.
- Ủy quyền chỉ áp cho job đó, kể cả khi chạy lại để train tiếp. Nó không bao gồm cài đặt, tải model, chép gói đi nơi khác, deploy, xóa dữ liệu.
- Mọi quyết định tự động được ghi rõ là tự động (trong caption, manifest, `REPORT.md`). Khi báo cáo, không gọi chúng là kết quả nghe hay đánh giá chất lượng.
- Các quy tắc dưới đây vẫn giữ: autopilot dừng khi lỗi, không tự đổi method hay cấu hình, không train lại từ step 0.

## Quy tắc không được phá

- **Không tự đổi training method.** `full_decoder`, `partial_decoder`, `full_model`, `lora` là bốn thứ khác nhau (`docs/MODEL_COMPATIBILITY.md`). Người dùng yêu cầu cái nào thì làm cái đó; không khả thi thì dừng và báo, không thay bằng LoRA. Cập nhật toàn bộ decoder không gọi là cập nhật toàn bộ model.
- **Không tự đổi cấu hình khi lỗi.** OOM, NaN, hết đĩa: báo nguyên nhân kèm số đo, đề xuất, chờ quyết định. Không tự giảm batch, đổi optimizer hay hạ learning rate.
- **Không bắt đầu lại từ step 0** một run đã có checkpoint. Checkpoint hỏng thì trainer lùi về cái hợp lệ trước đó; không còn cái nào thì đó là lỗi cần người xem.
- **Không bịa phân tích audio.** Bạn không nghe được file WAV chỉ vì có đường dẫn. Chỉ dùng số đo do script trả về và điều người dùng nói. Trường chưa có bằng chứng để `null`.
- **Chỉ người dùng duyệt** caption (`status: approved`), quyết định vocal của một file, và chọn checkpoint. Bạn đề xuất; `--human` chỉ dùng khi ghi lại quyết định của chính họ. Ngoại lệ duy nhất: một job autopilot mà họ đã ủy quyền.
- **Detector vocal chỉ là tín hiệu sàng lọc.** File nghi có singing, speech, humming, vocal chops phải được người dùng nghe. Không tự tách vocal rồi coi phần còn lại là sạch.
- **Trung thực về kiểm thử.** Test với backend giả chứng minh vòng đời run, không chứng minh training trên GPU. Loss và điểm tự động không phải bằng chứng nhạc hay hơn. Không hứa một cấu hình vừa VRAM khi chưa đo.
- Hai GPU ở hai máy không cộng gộp VRAM. Mặc định một máy, một GPU, một run.

## Lệnh phát triển

```bash
py -m venv .venv
.venv\Scripts\python -m pip install -e ".[dev]"
.venv\Scripts\python -m pytest -q
.venv\Scripts\python -m pytest tests/test_session.py -k resume
```

Linux/WSL2: `python3 -m venv .venv && .venv/bin/python -m pip install -e ".[dev]"`.

Sau khi đổi code: chạy test, cập nhật `docs/STATUS.md` nếu mức kiểm chứng của một hạng mục đổi. Kết quả đo thật trên GPU ghi vào `docs/STATUS.md` kèm ngày, máy, cấu hình.
