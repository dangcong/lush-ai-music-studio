# Thiết kế: Quy trình tạo Style LoRA trên website (Catalog → Mô tả → Tạo LoRA → Nghe & chọn bản)

Ngày: 2026-11-23. Trạng thái: **đã duyệt 2026-11-23** (người dùng: "tôi duyệt").
Mockup đã duyệt khi trao đổi (trong `.superpowers/brainstorm/1672-1795426550/content/`, git-ignored): `library-v3.png` (catalog), `create-lora.png` (hộp Tạo LoRA), `after-train.png` (sau khi train; có chỉnh sửa ở mục 6.4).

## 1. Mục đích và lý do

Người dùng có khoảng 250 bài Lo-fi instrumental tự sản xuất, toàn quyền sử dụng. Họ muốn làm **toàn bộ** quy trình train Style LoRA ngay trên website Lush Studio, không qua CLI hay trang localhost:

1. Tạo catalog.
2. Tải nhạc lên.
3. Cho máy viết mô tả bằng model tự chọn, và so sánh các model.
4. Chọn bài, đặt tên, train LoRA.
5. Nghe các bản lưu và chọn một bản đưa vào Style Preset.

Đây là giai đoạn **thử nghiệm**. Người dùng muốn đổi model mô tả dễ dàng để tìm ra model viết mô tả tốt nhất cho nhạc của mình.

Thiết kế này **thay thế** cách làm trong `2026-11-23-lofi-style-lora-design.md` (Dataset Factory chạy bằng CLI và trang duyệt localhost) ở các phần giao diện, duyệt và xuất dataset. Phần BPM/key, cách đọc dataset của trainer và quy tắc loại bài có giọng vẫn dùng tiếp. Code trên nhánh `feat/lofi-dataset-factory` (trong `lush-training/`) là tài liệu tham khảo để chuyển logic sang worker. Không import trực tiếp (D28).

## 2. Mục tiêu, không làm, phản mục tiêu

**Mục tiêu**
- G1. Catalog là một bảng sống trên website, mỗi bài một dòng. Tải thêm nhạc vào bất cứ lúc nào.
- G2. Tempo và Key tự đo ngay khi tải lên.
- G3. Mô tả bài (Mood, Instruments, Vocals, Music caption) được tạo bằng model người dùng chọn. Mỗi bài giữ riêng kết quả của từng model, để so sánh và chuyển qua lại.
- G4. Tạo LoRA từ các bài đang chọn, với tùy chọn train. Kết quả là một bản chụp bất biến.
- G5. Sau khi train có nhạc nghe thử cho từng bản lưu (epoch), điểm máy chấm và ghi chú. Người dùng chọn một bản đưa vào Style Preset.

**Không làm**
- Fine-tune base model (T-012).
- Tải nhạc từ Internet, tự tách vocal, nhạc có lời.
- Chạy nhiều job GPU cùng lúc trên một node. V1 vẫn chỉ một job mỗi node.
- Hiện thông số model (dung lượng, VRAM, tốc độ, giấy phép) trên giao diện. Người dùng đã yêu cầu như vậy.
- Thanh chấm "Đúng câu lệnh" (CLAP). Người dùng đã chọn không làm.

**Phản mục tiêu** (chạy được về kỹ thuật nhưng vẫn coi là thất bại)
- Đổi model ở một bài làm mất kết quả của model khác.
- Sửa catalog sau khi train làm thay đổi hồ sơ "LoRA này được train từ".
- Bài có giọng hát lọt vào train mà không ai được báo.
- Train hoặc tạo mô tả làm hỏng việc tạo nhạc mà người dùng không được báo trước.
- Bản LoRA được chọn chỉ vì là epoch cuối.

## 3. Hiện trạng liên quan (đã kiểm 2026-11-23)

