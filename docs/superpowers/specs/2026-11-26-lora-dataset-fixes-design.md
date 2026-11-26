# Sửa thiết kế dữ liệu train LoRA: 6 điểm

Ngày: 2026-11-26. Trạng thái: **chờ người dùng duyệt**, chưa sửa code.

Bổ sung cho `2026-11-23-lofi-style-lora-design.md` (D30) và `2026-11-23-lora-studio-web-design.md`. Khung thiết kế cũ giữ nguyên:
- Mỗi lần train là một phiên bản dataset không sửa được.
- Bài giữ riêng để chấm được tách theo bài.
- Bài có giọng người bị loại.
- Caption không chứa BPM/key.
- Train cả bài, tối đa 240 s.

## Vì sao

Run A (95 bài lẫn thể loại, 50 epoch) gần như không tạo ra chất riêng. Bản dựng tay đã có ba lỗi:
- epoch thấp;
- lẫn thể loại (neo-soul, jazz-funk, ambient);
- BPM bị đo gấp đôi (144–162).

Luồng catalog trên web chưa chặn được cả ba lỗi này. Mục tiêu: người dùng chọn bài trên web thì được cảnh báo đúng lúc, và mặc định đủ tốt để train lần đầu đã ra chất.

## 1. Epoch mặc định (web)

**Hiện tại**

`apps/web/src/features/lora/createLoraForm.ts` → `recommendedEpochs`:

| Số bài | Epoch "Chuẩn" |
|---|---|
| dưới 10 | 300 |
| 10–50 | 150 |
| trên 50 | 80 |

"Kỹ" = 1,5 × "Chuẩn". Giới hạn `max_minutes` = 120.

**Sửa**

| Số bài | Chuẩn | Kỹ |
|---|---|---|
| ≤ 25 | 500 | 800 |
| 26–60 | 300 | 500 |
| > 60 | 150 | 250 |

- Số checkpoint giữ 10 bản, nên 500 epoch lưu mỗi 50 epoch.
- Nâng giới hạn `max_minutes` lên 720 phút. Run B (20 bài × 500 epoch) mất khoảng 9 giờ.
- Sửa câu chú thích trong code: ghi nguồn là tài liệu chính thức ACE-Step cộng với kết quả Run A/B, thay cho bảng Side-Step.
- **Con số cuối chốt sau khi người dùng nghe Run B.** Nếu 500 epoch vẫn nhạt thì tăng; nếu epoch 200–300 đã đủ thì giảm.

**Test**

Cập nhật `createLoraForm.test.ts` với bảng mới. Kiểm `max_minutes` ở cả web lẫn validation API (`lora_builds.rs` và `training.rs`, nếu API có chặn).

## 2. Kiểm độ đồng nhất chất nhạc

**Hiện tại:** không có. Người dùng tự tick chọn bài.

**Sửa**

- **Worker** (`analyze_track`):
  - Tính thêm embedding MERT trung bình cho mỗi bài. Dùng lại scorer MERT có sẵn trong `evaluation.py` / `scoring.py`.
  - Tính thêm điểm chất lượng Audiobox.
  - Gửi trong `result.analysis` dưới dạng `style_embedding` (768 số thực) và `quality`.
  - Máy không có MERT thì bỏ qua, không làm hỏng job.
- **API**, ở bước xem trước build trong `lora_builds.rs`:
  - Tính tâm embedding của các bài được chọn, rồi tính cosine của từng bài tới tâm.
  - Bài có cosine thấp hơn `trung vị − 2 × MAD` bị đánh dấu "lạc chất". Ngưỡng tương đối, không phải số cứng.
  - Bài lạc chất chỉ hiện cảnh báo, không chặn, kèm danh sách bài.
  - Thêm một cảnh báo khi chọn quá 25 bài: "nhóm lớn dễ lẫn chất".
  - Bài chưa có embedding (phân tích trước bản này) hiện cảnh báo "chưa đo, hãy phân tích lại".
- **Web:**
  - Dialog tạo LoRA hiện cảnh báo, và mỗi bài lạc chất có nút bỏ chọn.
  - Bảng catalog thêm cột "Độ hợp nhóm", hiện điểm đã tính cho lần chọn gần nhất.
- **Hợp đồng:**
  - `WORKER_PROTOCOL.md`: thêm hai trường mới trong kết quả `analyze_track`.
  - `API_CONTRACTS.md`: thêm `outliers` trong phản hồi xem trước.

**Test**

- pytest: `analyze_track` có hoặc không có MERT (scorer giả).
- cargo test: một tập 6 bài có 1 bài lệch hướng thì bài đó bị đánh dấu; dưới 4 bài thì không đánh dấu.

**Rủi ro:** MERT nạp thêm khoảng 1–2 GB VRAM khi phân tích. Phân tích chạy trên rtx6000 khi không train, nên chấp nhận được.

## 3. BPM gấp đôi / gấp nửa

**Hiện tại:**
- `analysis.py::estimate_tempo` trả về giá trị librosa kèm `[bpm/2, bpm*2]`.
- API ghi thẳng giá trị librosa vào `tempo_bpm`.
- Genre đã có sẵn `bpm_min` và `bpm_max`. Lo-fi và các thể loại con đều là 60–95.

