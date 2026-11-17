# VPS_DEPLOYMENT — Control Plane

> **2026-11-16 — đang chuyển server (D29, T-013).** Website chuyển từ VPS cũ (`lush1-vps`, SSH alias hiện là `lush1-ts`, IP `194.233.69.135`) sang server mới **`ssh lushvietnix2`** (Vietnix, Ubuntu 24.04, 2 CPU, 3,8 GB RAM, IP `14.225.220.26`, Tailscale `100.96.4.9`). Dữ liệu bắt đầu trống theo quyết định của người dùng. Phần **"Server mới"** ngay dưới đây là hiện trạng; các mục phía sau mô tả VPS cũ và còn đúng về cách vận hành (đường dẫn, lệnh), chỉ khác tên máy, chứng chỉ và DNS.

## Server mới `lushvietnix2` (2026-11-16)

Server **dùng chung** với `app.lushmedia.net` (dịch vụ `lushlogin`, cổng 3100) và `mail.lushmedia.net` — không đụng tới cấu hình của chúng. Tường lửa (ufw) chỉ nhận 80/443 từ dải IP Cloudflare, nên tên miền **phải bật proxy Cloudflare**; chứng chỉ là Cloudflare Origin của `darkambient.co` (`/etc/ssl/cloudflare/darkambient.co.{pem,key}`, chép từ VPS cũ, hết hạn 2041), không dùng Let's Encrypt.

| Hạng mục | Trạng thái |
|---|---|
| Docker (build Rust, giới hạn 2,6 GB RAM cho container), Tailscale (`lush-web-vietnix`, `100.96.4.9`) | ✅ Đã cài |
| `lush-api` dưới systemd, `127.0.0.1:8018`, bản `023dc7a`, database mới | ✅ Đang chạy; đăng nhập admin và `/api/health` đạt qua nginx + TLS tại chỗ |
| Site nginx `lush-studio.conf` (= `infrastructure/vps/nginx-lush-studio.conf`); sao lưu `/root/nginx-backup-before-lush-studio-*` | ✅ Đã bật |
| Mật khẩu admin ban đầu | `/root/lush-admin-password.txt` trên server (ngẫu nhiên, 20 ký tự) |
| DNS `suno.darkambient.co` → `14.225.220.26`, **Proxied** | ⬜ Người dùng đổi trên Cloudflare. Chưa đổi thì tên miền vẫn về VPS cũ |
| Cổng 8081 cho worker trên giao diện Tailscale (`ufw allow in on tailscale0 to any port 8081 proto tcp`) | ✅ Đã mở 2026-11-16 (người dùng đồng ý). Từ máy dev qua Tailscale: `/api/health` 200, đường khác 403; qua IP công khai không vào được |
| Render Node: tạo node mới trong Quản trị, đặt `NODE_TOKEN` và `CONTROL_PLANE_URL=http://100.96.4.9:8081` trong `worker.env` | ⬜ Chưa làm; ngày 2026-11-16 không SSH được vào máy render (`100.114.251.82` timeout, không thấy trong tailnet) |
| Kiểm tra từ Internet qua Cloudflare, tạo nhạc thật | ⬜ Chưa (chờ DNS và worker) |
| Cron sao lưu hằng ngày | ⬜ Chưa đặt |

VPS cũ còn chạy các site khác (`darkambient.co`, `temp.darkambient.co`, `mail.lushmedia.net`, `ytm.lushmedia.net`, mail server Docker). **Không xóa VPS cũ** chỉ vì Lush Studio đã chuyển đi.

## VPS cũ `lush1-vps` (tham khảo)

Truy cập: `ssh lush1-ts` (Ubuntu 24.04, 4 CPU, 8 GB RAM). VPS **dùng chung** với các website và mail server khác của công ty (nginx, Docker) — không đụng tới cấu hình của chúng.

## Trạng thái (2026-11-10)

| Hạng mục | Trạng thái |
|---|---|
| `lush-api` chạy dưới systemd, `127.0.0.1:8018`, tự khởi động cùng máy | ✅ Đang chạy |
| Database + file nhạc ở `/var/lib/lush` | ✅ |
| Chạy thử qua SSH tunnel: đăng nhập, tạo nhạc (worker mock trên máy Windows), thư viện, phát/tải, node offline | ✅ Đã chạy |
| **https://suno.darkambient.co** — nginx + Let's Encrypt, cookie `Secure`. Tên cũ `studio.darkambient.co` chỉ chuyển hướng 301 | ✅ Công khai (đổi tên 2026-11-11) |
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

- DNS: `suno.darkambient.co` (và tên cũ `studio.darkambient.co`) → `194.233.69.135`, bản ghi Cloudflare **DNS only** (không proxy). Vì trình duyệt nối thẳng tới VPS nên dùng chứng chỉ Let's Encrypt riêng, không dùng chứng chỉ Cloudflare Origin của `darkambient.co` (chỉ Cloudflare tin chứng chỉ đó). Nếu sau này bật proxy Cloudflare, chứng chỉ này vẫn hợp lệ ở chế độ Full (strict).
- Upload Reference/Source Audio đi qua `location = /api/v1/audio-inputs` (`client_max_body_size 60m`); giới hạn thật do API đặt (`MAX_INPUT_AUDIO_MB`, mặc định 50). Thiếu block này nginx chặn mọi file trên 1 MB bằng `413`.
- Cấu hình: `/etc/nginx/sites-available/lush-studio.conf` = `infrastructure/vps/nginx-lush-studio.conf`. Bản sao lưu toàn bộ `/etc/nginx` trước khi thêm site: `/root/nginx-backup-before-lush-studio-*`; bản trước khi đổi tên: `/root/lush-studio.conf.bak-*`.
- Chứng chỉ: `certbot certonly --webroot -w /var/www/letsencrypt -d suno.darkambient.co` (cùng cách với các site khác; tên cũ có chứng chỉ riêng, vẫn tự gia hạn). `certbot.timer` tự gia hạn; hook có sẵn `/etc/letsencrypt/renewal-hooks/deploy/spoticheck-nginx-reload.sh` tải lại nginx sau gia hạn.
- `/etc/lush/api.env`: `COOKIE_SECURE=true`.

Site công khai chặn `/api/worker/*` và giới hạn tần suất đăng nhập.

## Tài khoản

Mật khẩu `admin` đã được người dùng đặt lại ngày 2026-11-11 để thử (ghi thẳng hash vào DB vì ngắn hơn 8 ký tự mà API yêu cầu; DB sao lưu trước ở `/var/backups/lush/lush-20261111-060814.db.gz`). Đây là mật khẩu yếu trên site công khai — đổi lại mật khẩu mạnh sau khi thử xong. `/root/lush-admin-password.txt` không còn đúng.

Ngày 2026-11-16 người dùng yêu cầu đặt lại mật khẩu `admin` lần nữa, cũng bằng cách ghi hash Argon2id vào DB (mật khẩu 4 ký tự, API không nhận). DB sao lưu trước ở `/var/backups/lush/lush-20261116-044013.db.gz`. Các phiên đăng nhập cũ không bị thu hồi. Cảnh báo mật khẩu yếu ở trên vẫn nguyên.

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