- **Kiến trúc:** VPS `lushvietnix2` (control plane: API Rust + SQLite + artifacts) và node `rtx6000` (worker Python + `acestep-api`). Worker luôn chủ động gọi VPS. VPS **không** xử lý audio, tải model hay train (quy tắc người dùng).
- **Upload audio** đã có: `routes/inputs.rs`. Worker tải file đầu vào qua `GET /jobs/{id}/inputs/{artifact_id}` và so SHA-256.
- **Job `train_lora`** đã chạy (M1), nhưng hiện đọc dataset là **thư mục có sẵn trên node** (`data/datasets/<key>/` với `dataset.json` + `audio/`), node báo danh sách khi register. Các bước của worker: dừng `acestep-api` → preprocess Side-Step (`--max-duration 240`) → train → kiểm adapter → bật lại inference → LoRA vào Registry `evaluation`. Adapter không tải lên VPS (D9).
- **Style Preset** đã có `lora_scale`. LoRA được dùng qua Style Preset, không có ô chọn LoRA riêng.
- **Web:** mục "Datasets" ở sidebar đang là "Sắp có". Form `TrainingRunForm` đã có các ô và giá trị mặc định đã thử: rank 32, alpha 64, lr 0.0001, epochs 4, grad accum 4, save_every 2, warmup 2, max 30 phút.
- **Chưa có:** catalog, job phân tích hay mô tả, job đánh giá, nhiều bản lưu (checkpoint) cho một LoRA trong Registry.

## 4. Quyết định đã chốt với người dùng

| # | Quyết định |
|---|---|
| W1 | Mỗi **catalog** là một sheet trên website (ví dụ "Lo-fi chill 2026"). Không có bước "tạo data sheet" riêng. Muốn thêm nhạc thì tải vào catalog. Genre đặt một lần cho cả catalog, không có cột Genre ở từng bài |
| W2 | Một bài một dòng. Cột: chọn, Bài (nghe được), **Tech stack**, Tempo, Key, Mood, Instruments, Vocals, Music caption (tiếng Anh) |
| W3 | **Tempo và Key tự đo khi tải lên**, dùng chung cho mọi model. Đổi model không làm đổi hai ô này. Sửa tay được |
| W4 | 4 model mô tả: **ACE-Step Captioner, MOSS-Music 8B, Qwen3-Omni Captioner, NVIDIA Music Flamingo**. Dùng cả Flamingo, không chặn. Giao diện không hiện thông số model |
| W5 | Mỗi bài có **kết quả riêng cho từng model**. Bài mới tải lên chưa có mô tả. Chọn model rồi bấm chạy thì mới tạo. Đổi sang model chưa chạy thì các ô trống. Chọn lại model đã chạy thì hiện ngay kết quả cũ |
| W6 | Tick nhiều bài rồi chọn model ở menu và bấm chạy. **Bài nào đã có kết quả của model đó thì bỏ qua** |
| W7 | **Chạy lại** ghi đè kết quả hiện tại, kể cả phần đã sửa tay. Có nút **Quay về** để lấy lại bản trước. Sửa tay từng ô được |
| W8 | Cột Tech stack có dấu chấm cho biết bài đã có kết quả của những model nào |
| W9 | **Không có bước Duyệt/Loại.** Chọn bài bằng tick, "Chọn tất cả" hoặc nhập số lượng → nút **Tạo LoRA** → đặt tên và chọn tùy chọn |
| W10 | Khi tạo LoRA, mỗi bài dùng mô tả của **model đang hiện ở cột Tech stack của bài đó**. Được trộn model trong một LoRA |
| W11 | Hộp Tạo LoRA chia 3 nhóm: **Cần điền** (tên, từ khóa gọi style) → **Nên chỉnh** (kiểu train, epoch, lưu mỗi N epoch, số bài giữ lại, câu lệnh nghe thử) → **Nâng cao**, thu gọn (rank, alpha, LR, grad accum, warmup, seed, giới hạn thời gian, độ dài tối đa mỗi bài, base model) |
| W12 | Mặc định **giữ lại 2 bài** để nghe thử, không đưa vào train. Đặt 0 được |
| W13 | Sau khi train: mỗi epoch một dòng; người dùng **chỉ ghi chú** (không chấm sao) |
| W14 | Máy chấm: **MERT** cho "Giống style" và "Giống bài train", thêm so giai điệu bằng librosa để bắt chép nốt. **Audiobox Aesthetics** cho "Chất lượng âm thanh". Điểm chỉ để tham khảo |
| W15 | Người dùng chọn **một** epoch để đưa vào Style Preset |
| W16 | Các bản lưu không được chọn: **tự xóa sau 30 ngày**, kèm nút **Xóa ngay** |
| W18 | Đưa vào Style Preset = LoRA `approved`, Style dùng được cho **mọi người dùng** ngay |
| W17 | Vẫn giữ từ trước: một job GPU mỗi node; cài hay tải model và chạy GPU lần đầu đều cần người dùng duyệt; không đổi production khi chưa được phép; bài có giọng người bị loại khỏi train (U6); bài dài hơn 240 s chỉ dùng 240 s đầu (U7) |

