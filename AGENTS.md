# AGENTS.md

Hướng dẫn cho AI agent (và người mới) làm việc trong repository này. Giao tiếp và tài liệu bằng tiếng Việt; code, tên hàm, API, config key, lệnh terminal giữ tiếng Anh.

## Dự án

Lush AI Music Studio: nền tảng web tạo nhạc AI nội bộ. Kiến trúc **Control Plane** (VPS, không GPU) + **Compute Plane** (Render Node Ubuntu có GPU). Chi tiết: `docs/ARCHITECTURE.md`. Trạng thái và việc tiếp theo: `docs/ROADMAP.md`. Lý do các quyết định: `docs/DECISIONS.md`.

| Thư mục | Nội dung | Stack |
|---|---|---|
| `apps/api` | Control plane: REST API, job queue, registry | Rust, Axum, SQLx, SQLite WAL |
| `apps/web` | Website | React 19, TypeScript, Vite |
| `workers/music-worker` | Render node worker + engine adapters | Python, httpx, numpy |
| `infrastructure/` | systemd, Caddy, script triển khai | |
| `docs/` | Tài liệu; hai file hợp đồng là nguồn sự thật | |

## Lệnh

```bash
# API (apps/api)
cargo run                       # http://127.0.0.1:8080, đọc .env
cargo test                      # toàn bộ integration test
cargo test --test api cancel    # một test theo tên (lọc chuỗi)
cargo clippy --all-targets && cargo fmt

# Web (apps/web)
npm install && npm run dev      # http://localhost:5173, proxy /api -> 8080
npm test                        # vitest
npx vitest run src/path/file.test.tsx   # một file test
npm run build                   # tsc + vite build

# Worker (workers/music-worker)
py -m venv .venv && .venv\Scripts\python -m pip install -e ".[dev]"
.venv\Scripts\python -m pytest -q
.venv\Scripts\python -m pytest tests/test_runner.py -k cancel
.venv\Scripts\python -m lush_worker    # cần CONTROL_PLANE_URL, NODE_TOKEN, WORKER_ENGINES=mock
```

Chạy cả ba ở chế độ mock trên Windows: `scripts\dev.ps1` (xem README).

Lưu ý Windows: `cargo test`/`cargo build` thất bại với "Access is denied" nếu `lush-api.exe` đang chạy — dừng server trước.

## Quy tắc kiến trúc

- **Hai hợp đồng là nguồn sự thật**: `docs/API_CONTRACTS.md` (web ↔ API) và `docs/WORKER_PROTOCOL.md` (worker ↔ API). Đổi hành vi ở một phía thì sửa hợp đồng và phía còn lại trong cùng thay đổi.
- Worker **không bao giờ** đụng database; mọi thứ đi qua worker protocol. Worker luôn chủ động gọi VPS; VPS không gọi vào Render Node.
- Mọi chuyển trạng thái job nằm ở `apps/api/src/queue.rs` và `routes/worker.rs`, mỗi chuyển trạng thái là **một câu SQL nguyên tử** có điều kiện `status`/`lease_token`. Không đọc-rồi-ghi trạng thái job.
- Genre, preset, model là **dữ liệu**. Không hardcode danh sách thể loại/model ở backend, worker hay web.
- API công khai chỉ dùng Artifact ID; không lộ đường dẫn file. Storage key chỉ ghép từ ID phía server.
- Engine mới = một adapter trong `workers/music-worker/src/lush_worker/engines/`, khai báo `capabilities`. Không giả định engine nào cũng hỗ trợ mọi capability.
- SQL viết bằng `sqlx::query`/`query_as` runtime (không macro kiểm tra compile-time) để build không cần database. Schema thay đổi = file migration mới trong `apps/api/migrations/`, không sửa migration cũ.
- V1 giữ đơn giản: modular monolith, không Redis, không Kubernetes, một job GPU mỗi node.

## Không được làm khi chưa có phê duyệt

Deploy lên VPS/Render Node thật, đổi firewall/driver, xóa dữ liệu, push lên remote, tải hoặc train model lớn. Không commit secrets, database, model/LoRA weights, dataset, nhạc đã tạo. Không sửa repo nghiên cứu `SA1NG/ai_music_studio`.

## Trung thực về kiểm thử

Phân biệt rõ "đã test với mock" và "đã chạy thật". Adapter ACE-Step hiện **chỉ** được unit test bằng HTTP giả; chưa chạy với GPU thật. Không báo hoàn thành chức năng chưa có kết quả kiểm thử thực tế.
