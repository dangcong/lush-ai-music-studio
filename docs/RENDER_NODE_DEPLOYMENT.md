# RENDER_NODE_DEPLOYMENT — Compute Plane

## Node hiện tại: `rtx6000` (từ 2026-11-23)

Truy cập `ssh rtx6000` (user `ai-rtx6000`), Ubuntu 24.04.5, RTX PRO 6000 Blackwell 96 GB (driver 595.99, sm_120), 32 core, 123 GB RAM, Tailscale `100.121.91.61`. Mọi file của studio nằm dưới **`/media/ai-rtx6000/AI_DATA/AiStudio`** (NVMe 3,6 TB, ext4 nhãn `AI_DATA`, trong `/etc/fstab` theo UUID với `nofail`). Bố cục thư mục, hai systemd user unit và cách cài giống node cũ bên dưới, chỉ thay `LUSH_ROOT`. `scripts/deploy.sh` mặc định `RENDER_HOST=rtx6000` và `LUSH_ROOT` này.

- Worker gọi website mới `lushvietnix2` qua Tailscale: `CONTROL_PLANE_URL=http://100.96.4.9:8081`. Node trong Quản trị tên `rtx6000`.
- `config/acestep.env`: `ACESTEP_API_KEY`, `ACESTEP_CONFIG_PATH=acestep-v15-xl-sft`, `ACESTEP_INIT_LLM=true`, `ACESTEP_LM_MODEL_PATH=acestep-5Hz-lm-4B` (D36; tắt LM thì `ACESTEP_INIT_LLM=false`, công tắc LM tự ẩn), `ACESTEP_CHECKPOINTS_DIR=<LUSH_ROOT>/ACE-Step-1.5/checkpoints`, **`ACESTEP_NO_INIT=false`** (bắt buộc: không có dòng này ACE-Step `ca1e85f` chỉ nạp model khi có request, worker gọi `/v1/lora/status` trước và job lỗi `Model not initialized`).
- ACE-Step `ca1e85f`, PyTorch 2.10 + CUDA 12.8 nhận GPU; checkpoints 9,4 GB; model nạp xong chiếm ~8,4 GB VRAM.
- Root đã làm (người dùng, 2026-11-23): tạo thư mục, fstab, `loginctl enable-linger ai-rtx6000`, cài `git` + `ffmpeg`.
- Đã xác minh 2026-11-23: worker đăng ký với website; một bài Text to Music 60 s (Lo-fi) tạo trên production qua API, xong trong ~15 s, có track `audio_master` + `audio_preview`. Chưa xác minh: nghe chất lượng, khởi động lại máy, Train Style trên node này.

## Node cũ: `render-node-01` (không còn dùng, ngoại tuyến từ 2026-11-16)

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
│   └── checkpoints/       model weights (D36: DiT `acestep-v15-xl-sft` ~20 GB, LM `acestep-5Hz-lm-4B` ~8,4 GB, VAE, Qwen3-Embedding-0.6B)
├── lush-worker/           code + venv của worker (thay mới mỗi lần cài)
├── config/                worker.env, acestep.env (quyền 600; chứa node token và API key)
├── data/work/             file tạm của job
├── data/loras/<key>/      LoRA đã train (adapter_config.json + adapter_model.safetensors)
├── data/datasets/<tên>/<version>/   dataset version: dataset.json + audio/
├── data/import/           nơi chép dữ liệu thô lên node (chưa có công cụ import)
├── data/tensors/          tensor đã preprocess, theo dataset version + base model
├── data/training/<job>/   mỗi training run: out/ (checkpoint), train.log, run.json
├── cache/                 cache uv, Python 3.12 do uv quản lý, cache Hugging Face
└── src/                   bản source dùng để cài
```

Ngoài thư mục này chỉ có: unit file trong `~/.config/systemd/user/` và công cụ `uv` ở `~/.local/bin`.

Lựa chọn model (D36): DiT 4B XL SFT cho cả tạo nhạc và train LoRA; LM 4B nạp sẵn cho "Thinking (LM)" và captioner `acestep-lm`. Mục cũ (DiT 2B turbo cho GPU 12 GB của `render-node-01`) là lịch sử.

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

## Cài model mô tả

Model mô tả (captioner) cho "Danh mục bài hát" là script riêng, khai báo trong `CAPTIONER_DIR` (xem `WORKER_PROTOCOL.md`, "Captioner subprocess"). Script MOSS-Music 8B nằm ở `workers/music-worker/captioners/moss_music/`; hướng dẫn tạo venv, tải model và tự kiểm ở `README.md` trong thư mục đó. 🟡 Chưa chạy với GPU thật; bước tải model (~16 GB) chỉ làm sau khi người dùng duyệt.

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