## 5. Giấy phép (ghi lại, không chặn)

Người dùng chọn dùng cả các model phi thương mại cho giai đoạn thử nghiệm:

- **Không cho dùng thương mại:** Music Flamingo (NVIDIA OneWay Noncommercial), MERT-v1-330M (CC BY-NC 4.0).
- **Cho dùng thương mại:** MOSS-Music (Apache 2.0), Qwen3-Omni Captioner (Apache 2.0), Audiobox Aesthetics (CC BY 4.0, có một phần MIT).
- **Chưa rõ:** ACE-Step Captioner. Thẻ model ghi MIT, nhưng repo không có file LICENSE.

Hệ thống không chặn model nào. Bù lại, mỗi mô tả lưu tên model đã viết ra nó, và hồ sơ LoRA ghi số bài theo từng model, để sau này lọc được (ví dụ "LoRA không dùng mô tả Flamingo"). Trước khi tải mỗi model, tôi vẫn đọc LICENSE và báo dung lượng để người dùng duyệt.

## 6. Thiết kế

### 6.1 Màn hình Catalog (`/datasets` → đổi tên thành "Catalog")

- **Danh sách catalog:** tên, genre, số bài, tổng phút, số LoRA đã tạo, nút **Tạo catalog**. Tạo catalog cần nhập tên và genre (chọn từ Genre Registry).
- **Trang một catalog:** theo `library-v3.png`.
  - Đầu trang: nút **⬆ Tải nhạc lên** (nhiều file, kéo-thả, MP3/WAV/FLAC) và **Tạo LoRA (N bài đã chọn)**.
  - Khi có bài được tick, thanh chọn hiện ra với: số bài đã chọn, **Chọn tất cả**, ô **Chọn N bài** (*lựa chọn của tôi: chọn N bài đầu theo thứ tự đang hiển thị*), menu **Model tạo mô tả**, nút **Tạo mô tả cho N bài** và **Xóa khỏi catalog**.
- **Mỗi dòng:**
  - Tên file, trình phát, thời lượng. Bài > 240 s ghi "train 4 phút đầu".
  - **Tech stack:** menu chọn model, dấu chấm cho các model đã có kết quả, và nút **Tạo / Chạy lại / Quay về** cho riêng bài đó.
  - **Tempo, Key:** sửa được. Dòng chữ xanh "Máy: 78 (tin cậy cao)" là giá trị đo. Đo không chắc thì ô trống và hiện các khả năng, ví dụ "82 hoặc 164".
  - **Mood, Instruments, Vocals, Caption:** sửa được, thuộc model đang chọn.
  - **Trạng thái:** Mới tải lên / Đang đo / Đang tạo mô tả… / Có mô tả / Lỗi (kèm nút Thử lại).
  - Bài có mô tả nhắc tới giọng người (từ Vocals hoặc caption) có cảnh báo "Có thể có giọng" (*lựa chọn của tôi*).
- **Xóa bài:** ẩn khỏi catalog (*lựa chọn của tôi: xóa mềm*). File gốc được giữ khi còn LoRA nào đã train từ nó. Không còn LoRA nào dùng thì xóa thật sau 30 ngày.
- Trùng file (cùng SHA-256) trong một catalog: từ chối khi tải lên và báo tên bài đã có (*lựa chọn của tôi*).

### 6.2 Dữ liệu mô tả theo model (W5–W8)

Với mỗi cặp (bài, model) có một **ngăn mô tả** chứa danh sách các **lượt** (revision) theo thời gian. Mỗi lượt gồm: mood, instruments, vocals, caption, nguồn (`model@revision` hoặc `human`), thời điểm và người làm.

