# DEPLOY — Quy trình phát hành

Một lệnh, chạy trên máy dev (Git Bash) ở gốc repo, luôn từ commit đã push:

```bash
bash scripts/deploy.sh --dry-run     # xem sẽ deploy gì
bash scripts/deploy.sh               # deploy (mặc định --target auto)
bash scripts/deploy.sh --target vps|render|all
```

Trong Claude Code: gõ `/deploy` (skill `.claude/skills/deploy/`) — agent commit, push rồi chạy script này.

## Script làm gì

| Bước | Nội dung | Dừng khi |
|---|---|---|
| 1. Preflight | Cây làm việc sạch, đang ở `main`, HEAD = `origin/main` | Có file chưa commit, chưa push |
| 2. So revision | Đọc revision đang chạy: `/opt/lush/current/REVISION` (VPS), `/mnt/ai-studio/AiStudio/lush-worker/REVISION` (render). `auto` chỉ deploy phần có file đổi | Không SSH được |
| 3. Drift | So `lush-api.service`, nginx `lush-studio.conf` trên VPS với repo | Khác nhau → áp dụng tay (VPS dùng chung) |
| 4. Test + build | VPS: `cargo test`, `npm test`, `npm run build`. Render: `pytest` | Test lỗi |
| 5. VPS | Upload `git archive HEAD`, build Rust trong Docker, `/opt/lush/deploy.sh` (sao lưu DB, đổi symlink, health check, **tự rollback**), giữ 5 bản gần nhất | Health check public lỗi |
| 6. Render | `install.sh`, restart `lush-worker` (restart `acestep-api` chỉ khi unit của nó đổi), chờ log `registered as node`; lỗi → **tự cài lại revision trước** | Worker không lên |

Phần nào thuộc VPS: `apps/api`, `apps/web`, `infrastructure/vps`. Phần nào thuộc render: `workers/`, `infrastructure/render-node`. Chỉ đổi `docs/` → không deploy gì.

## Lưu ý

- Migration mới: script in cảnh báo. Rollback sau migration cần khôi phục DB từ `/var/backups/lush` (xem `VPS_DEPLOYMENT.md`).
- Script không sửa: nginx, unit systemd của root, `/etc/lush/api.env`, `ACE-Step-1.5/`, model, `config/*.env` trên render.
- Lần deploy đầu sau khi có script: server chưa có file `REVISION` nên `auto` deploy cả hai.
