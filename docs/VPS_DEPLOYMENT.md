# VPS_DEPLOYMENT — Control Plane

Truy cập: `ssh lush1-vps` (Ubuntu 24.04, 4 CPU, 8 GB RAM). VPS **dùng chung** với các website và mail server khác của công ty (nginx, Docker) — không đụng tới cấu hình của chúng.

## Trạng thái (2026-11-10)

| Hạng mục | Trạng thái |
|---|---|
| `lush-api` chạy dưới systemd, `127.0.0.1:8018`, tự khởi động cùng máy | ✅ Đang chạy |
| Database + file nhạc ở `/var/lib/lush` | ✅ |
| Chạy thử qua SSH tunnel: đăng nhập, tạo nhạc (worker mock trên máy Windows), thư viện, phát/tải, node offline | ✅ Đã chạy |
| **https://studio.darkambient.co** — nginx + Let's Encrypt, cookie `Secure` | ✅ Công khai (2026-11-10) |
| Tailscale (kênh riêng cho Render Node) | ⬜ Chưa cài |
| Cron sao lưu hằng ngày | ⬜ Chưa đặt |

## Bố cục trên VPS

| Đường dẫn | Nội dung |
|---|---|
| `/opt/lush/releases/<timestamp>/` | Mỗi bản phát hành: `lush-api` + `web/` |
| `/opt/lush/current` | Symlink tới bản đang chạy |
| `/opt/lush/deploy.sh`, `backup.sh` | Script triển khai và sao lưu |
| `/etc/lush/api.env` | Cấu hình (root, 600) |
| `/var/lib/lush/` | `lush.db`, `artifacts/` — deploy không bao giờ đụng tới |
| `/var/backups/lush/` | Bản sao lưu database |
| `/opt/lush-src/` | Bản source dùng để build (có thể xóa) |
| `/root/lush-admin-password.txt` | Mật khẩu admin ban đầu (xóa sau khi đổi mật khẩu) |

`lush-api` tự phục vụ luôn trang web React (`WEB_DIST=/opt/lush/current/web`), nên nginx chỉ cần proxy mọi thứ tới `127.0.0.1:8018`.

## Phát hành bản mới

Cách chuẩn: `bash scripts/deploy.sh` hoặc `/deploy` — xem `DEPLOY.md`. Các lệnh dưới đây là những gì script làm, để tham khảo hoặc xử lý sự cố.

Build Rust trong container tạm (không cài toolchain lên VPS dùng chung); build web trên máy dev.

```bash
# Máy dev (Git Bash), ở thư mục gốc repo
git archive --format=tar HEAD | ssh lush1-vps 'rm -rf /opt/lush-src && mkdir -p /opt/lush-src && tar -x -C /opt/lush-src'
(cd apps/web && npm run build && tar -czf ../../web-dist.tgz --transform 's,^dist,web,' dist)
scp web-dist.tgz lush1-vps:/opt/lush-src/

# Trên VPS
ssh lush1-vps
cd /opt/lush-src/apps/api
docker run --rm -v "$PWD":/src -v lush-cargo-registry:/usr/local/cargo/registry \
  -v lush-cargo-target:/src/target -w /src rust:1-bookworm \
  sh -c 'cargo build --release --locked && cp target/release/lush-api /src/lush-api.bin'
mkdir -p /tmp/rel && cp lush-api.bin /tmp/rel/lush-api && tar -xzf ../../web-dist.tgz -C /tmp/rel
tar -czf /opt/lush-src/release.tar.gz -C /tmp/rel . && rm -rf /tmp/rel
/opt/lush/deploy.sh /opt/lush-src/release.tar.gz
```

`deploy.sh` sao lưu database, giải nén bản mới (chủ sở hữu root, quyền đọc cho mọi user), đổi symlink, khởi động lại, kiểm tra `/api/health`, và **tự rollback** nếu bản mới không lên. Migrations chạy khi `lush-api` khởi động.

Lưu ý: `lush-api` không có tham số dòng lệnh — chạy file binary là khởi động server. Đừng chạy `lush-api --help` để "kiểm tra".

## Tên miền (đã bật)

- DNS: `studio.darkambient.co` → `194.233.69.135`, bản ghi Cloudflare **DNS only** (không proxy). Vì trình duyệt nối thẳng tới VPS nên dùng chứng chỉ Let's Encrypt riêng, không dùng chứng chỉ Cloudflare Origin của `darkambient.co` (chỉ Cloudflare tin chứng chỉ đó). Nếu sau này bật proxy Cloudflare, chứng chỉ này vẫn hợp lệ ở chế độ Full (strict).
- Cấu hình: `/etc/nginx/sites-available/lush-studio.conf` = `infrastructure/vps/nginx-lush-studio.conf`. Bản sao lưu toàn bộ `/etc/nginx` trước khi thêm site: `/root/nginx-backup-before-lush-studio-*`.
- Chứng chỉ: `certbot certonly --webroot -w /var/www/letsencrypt -d studio.darkambient.co` (cùng cách với các site khác). `certbot.timer` tự gia hạn; hook có sẵn `/etc/letsencrypt/renewal-hooks/deploy/spoticheck-nginx-reload.sh` tải lại nginx sau gia hạn.
- `/etc/lush/api.env`: `COOKIE_SECURE=true`.

Site công khai chặn `/api/worker/*` và giới hạn tần suất đăng nhập.

## Vận hành

| Việc | Lệnh (trên VPS) |
|---|---|
| Log | `journalctl -u lush-api -f` |
| Health | `curl -s http://127.0.0.1:8018/api/health` |
| Sao lưu DB | `/opt/lush/backup.sh` — nên đặt cron hằng ngày |
| Sao lưu file nhạc | rsync/restic thư mục `/var/lib/lush/artifacts` |
| Rollback | `ln -sfn /opt/lush/releases/<bản cũ> /opt/lush/current && systemctl restart lush-api` |
| Truy cập thẳng API, bỏ qua nginx | Máy dev: `ssh -N -L 18018:127.0.0.1:8018 lush1-vps`, mở http://localhost:18018 |

Rollback bản có migration mới: khôi phục database từ bản sao lưu trước khi deploy (migration không tự đảo ngược).

## Thêm Render Node

Quản trị → Render Nodes → Thêm node → sao chép token (chỉ hiện một lần) → cấu hình worker theo `RENDER_NODE_DEPLOYMENT.md`.

## Dữ liệu thử đang có trên VPS

Từ lần chạy thử 2026-11-10: tài khoản `qa-test` (đã khóa), node `qa-windows-mock` (đã vô hiệu hóa), một bài nhạc mock (đã xóa mềm), hai job thử (một hoàn tất, một đã hủy). Có thể để nguyên; chúng không hiện với nhân viên.