| Thao tác | Kết quả |
|---|---|
| Tạo / Chạy lại | Thêm một lượt mới, nguồn = model. Lượt mới thành bản hiện tại |
| Sửa tay một ô | Thêm một lượt mới, chép bản hiện tại và đổi ô đó, nguồn = `human` (*lựa chọn của tôi: sửa tay cũng là một lượt, để Quay về lấy lại được*) |
| Quay về | Bản hiện tại lùi về lượt trước. Lượt sau không bị xóa, nên bấm "Tới" để trở lại được (*lựa chọn của tôi*) |
| Đổi model ở cột Tech stack | Chỉ đổi con trỏ "model đang hiện" của bài. Không xóa gì |

Tempo/Key nằm ở cấp **bài**, không theo model. Chúng có lịch sử đơn giản: giá trị đo, giá trị người sửa, nguồn.

**Cách mỗi model điền 4 ô** (*lựa chọn của tôi*):

- **Model nhận câu lệnh (MOSS-Music, Music Flamingo):** worker yêu cầu trả JSON gồm `mood`, `instruments`, `vocals`, `caption`. JSON hỏng thì thử lại một lần. Vẫn hỏng thì đưa toàn bộ câu trả lời vào ô Caption, còn Mood, Instruments, Vocals được rút từ câu trả lời đó bằng từ khóa như nhóm model chỉ viết mô tả (cập nhật khi làm P1).
- **Model chỉ viết mô tả (ACE-Step Captioner, Qwen3-Omni Captioner):**
  - Ô Caption nhận nguyên văn câu mô tả.
  - Mood, Instruments, Vocals được rút từ caption bằng danh sách từ khóa cố định (nhạc cụ, mood, từ chỉ giọng người).
  - Không dùng thêm model thứ hai.
- **Cảnh báo giọng người:** áp cho mọi model. Bật khi Vocals khác "Instrumental", hoặc khi caption chứa từ chỉ giọng người (vocal, voice, singing, hum, chant, spoken…).

**Quyền:** catalog dùng chung cho mọi `admin` và `trainer`, không thuộc riêng người tạo (*lựa chọn của tôi*). Mỗi lượt mô tả ghi người làm.

### 6.3 Hộp Tạo LoRA (`create-lora.png`)

- **Đầu vào:** các bài đang chọn.
- **Kiểm tra trước khi cho bấm "Bắt đầu train":**
  - Mọi bài có mô tả ở model đang hiện. Bài nào chưa có thì liệt kê, kèm nút **Tạo mô tả** hoặc **Bỏ bài này**.
  - Bài có cảnh báo giọng người được liệt kê. Người dùng phải **Bỏ bài** hoặc **Xác nhận không có giọng** cho từng bài (U6, W17).
  - Tempo và Key: thiếu thì vẫn cho train, ghi `N/A` như upstream (*lựa chọn của tôi*). Mockup ghi "Mọi bài có Tempo và Key" là kiểm cứng; tôi đổi thành cảnh báo.
  - Tên LoRA không trùng.
- **Cần điền:**
  - Tên LoRA (bắt buộc).
  - Từ khóa gọi style, mặc định trống (*lựa chọn của tôi*). Nếu có thì thêm vào đầu mọi caption khi train và tự gắn vào Style Preset.
- **Nên chỉnh:**
  - Kiểu train: Thử nhanh 1 epoch / Chuẩn 4 / Kỹ 8 / Tự nhập.
  - Epoch.
  - Lưu bản mỗi N epoch, mặc định 1.
  - Số bài giữ lại, mặc định 2 (W12), chọn ngẫu nhiên có seed và hiện tên ra.
  - Câu lệnh nghe thử: điền sẵn từ genre và từ khóa, sửa được.
- **Nâng cao:** giá trị mặc định như form hiện có (mục 3). Độ dài tối đa mỗi bài là 240 s, cố định trong lần này (U7). Base model là ACE-Step 1.5 turbo.
- **Cột phải:**
  - Số bài, số bài train / giữ lại, tổng phút.
  - Thanh màu số bài theo model mô tả.
  - Dự kiến: số bước train, số bản lưu, số bài nghe thử. Thời gian ước theo lần train trước, nếu chưa có thì để "—".
  - Cảnh báo: trong lúc train, rtx6000 không tạo nhạc được.
