---
name: deploy
description: Use when the user says "deploy", "/deploy", "triển khai", "đưa lên server", "cập nhật server" or asks to commit, push and release Lush AI Music Studio to the VPS (studio.darkambient.co) or the render node.
---

# Deploy Lush AI Music Studio

Gọi skill này = người dùng **đã phê duyệt** cho lần này: commit, `git push origin main`, deploy lên VPS và/hoặc Render Node. Không mở rộng phê duyệt sang việc khác (xóa dữ liệu, sửa nginx/systemd của root, nâng cấp ACE-Step, tải model, force push).

Mọi bước server nằm trong `scripts/deploy.sh` (chi tiết: `docs/DEPLOY.md`). Không tự gõ lệnh deploy bằng tay thay cho script.

## Quy trình

Làm việc ở thư mục repo `e:\ToolAI\AiStudio\lush-ai-music-studio`, lệnh chạy bằng Bash (Git Bash).

1. **Xem thay đổi:** `git status --short` và `git diff --stat`. Nếu không có gì để commit và HEAD đã push, nhảy tới bước 5.
2. **Lọc file cấm:** không commit `.env`, `*.db`, model/LoRA (`*.safetensors`, `*.ckpt`, ...), dataset, nhạc đã tạo, file build. Thấy file lạ hoặc file không rõ của ai → hỏi người dùng, không tự commit.
3. **Commit:** `git add` đúng các file liên quan, message tiếng Anh mô tả thay đổi (theo kiểu `git log --oneline`), kết thúc bằng dòng `Co-Authored-By` theo quy định phiên. Thay đổi có ý nghĩa với người dùng thì thêm một dòng vào `docs/CHANGELOG.md` trong cùng commit.
4. **Push:** `git push origin main`.
   - Chưa có remote `origin` → DỪNG, hỏi người dùng URL repo (hoặc đề nghị `gh repo create <tên> --private --source . --push`). Không tự tạo repo.
   - Push bị từ chối (non-fast-forward) → DỪNG, báo người dùng. Không `--force`, không rebase tự ý.
5. **Xem kế hoạch:** `bash scripts/deploy.sh --dry-run`. Báo người dùng ngắn gọn: deploy VPS / Render / không gì cả, có migration mới không.
6. **Deploy:** `bash scripts/deploy.sh` (chạy nền, có thể mất 5–15 phút vì build Rust trong Docker trên VPS). Tùy chọn `--target vps|render|all` chỉ khi người dùng yêu cầu.
7. **Báo cáo** (tiếng Việt, ngắn): commit nào, phần nào đã deploy, kết quả health check / worker `registered as node`. Nói rõ: deploy thành công ≠ tính năng mới đã được kiểm thử thật; nếu chưa thử tính năng trên site thì ghi là chưa thử.

## Khi script báo `DEPLOY FAILED`

Đọc thông báo, dùng superpowers:systematic-debugging, báo người dùng. Script đã tự rollback (VPS qua `deploy.sh`, worker về revision trước).

| Thông báo | Làm gì |
|---|---|
| `working tree is not clean` | Quay lại bước 1–3 |
| `is not what origin/main has` | Push (bước 4) |
| `cargo test` / `npm test` / `pytest failed` | Sửa lỗi, commit, chạy lại. Không dùng `--skip-tests` để vượt qua |
| `lush-api.exe is running locally` | Hỏi người dùng tắt server dev |
| `differs from the repo` (lush-api.service, nginx) | DỪNG. File của root trên VPS dùng chung — đưa người dùng diff và lệnh áp dụng tay |
| `public health check failed`, `rolled back` | Xem log: `ssh lush1-vps journalctl -u lush-api -n 50` |
| `new worker failed` | Xem log: `ssh render journalctl --user -u lush-worker -n 50` |
| `cannot ssh` | Báo người dùng kiểm tra mạng/SSH; không thử đường khác |

## Không được

- Sửa server bằng tay để "cho nhanh" thay vì sửa repo rồi deploy lại.
- Chạy lại script liên tục khi nó lỗi mà chưa hiểu nguyên nhân.
- Báo "đã deploy" khi script chưa in `==> DONE`.
- Đụng tới nginx/Docker/mail của các site khác trên VPS, hay `ACE-Step-1.5/`, model, `config/*.env` trên render node.
