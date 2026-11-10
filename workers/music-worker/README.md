# Lush Music Worker

Worker Python chạy trên Render Node (Compute Plane) của Lush AI Music Studio. Worker luôn **chủ động** gọi
tới Control Plane (VPS) theo giao thức trong [`docs/WORKER_PROTOCOL.md`](../../docs/WORKER_PROTOCOL.md):
đăng ký → heartbeat → nhận job → chạy engine → upload file → báo hoàn tất. Mỗi lúc chỉ chạy một job.

Engine hiện có:

| Engine | Dùng để | Trạng thái |
|---|---|---|
| `mock` | Máy dev không có GPU. Sinh WAV thật (nghe được), xác định theo seed. | Đã chạy thật trên Windows |
| `acestep` | ACE-Step 1.5 qua REST API (`acestep-api`, cổng 8001). | **Chưa chạy thật** — chỉ mới unit test bằng HTTP giả |

## Cài đặt

Yêu cầu Python >= 3.10. Cài vào venv riêng, không cài vào Python hệ thống.

Windows (PowerShell):

```powershell
cd workers\music-worker
py -m venv .venv
.venv\Scripts\python -m pip install -e ".[dev]"
```

Ubuntu:

```bash
cd workers/music-worker
python3 -m venv .venv
.venv/bin/python -m pip install -e ".[dev]"
```

Muốn có file MP3 preview thì cần `ffmpeg` trong PATH (Ubuntu: `sudo apt install ffmpeg`). Không có ffmpeg
worker vẫn chạy, chỉ upload `audio_master` (WAV) và ghi cảnh báo — không bao giờ tạo MP3 giả.

## Chạy ở mock mode (Windows)

1. Admin tạo node trên Control Plane và lấy node token (chỉ hiện một lần).
2. Tạo file `.env` trong `workers\music-worker` (copy từ `.env.example`):

   ```ini
   CONTROL_PLANE_URL=http://localhost:8080
   NODE_TOKEN=<token của node>
   WORKER_ENGINES=mock
   ```

3. Chạy (đứng trong `workers\music-worker`, vì `.env` được đọc từ thư mục hiện tại):

   ```powershell
   .venv\Scripts\python -m lush_worker
   ```

   Hoặc dùng console script: `.venv\Scripts\lush-worker`.

Dừng bằng Ctrl+C: job đang chạy được trả về hàng đợi (`release`). Bấm Ctrl+C lần hai để thoát ngay.

## Chạy với ACE-Step (Ubuntu, có GPU)

1. Bật server ACE-Step 1.5 trên cùng máy: `uv run acestep-api` trong thư mục `ACE-Step-1.5` (cổng 8001).
2. `.env`:

   ```ini
   CONTROL_PLANE_URL=https://<địa chỉ VPS>
   NODE_TOKEN=<token của node>
   WORKER_ENGINES=acestep
   ACESTEP_URL=http://127.0.0.1:8001
   ACESTEP_API_KEY=
   ACESTEP_TIMEOUT=1800
   LORA_DIR=/srv/lush/loras
   ```

3. `.venv/bin/python -m lush_worker`

**LoRA** nằm sẵn trên Render Node, không tải từ VPS. Mỗi LoRA là một thư mục con của `LORA_DIR`, tên thư
mục chính là `key` của LoRA trong Registry:

```
LORA_DIR/
  <key>/adapter_config.json
  <key>/adapter_model.safetensors
```

Worker báo lên VPS các thư mục con có `adapter_config.json`. Model báo lên là các model DiT **đang được
nạp** trong ACE-Step (lấy từ `/v1/model_inventory`).

## Biến môi trường

Đọc từ biến môi trường và từ file `.env` ở thư mục hiện tại; biến môi trường thật được ưu tiên hơn file.

| Biến | Mặc định | Ý nghĩa |
|---|---|---|
| `CONTROL_PLANE_URL` | (bắt buộc) | Địa chỉ gốc của VPS, không kèm `/api/worker/v1` |
| `NODE_TOKEN` | (bắt buộc) | Token của node, gửi qua `Authorization: Bearer` |
| `WORKER_ENGINES` | `mock` | `mock`, `acestep` hoặc `mock,acestep` |
| `ACESTEP_URL` | `http://127.0.0.1:8001` | Địa chỉ server `acestep-api` |
| `ACESTEP_API_KEY` | rỗng | API key của ACE-Step (nếu server bật) |
| `ACESTEP_TIMEOUT` | `1800` | Số giây tối đa chờ một task sinh nhạc |
| `LORA_DIR` | rỗng | Thư mục chứa LoRA trên node |
| `WORK_DIR` | `<thư mục tạm>/lush-worker` | Thư mục tạm cho file của từng job (xóa sau mỗi job) |
| `MOCK_STEP_DELAY` | `0.3` | Số giây nghỉ mỗi bước giả lập của mock engine (8 bước) |
| `LOG_LEVEL` | `INFO` | `DEBUG` / `INFO` / `WARNING` / `ERROR` |