- **Bấm "Bắt đầu train":** API tạo một **bản chụp dataset** bất biến gồm danh sách bài (artifact ID + SHA-256), caption cuối cùng (đã gắn từ khóa), tempo, key, model nguồn của từng bài, bài giữ lại và toàn bộ config. Sau đó API tạo job `train_lora`. Sửa catalog về sau không làm đổi bản chụp này.

### 6.4 Màn hình sau khi train (theo `after-train.png`, có chỉnh)

- **Tiến độ 4 bước:**
  1. Chuẩn bị dữ liệu.
  2. Train: epoch, bước, loss.
  3. Tạo nhạc nghe thử và chấm điểm.
  4. Bạn nghe và chọn bản.
  Đóng trang thì hệ thống vẫn chạy. Có nút **Hủy**.
- **Bài mẫu, hiện một lần ở đầu bảng:**
  - Câu lệnh nghe thử chạy bằng ACE-Step gốc (không LoRA).
  - Bài thật của mỗi bài giữ lại.
- **Bảng bản lưu, mỗi epoch một dòng:**
  - **Câu lệnh nghe thử, có LoRA.**
  - **LoRA tạo lại từng bài giữ lại**, dùng caption của bài đó làm câu lệnh.
  - **Máy chấm:** ba thanh "Giống style", "Giống bài train", "Chất lượng âm thanh", kèm cảnh báo đỏ "gần giống <tên bài>" khi vượt ngưỡng.
  - **Ghi chú** của người dùng.
  - Bấm vào dòng để chọn bản.
- **Biểu đồ loss** theo bước, có vạch chia epoch.
- **Cột phải:**
  - **Đưa vào Style Preset:** bản đã chọn, tên Style, độ mạnh mặc định 0.7 (*lựa chọn của tôi*), nút đưa vào.
  - **Thử nhanh:** gõ câu lệnh riêng → tạo một bài bằng bản đang chọn. Đây là job generate thường có LoRA.
  - **LoRA này được train từ:** đọc từ bản chụp, có link xem danh sách bài và mô tả đã dùng.
- **Trên cùng:**
  - **Train lại với tùy chọn khác:** mở lại hộp Tạo LoRA với cùng bài và config, điền sẵn.
  - **Xóa LoRA.**
  - Ghi chú "các bản không chọn tự xóa sau 30 ngày", kèm nút **Xóa ngay**.

### 6.5 Job mới trên worker

Mọi việc xử lý audio và model đều chạy trên node, qua hàng đợi như job thường. Mỗi job một node, một lúc một job.

| Job | Tạo khi | Worker làm | Trả về VPS |
|---|---|---|---|
| `analyze_track` | Bài vừa tải lên | Tải file, đo duration, tempo, key (librosa + độ tin cậy, lấy từ `lush-training` sang), loudness, clipping, im lặng | JSON phân tích |
| `caption_tracks` | Bấm Tạo mô tả cho một nhóm bài với một model | Nạp model **một lần**, mô tả lần lượt từng bài, báo tiến độ theo bài | Mỗi bài một lượt mô tả (mục 6.2) |
| `train_lora` (mở rộng) | Bấm Bắt đầu train | Tải audio từ bản chụp, dựng `data/datasets/<lora-key>/` (dataset.json + WAV 48 kHz) rồi chạy các bước như hiện nay. Lưu mọi bản lưu theo `save_every` | Như hiện nay, kèm danh sách bản lưu (epoch, bước, SHA-256) và loss theo bước |
| `eval_lora` | Ngay sau khi `train_lora` xong (*lựa chọn của tôi: job riêng, để train lỗi hay hủy không kéo theo đánh giá*) | Render bài mẫu không LoRA, rồi với mỗi bản lưu render câu lệnh nghe thử và từng bài giữ lại. Chấm MERT, librosa, Audiobox | Các file nhạc nghe thử (artifact, lên VPS như track thường) và điểm |

