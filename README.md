# Lush AI Music Studio

Nền tảng web tạo nhạc AI nội bộ của Lush Media. Kiến trúc **Control Plane** (VPS: website + API + hàng đợi) và **Compute Plane** (Render Node Ubuntu có GPU: ACE-Step 1.5).

- Tổng quan và sơ đồ: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- Trạng thái và việc tiếp theo: [docs/ROADMAP.md](docs/ROADMAP.md)
- Mức độ đã kiểm thử: [docs/TESTING.md](docs/TESTING.md)

## Cấu trúc

```
apps/api               Rust (Axum, SQLx, SQLite) — control plane
apps/web               React 19 + TypeScript + Vite — website
workers/music-worker   Python — render node worker, engine adapters (mock, acestep)
infrastructure/        systemd, nginx, script triển khai VPS và Render Node
docs/                  Tài liệu; API_CONTRACTS.md và WORKER_PROTOCOL.md là nguồn sự thật
scripts/dev.ps1        Chạy cả hệ thống ở chế độ mock trên Windows
```

## Chạy trên Windows (không cần GPU)

Cần: Rust stable (kèm MSVC Build Tools), Node.js 24+, Python 3.10+.

```powershell
powershell -ExecutionPolicy Bypass -File scripts\dev.ps1
```

Script mở ba cửa sổ (API, worker mock, web), tự tạo tài khoản `admin` / `admin-dev-password` và node `dev-node-01`. Mở http://localhost:5173, đăng nhập, vào **Tạo nhạc**, chọn model *Mock Engine v1* và bấm Generate. Mock Engine sinh file WAV tổng hợp (không phải nhạc AI) để kiểm tra toàn bộ luồng.

Chạy tay từng phần:

```powershell
# 1) API — http://127.0.0.1:8080
cd apps\api
copy .env.example .env        # đặt BOOTSTRAP_ADMIN_PASSWORD
cargo run

# 2) Web — http://localhost:5173 (proxy /api tới 8080)
cd apps\web
npm install
npm run dev

# 3) Worker — tạo node ở Quản trị → Render Nodes để lấy token
cd workers\music-worker
py -m venv .venv
.venv\Scripts\python -m pip install -e ".[dev]"
copy .env.example .env        # CONTROL_PLANE_URL, NODE_TOKEN, WORKER_ENGINES=mock
.venv\Scripts\python -m lush_worker
```

## Test

```powershell
cd apps\api;              cargo test
cd apps\web;              npm test
cd workers\music-worker;  .venv\Scripts\python -m pytest -q
```

## Triển khai

- VPS: [docs/VPS_DEPLOYMENT.md](docs/VPS_DEPLOYMENT.md)
- Render Node: [docs/RENDER_NODE_DEPLOYMENT.md](docs/RENDER_NODE_DEPLOYMENT.md)

Web release và worker release tách riêng. Website: **https://suno.darkambient.co** (VPS `ssh lush1-vps`). Phát hành: `/deploy` hoặc `bash scripts/deploy.sh` — [docs/DEPLOY.md](docs/DEPLOY.md).

## Không commit

Secrets, `.env`, database, model/LoRA weights, dataset, nhạc đã tạo — đã có trong `.gitignore`.