## Hành vi khi có sự cố

| Tình huống | Worker làm gì |
|---|---|
| Không đăng ký được (mất mạng, 5xx, 401/403) | Thử lại với backoff 1s → 30s, log rõ lý do |
| `409 protocol_mismatch` khi đăng ký | Dừng hẳn (mã thoát 3): cần nâng cấp worker |
| `401/403` khi claim | Log lỗi, chờ 30s rồi thử lại |
| Mất kết nối VPS khi claim | Chờ 5s, đăng ký lại rồi tiếp tục |
| `409 lease_lost` | Bỏ job, không upload, không gọi `complete`/`fail` |
| Yêu cầu hủy (`cancel_requested` hoặc `cancel_job_ids`) | Dừng, gọi `fail` với `cancelled: true` |
| Engine tạm không sẵn sàng (ACE-Step tắt, mất kết nối) | `fail` với `retryable: true` |
| Tham số sai, ACE-Step báo task lỗi, task quá `ACESTEP_TIMEOUT` | `fail` với `retryable: false` |
| `422 checksum_mismatch` hoặc lỗi mạng khi upload | Upload lại, tối đa 4 lần |
| Báo progress thất bại | Chỉ log cảnh báo, job vẫn chạy tiếp |
| Ctrl+C / SIGTERM | Gọi `release` cho job đang chạy rồi thoát |
| Engine không liệt kê được model lúc đăng ký | Báo danh sách model rỗng cho engine đó; tự đăng ký lại mỗi 60 giây |

## Chạy test

```powershell
.venv\Scripts\python -m pytest -q
```

Test dùng `respx` giả lập cả Control Plane lẫn ACE-Step, không cần GPU hay mạng.

## Giới hạn đã biết

- **Engine `acestep` chưa được chạy với ACE-Step thật.** Logic gọi API được port từ adapter của repo nghiên
  cứu (đã đối chiếu mã nguồn ACE-Step ở commit `ca1e85f`), nhưng trong repo này mới chỉ được kiểm bằng HTTP
  giả. Cần chạy thử trên máy có GPU trước khi dùng thật.
- **ACE-Step không có API hủy task đã được xác minh.** Khi job bị hủy (hoặc mất lease) lúc task đang chạy,
  worker tiếp tục chờ task kết thúc, bỏ kết quả, rồi mới báo `cancelled`. Làm vậy để worker không nhận job
  mới trong khi GPU còn bận task mồ côi, và không đổi trạng thái LoRA dùng chung khi task còn chạy. Hệ quả:
  job hiện "đang hủy" cho tới khi task hiện tại xong (tối đa `ACESTEP_TIMEOUT`). Nếu hủy trước khi gửi task
  thì dừng ngay. Khi tắt worker (Ctrl+C/SIGTERM) thì không chờ: job được `release` ngay, task trong ACE-Step
  vẫn chạy nốt.
- Task quá `ACESTEP_TIMEOUT` được báo lỗi không retry, vì task có thể vẫn đang chạy trong ACE-Step.
- Không dùng `/v1/lora/unload` (hỏng ở ACE-Step commit `ca1e85f`): job không dùng LoRA thì tắt bằng
  `/v1/lora/toggle`. LoRA đã nạp vẫn chiếm VRAM cho tới khi khởi động lại ACE-Step.
- Payload `generate` của giao thức v1 không có lời bài hát, nên mọi job ACE-Step được gửi dạng instrumental
  (`lyrics: "[Instrumental]"`), không dùng LM.
- Chỉ hỗ trợ job `generate` (text2music). Chưa có extend/remix/inpaint/reference audio, training, autolabel.
- Mock engine bỏ qua LoRA và xuất WAV mono 16-bit 44.1 kHz.
- Việc tạo MP3 bằng ffmpeg thật chưa được chạy thử trên máy dev (máy không có ffmpeg); đường đi "không có
  ffmpeg" thì đã chạy thật.
- Thống kê GPU chỉ lấy GPU đầu tiên từ `nvidia-smi`.