**Sửa**

Ở API, khi nhận kết quả `analyze_track` (`worker.rs`, đoạn UPDATE `catalog_tracks`):
- Lấy genre của catalog.
- Nếu BPM đo được nằm ngoài khoảng `[bpm_min, bpm_max]` mà có một phương án ÷2 hoặc ×2 nằm trong khoảng, thì ghi phương án đó, `tempo_source = 'auto'`.
- Ghi thêm `tempo_folded_from` vào JSON analysis để web hiện "đã chỉnh từ 152".
- Không có phương án nào lọt khoảng thì giữ nguyên và web hiện cảnh báo.
- Giá trị người nhập tay (`human`) không bao giờ bị đổi.

Thêm nút "Chuẩn hóa lại BPM" cho catalog đã phân tích trước bản này. Nút áp cùng luật lên JSON analysis đã lưu, không cần chạy lại GPU.

Không đặt luật này trong worker, vì worker không biết genre.

**Test (cargo test)**
- 152 → 76 trong khoảng 60–95.
- 40 → 80.
- 120 không có phương án lọt khoảng → giữ 120 và có cảnh báo.
- Giá trị `human` không đổi.

## 4. Chuẩn hóa âm lượng trước khi train

**Hiện tại:** `audio.py::to_wav48k` chỉ đổi sample rate và số kênh.

**Sửa**

- Đỉnh mỗi bài về −1 dBFS, đo bằng sample peak: hai lượt ffmpeg, đo `volumedetect` rồi áp `volume`.
- Chỉ đổi mức to nhỏ, không nén và không limiter, để giữ nguyên dynamic của bản gốc.
- Ghi mức tăng hoặc giảm (dB) vào manifest snapshot để truy lại.

Không dùng LUFS vì nó sẽ phải cắt đỉnh (clipping) ở những bài nhiều dynamic. Đây là lựa chọn của tôi, bạn có thể đổi.

**Test (pytest)**

- File sine −12 dBFS → đỉnh đầu ra −1 dBFS (±0,1).
- File im lặng → không chia cho 0, giữ nguyên.

Dataset đã có (`lofi-hiphop-core20` đã chuẩn hóa tay) không bị ảnh hưởng. Snapshot mới áp dụng luật này.

## 5. Base model trong spec

- Sửa spec D30 (`2026-11-23-lofi-style-lora-design.md`): base model là `acestep-v15-xl-sft` (D36), bỏ các config T1/T2 dành cho turbo (rank 32, 1–5 epoch), và trỏ tới mục 1 ở trên.
- **Không làm trong đợt này:** tải XL Base (khoảng 20 GB) để thử "train trên Base, chạy trên SFT". Cộng đồng khuyên cách này, nhưng nó cần tải model lớn, và bản Side-Step đang dùng chưa xác minh là hỗ trợ. Việc này làm thành một thử nghiệm riêng, **cần duyệt riêng**, sau khi có kết quả Run B.

## 6. Caption máy chưa có người duyệt

**Hiện tại**

- `caption_revisions.source` phân biệt `model` và `human`.
- Bước build chỉ chặn khi bài thiếu caption. Caption máy chưa ai xem vẫn được đem train.

**Sửa**

- **API:** bước xem trước build thêm cảnh báo "N bài dùng mô tả máy chưa duyệt", kèm danh sách. Đây là cảnh báo, không chặn, để vẫn chạy thử nhanh được.
- **Web:** mỗi bài có nút "Duyệt mô tả", ghi một bản caption `human` giống hệt bản máy. Có thêm nút duyệt nhiều bài cùng lúc. Nút này là xác nhận đã đọc; nội dung sai thì sửa như hiện nay.
- **Web:** khi caption máy có từ không khớp genre, tô vàng từ đó. Danh sách từ đáng ngờ là dữ liệu theo genre (vd. lo-fi: `brass`, `orchestral`, `cinematic`, `vocal`, `rap`), lưu ở cột mới `caption_watch_words` của genres. Không hardcode.

**Test**

- cargo test: cảnh báo đếm đúng; caption `human` không bị tính.
- vitest: nút duyệt gọi đúng API.

## Thứ tự làm và phạm vi

1. Mục 1 (epoch) và mục 5 (sửa spec): nhỏ, làm trước.
2. Mục 3 (BPM) và mục 4 (âm lượng): vừa.
3. Mục 6 (duyệt caption): vừa, có migration thêm cột `caption_watch_words`.
4. Mục 2 (độ đồng nhất): lớn nhất, đụng worker, API, web và hai hợp đồng.

Mỗi mục một commit, có test. Ghi quyết định mới vào `DECISIONS.md` (D37).

**Cần duyệt riêng sau khi code xong:**
- Push và deploy (VPS và worker rtx6000).
- Tải XL Base: không nằm trong đợt này.

**Sau khi deploy, kiểm thật:** dựng lại nhóm core20 qua luồng catalog trên web, rồi xem:
- cảnh báo lạc chất có bắt được bài neo-soul hay ambient không;
- BPM có tự về 60–95 không;
- caption có hiện cảnh báo chưa duyệt không.

Chưa train lại trong đợt này.