- **Ưu tiên:** `generate` > `analyze_track` > `caption_tracks` > `eval_lora` > `train_lora`. `train_lora` vẫn giữ luật "chỉ khi không còn job generate chờ" (*lựa chọn của tôi*).
- **Model mô tả và model chấm** chạy trong **venv riêng của từng model** dưới `AI_DATA/AiStudio/models/<tên>/`, được worker gọi dưới dạng tiến trình con. Lý do: mỗi model cần phiên bản transformers riêng, MOSS cần SGLang fork. Không đụng venv của ACE-Step. Node chỉ báo một model trong `capabilities` khi venv và trọng số của model đó đã cài. Model chưa cài thì vẫn hiện trong menu nhưng bị mờ, ghi "Chưa cài trên máy" (*lựa chọn của tôi*).
- **Danh sách model mô tả và model chấm là dữ liệu**, lấy từ Model Registry (loại `captioner` / `scorer`), không hardcode (quy tắc AGENTS.md).
- **Tạo mô tả và chấm có chạy song song với `acestep-api` không?** Có (96 GB VRAM đủ), nhưng node vẫn chỉ nhận một job một lúc, nên chạy mô tả nghĩa là job tạo nhạc phải chờ. Giao diện báo điều này khi bấm Tạo mô tả cho nhiều bài.
- **Chấm điểm (W14):**
  - **Giống style:** độ tương đồng cosine giữa embedding MERT của bài tạo ra và trung bình embedding các bài train.
  - **Giống bài train:** độ tương đồng lớn nhất với từng bài train, theo MERT và theo giai điệu (chroma + DTW bằng librosa). Vượt ngưỡng thì hiện tên bài.
  - **Chất lượng âm thanh:** điểm PQ của Audiobox.
  - Ngưỡng ban đầu do builder đặt, ghi rõ là chưa hiệu chỉnh.
- **Cấu hình render khi đánh giá** (*lựa chọn của tôi*):
  - Mỗi lần render dài 60 giây, 1 seed cố định, lưu trong bản chụp.
  - Bài mẫu không LoRA render một lần.
  - Số lần render là `(số bản lưu) × (1 + số bài giữ lại) + 1`. Với Chuẩn 4 epoch, lưu mỗi epoch, 2 bài giữ lại thì có 13 bài.
  - Hộp Tạo LoRA hiện số này trước khi bấm train.

### 6.6 Bản lưu và Style Preset

- Một LoRA trong Registry có thêm danh sách **bản lưu** (checkpoint). Mỗi bản có epoch, bước, đường dẫn trên node, SHA-256, ghi chú, trạng thái (`kept` / `chosen` / `deleted`) và thời điểm hết hạn.
- **Chọn một bản và bấm "Đưa vào Style Preset":**
  - LoRA dùng bản đó làm adapter.
  - LoRA chuyển trạng thái `approved`, nên **mọi người dùng** (kể cả `member`) đều dùng được Style đó ngay (người dùng chốt 2026-11-23).
  - Tạo Style Preset mới gắn LoRA, `lora_scale` 0.7 và từ khóa (nếu có).
  - Đổi bản đã chọn về sau được: cập nhật Style đã tạo, không tạo Style mới (*lựa chọn của tôi*).
- **Xóa bản lưu:** VPS đánh dấu, worker xóa file khi rảnh. Việc dọn theo hạn 30 ngày cũng do worker làm, theo danh sách VPS gửi khi heartbeat (*lựa chọn của tôi*). Bản đang `chosen` không bao giờ bị xóa tự động.
- **Hạn 30 ngày bắt đầu từ lúc người dùng chọn bản** (*lựa chọn của tôi*). LoRA chưa chọn bản nào thì giữ mọi bản lưu, không tự xóa. Như vậy LoRA không bao giờ mất hết bản lưu.
- **Xóa LoRA:**
  - Bị chặn khi còn Style Preset dùng LoRA đó. Web báo tên các Style để gỡ trước.
  - Track đã tạo bằng LoRA vẫn giữ nguyên.
  - Hồ sơ LoRA và bản chụp dataset chuyển `archived`, không xóa hẳn (*lựa chọn của tôi*).
- **Luồng train cũ** (`/training-runs`, form hiện có, dataset có sẵn trên node): giữ chạy được cho các LoRA M1 đã có (*lựa chọn của tôi*). Nút tạo run mới chuyển sang luồng catalog. Trường `datasets` trong `register` giữ nguyên để tương thích. Không chuyển đổi dữ liệu cũ.

### 6.7 Quyền

- Catalog, Tạo LoRA, đưa vào Style: vai trò `admin` và `trainer` (*lựa chọn của tôi*, khớp với trang Training hiện có).
- `member` không vào được Catalog, nhưng dùng được mọi Style đã gắn LoRA `approved` (W18).

