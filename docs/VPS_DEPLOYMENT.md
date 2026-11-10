# VPS_DEPLOYMENT — Control Plane

> **Trạng thái:** các file trong `infrastructure/vps/` đã được soạn nhưng **chưa chạy trên VPS thật**. Lần triển khai đầu cần làm từng bước có người theo dõi và cần phê duyệt trước khi thực hiện.

VPS chạy: Caddy (HTTPS) → `lush-api` (Rust, một file thực thi) + SQLite + thư mục artifacts. Không GPU, không ACE-Step, không model.

## Yêu cầu

- Linux x86_64 (Ubuntu 24.04 khuyến nghị), tên miền trỏ về VPS, cổng 80/443 mở.
- Gói: `caddy`, `sqlite3`, `curl`, `tailscale`.
- Dung lượng: mỗi bài hoàn thành lưu WAV + MP3 (ước tính ~35 MB cho 3 phút WAV 48 kHz stereo).

## Bố cục

| Đường dẫn | Nội dung |
|---|---|
| `/opt/lush/releases/<timestamp>/` | Mỗi bản phát hành: `lush-api`, `web/` |
| `/opt/lush/current` | Symlink tới bản đang chạy |
| `/etc/lush/api.env` | Cấu hình (quyền 600), theo `apps/api/.env.example` |
| `/var/lib/lush/` | `lush.db`, `artifacts/` — **không bao giờ bị deploy đụng tới** |
| `/var/backups/lush/` | Bản sao lưu database |

## Cài lần đầu

```bash
useradd --system --home /var/lib/lush --shell /usr/sbin/nologin lush
mkdir -p /opt/lush/releases /etc/lush /var/lib/lush /var/backups/lush
chown -R lush:lush /var/lib/lush
install -m 600 apps/api/.env.example /etc/lush/api.env      # rồi chỉnh bên dưới
install -m 755 infrastructure/vps/deploy.sh infrastructure/vps/backup.sh /opt/lush/
install -m 644 infrastructure/vps/lush-api.service /etc/systemd/system/
install -m 644 infrastructure/vps/Caddyfile /etc/caddy/Caddyfile   # sửa tên miền + IP Tailscale
systemctl daemon-reload && systemctl enable lush-api
```

`/etc/lush/api.env` cho production:

```
BIND_ADDR=127.0.0.1:8080
DATA_DIR=/var/lib/lush
COOKIE_SECURE=true
BOOTSTRAP_ADMIN_USERNAME=admin
BOOTSTRAP_ADMIN_PASSWORD=<mật khẩu mạnh, xóa dòng này sau lần chạy đầu>
```

Mạng riêng: `tailscale up` trên VPS; lấy IP bằng `tailscale ip -4` và điền vào `Caddyfile` (listener riêng cổng 8081 chỉ phục vụ `/api/worker/*`). Website công khai chặn `/api/worker/*`.

## Build và phát hành

Build trên máy Linux x86_64 (CI hoặc chính VPS), không build trên Windows cho production:

```bash
(cd apps/api && cargo build --release)
(cd apps/web && npm ci && npm run build)
mkdir -p release && cp apps/api/target/release/lush-api release/ && cp -r apps/web/dist release/web
tar -czf lush-web-release.tar.gz -C release .
```

Trên VPS: `sudo /opt/lush/deploy.sh lush-web-release.tar.gz`. Script sao lưu database, giải nén bản mới, đổi symlink, khởi động lại, kiểm tra `/api/health`, và **tự rollback** nếu không lên. Migrations chạy khi `lush-api` khởi động.

## Vận hành

| Việc | Lệnh |
|---|---|
| Log | `journalctl -u lush-api -f` |
| Health | `curl -s http://127.0.0.1:8080/api/health` |
| Sao lưu DB thủ công | `/opt/lush/backup.sh` (nên đặt cron hằng ngày) |
| Sao lưu file nhạc | rsync/restic thư mục `/var/lib/lush/artifacts` |
| Rollback | `ln -sfn /opt/lush/releases/<bản cũ> /opt/lush/current && systemctl restart lush-api` |

Rollback bản có migration mới: khôi phục database từ bản sao lưu trước khi deploy (migration không tự đảo ngược).

## Thêm Render Node

Quản trị → Render Nodes → Thêm node → sao chép token (chỉ hiện một lần) → cấu hình worker theo `RENDER_NODE_DEPLOYMENT.md`.

## Việc còn lại trước khi coi là production

Rate limit cho `/api/v1/auth/login` ở Caddy; lịch cron cho `backup.sh`; sao lưu ngoài máy; giám sát dung lượng đĩa.
