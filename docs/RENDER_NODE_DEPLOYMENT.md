# RENDER_NODE_DEPLOYMENT — Compute Plane

Node đầu tiên: **`render-node-01`**, truy cập từ máy dev bằng `ssh render` (user `ubuntu`, hostname `root-ai2`).
Ubuntu 26.04 LTS, RTX 3060 12 GB (driver 595), 20 CPU, 30 GB RAM. Máy chỉ dành cho dự án studio (Stable Diffusion Forge đã gỡ ngày 2026-11-10 theo yêu cầu; ảnh cũ lưu ở `~/archive/`).

Hai tiến trình, đều là **systemd user unit** (không cần root; linger đã bật nên tự chạy khi máy khởi động, không cần đăng nhập):

| Unit | Vai trò | Nghe |
|---|---|---|
| `acestep-api.service` | REST server ACE-Step 1.5 | `127.0.0.1:8001` (có API key) |
| `lush-worker.service` | Worker của studio: nhận job từ VPS, gọi ACE-Step, upload kết quả | không nghe cổng nào |

Node không mở cổng nào. Worker gọi VPS qua Tailscale: `http://100.113.164.7:8081` (listener riêng trên VPS, chỉ nhận dải `100.64.0.0/10`).

## Lưu trữ: mọi thứ trên NVMe

Quy ước: **mọi file của studio trên máy render nằm dưới `/mnt/ai-studio/AiStudio`**, không đặt trên ổ hệ thống 238 GB.

| Ổ | Vai trò |
|---|---|
| `nvme1n1p1` KIOXIA 1,8 TB, ext4, nhãn `AI-Data` | Mount tại `/mnt/ai-studio` qua `/etc/fstab` (theo UUID, `noatime,nofail`) — dữ liệu studio |
| `nvme0n1` 238 GB | Hệ điều hành |
| `sda` WD 1 TB NTFS | Không dùng cho studio |

```
/mnt/ai-studio/AiStudio/
├── ACE-Step-1.5/          ACE-Step, commit ca1e85f; .venv (Python 3.12, PyTorch 2.10 + CUDA 12.8)
│   └── checkpoints/       model weights (~9,4 GB: DiT turbo, VAE, text encoder, LM 1.7B — LM tải kèm, không nạp)
├── lush-worker/           code + venv của worker (thay mới mỗi lần cài)
├── config/                worker.env, acestep.env (quyền 600; chứa node token và API key)
├── data/work/             file tạm của job
├── data/loras/            LoRA (Phase 3)
├── data/datasets/         dữ liệu train (Phase 3)
├── data/training/         checkpoint khi train (Phase 3)
├── cache/                 cache uv, Python 3.12 do uv quản lý, cache Hugging Face
└── src/                   bản source dùng để cài
```

Ngoài thư mục này chỉ có: unit file trong `~/.config/systemd/user/` và công cụ `uv` ở `~/.local/bin`.

Lựa chọn model: DiT 2B turbo, không nạp LM (`ACESTEP_INIT_LLM=false`) — theo bảng khuyến nghị của ACE-Step cho GPU 8–16 GB; worker không dùng LM (`thinking: false`).

## Cài / cập nhật worker

Cách chuẩn: `bash scripts/deploy.sh` hoặc `/deploy` — xem `DEPLOY.md`. Lệnh tay để tham khảo:

```bash
# Máy dev, ở gốc repo
ssh render 'rm -rf /mnt/ai-studio/AiStudio/src && mkdir -p /mnt/ai-studio/AiStudio/src'
git archive --format=tar HEAD | ssh render 'tar -x -C /mnt/ai-studio/AiStudio/src'
ssh render 'bash /mnt/ai-studio/AiStudio/src/infrastructure/render-node/install.sh /mnt/ai-studio/AiStudio/src && systemctl --user restart lush-worker'
```

`install.sh` (biến `LUSH_ROOT`, mặc định `/mnt/ai-studio/AiStudio`) dừng nếu ổ chưa mount; chỉ thay `lush-worker/` và unit file; không đụng model, LoRA, dataset, `ACE-Step-1.5/`, hay file cấu hình đã có. Restart gửi SIGTERM: worker trả job đang giữ về hàng đợi trước khi thoát.

Cài ACE-Step lần đầu (đã làm): `uv` vào `~/.local/bin`; với `UV_CACHE_DIR` và `UV_PYTHON_INSTALL_DIR` trỏ vào `cache/`: `git clone` + `git checkout ca1e85f` + `uv sync` trong `ACE-Step-1.5/`; rồi `systemctl --user enable --now acestep-api`. Model tự tải ở lần chạy đầu. Nâng cấp ACE-Step là việc riêng, phải kiểm thử lại adapter.

## Vận hành

| Việc | Lệnh (trên node) |
|---|---|
| Log worker | `journalctl --user -u lush-worker -f` |
| Log ACE-Step | `journalctl --user -u acestep-api -f` |
| Khởi động lại | `systemctl --user restart acestep-api lush-worker` |
| GPU | `nvidia-smi` |
| Thêm node mới | Quản trị → Render Nodes → Thêm node, rồi làm lại các bước trên máy mới với token mới |

## Việc cần root

User `ubuntu` phải nhập mật khẩu khi dùng `sudo`, nên agent không tự làm được; người quản trị chạy. Đã làm ngày 2026-11-10: mount NVMe vào `/mnt/ai-studio` (fstab, có bản sao `/etc/fstab.bak-lush-*`), cài `ffmpeg`, `loginctl enable-linger ubuntu`.

## Giới hạn đã biết

- Một job GPU tại một thời điểm mỗi node.
- ACE-Step không có API hủy đã xác minh: hủy job đang sinh phải chờ task hiện tại xong.
- Thống kê GPU chỉ lấy GPU đầu tiên.