## 7. Phân kỳ (ba phần, làm và nghiệm thu lần lượt)

1. **P1 – Catalog và mô tả:** catalog, tải lên, `analyze_track`, `caption_tracks` với **một** model đầu tiên. Thêm từng model mô tả khác khi người dùng duyệt tải. *(Lựa chọn của tôi: model đầu tiên là MOSS-Music 8B Instruct, vì giấy phép rõ ràng và điểm caption cao nhất theo tác giả. Chạy bằng transformers theo `infer.py` của tác giả, chưa dùng bản SGLang fork, để cài đặt nhẹ. Chỉ chuyển sang SGLang nếu đo thấy quá chậm.)*
2. **P2 – Tạo LoRA từ catalog:** bản chụp dataset, `train_lora` nhận bản chụp, nhiều bản lưu.
3. **P3 – Sau khi train:** `eval_lora`, máy chấm, màn hình chọn bản, đưa vào Style, dọn bản lưu.

Mỗi phần cần sửa cả hai hợp đồng (`API_CONTRACTS.md`, `WORKER_PROTOCOL.md`) trong cùng thay đổi, và có migration mới. Mỗi phần cần deploy lên VPS và rtx6000, nên phải được người dùng duyệt riêng.

## 8. Ràng buộc

- Mọi việc audio, model và train chạy trên `rtx6000`. **Không** chạy trên `lushvietnix2`.
- Tải bất kỳ model nào (4 model mô tả, MERT, Audiobox) đều cần người dùng duyệt sau khi tôi báo LICENSE, dung lượng và cách chạy.
- Giữ nguyên mount AI_DATA. Mọi file mới nằm dưới `/media/ai-rtx6000/AI_DATA/AiStudio/`.
- Không commit nhạc, dataset hay trọng số. Nhạc người dùng tải lên nằm trong storage artifact của VPS, giống audio input hiện nay.
- Theo quy tắc AGENTS.md: chuyển trạng thái job là câu SQL nguyên tử; SQL runtime; migration mới, không sửa migration cũ; API chỉ lộ Artifact ID.

## 9. Nghiệm thu

1. **P1:**
   - Tạo catalog và tải 3 bài thật. Tempo/Key tự hiện.
   - Tạo mô tả bằng model A, đổi sang model B: ô trống. Tạo B, chọn lại A: hiện kết quả A.
   - Chạy lại rồi Quay về: lấy lại đúng bản trước, kể cả phần sửa tay.
   - Tick nhiều bài có bài đã có kết quả: bài đó được bỏ qua.
   - Trong lúc mô tả, một bài tạo nhạc vẫn được xếp hàng và chạy sau.
2. **P2:**
   - Tạo LoRA với bài trộn 2 model: `dataset.json` trên node khớp bản chụp, đúng caption của model đang hiện ở từng bài.
   - Sửa catalog sau đó không làm đổi bản chụp.
   - Bài cảnh báo giọng người không vào được train khi chưa xác nhận.
   - Một run "Thử nhanh" chạy thật trên rtx6000 có số đo VRAM và thời gian. Inference bật lại, một bài tạo thường vẫn ra đúng.
3. **P3:**
   - Mỗi bản lưu có nhạc nghe thử và điểm.
   - Chọn epoch, đưa vào Style, tạo một bài bằng Style đó.
   - "Xóa ngay" xóa file trên node. Bản đã chọn không bị xóa.
4. Test tự động của API, web và worker đều đạt. Báo cáo phân biệt rõ phần đã chạy thật với phần chỉ test giả.

## 10. Hoàn tác

- Mỗi phần là một nhóm migration và code riêng, revert được.
- Bảng mới không đụng bảng `jobs`, `tracks` hiện có, ngoài việc thêm job type.
- Trên node, mọi file mới nằm trong `models/<tên>/`, `data/datasets/<lora-key>/` và `data/eval/`. Xóa được khi người dùng đồng ý.

## 11. Để builder tự quyết

- Schema bảng cụ thể.
- Tên endpoint.
- Cách poll tiến độ (giữ kiểu poll 2 s như hiện nay).
- Cấu trúc component web.
- Cách gọi tiến trình con cho model.
- Ngưỡng chấm điểm ban đầu.
- Định dạng embedding cache.
