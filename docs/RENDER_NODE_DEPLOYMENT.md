# RENDER_NODE_DEPLOYMENT — Compute Plane

> **Trạng thái:** `infrastructure/render-node/` đã được soạn nhưng **chưa chạy trên máy Ubuntu thật**; adapter ACE-Step chưa chạy với GPU thật. Lần đầu cần làm có người theo dõi và cần phê duyệt.

Node đầu tiên: `render-node-01`, Ubuntu Server 24.04 LTS, RTX 3060 12GB.

Render Node chạy hai tiến trình độc lập:
1. **`acestep-api`** — server REST của ACE-Step 1.5, chỉ nghe `127.0.0.1:8001`, có môi trường Python riêng.
2. **`lush-worker`** — worker của dự án này, gọi `acestep-api` qua localhost và gọi VPS qua Tailscale.

Node không mở cổng nào ra ngoài. Website frontend không cài ở đây.

## Điều kiện có sẵn (script của dự án không tự làm)

- Driver NVIDIA + CUDA hoạt động (`nvidia-smi` chạy được). **Không** tự động cài/đổi driver.
- ACE-Step 1.5 cài theo hướng dẫn chính thức (https://github.com/ace-step/ACE-Step-1.5), đã tải model (ưu tiên Turbo), chạy được `uv run acestep-api`. Nên chạy dưới systemd riêng, bind `127.0.0.1`.
- `ffmpeg` (để tạo MP3 preview; thiếu thì chỉ có WAV), `python3-venv`, `rsync`, `tailscale`.
- `tailscale up` và ping được IP Tailscale của VPS.

## Cài worker

```bash
git clone <repo> /opt/src/lush && cd /opt/src/lush      # hoặc git pull để cập nhật
sudo infrastructure/render-node/install.sh /opt/src/lush
sudo nano /etc/lush/worker.env
sudo systemctl restart lush-worker && journalctl -u lush-worker -f
```

`/etc/lush/worker.env`:

```
CONTROL_PLANE_URL=http://<IP Tailscale của VPS>:8081
NODE_TOKEN=<token tạo ở Quản trị → Render Nodes>
WORKER_ENGINES=acestep
ACESTEP_URL=http://127.0.0.1:8001
LORA_DIR=/var/lib/lush-worker/loras
WORK_DIR=/var/lib/lush-worker/work
```

Danh sách đầy đủ biến môi trường: `workers/music-worker/README.md`.

## Kiểm tra sau khi cài

1. Log worker có dòng đăng ký thành công; trang Render Nodes hiện node **online** kèm GPU và model ACE-Step đang nạp.
2. Tạo một bài 30 giây từ website → job chạy → nghe được.
3. Tắt worker giữa job → job về hàng đợi; bật lại → job chạy lại.

## Cập nhật

`git pull` rồi chạy lại `install.sh`. Chỉ code worker và venv được thay; **model, LoRA, dataset, checkpoint, `/etc/lush/worker.env` không bị đụng tới** và model không bị tải lại. `systemctl restart lush-worker` gửi SIGTERM: worker trả job đang giữ về hàng đợi trước khi thoát.

Phiên bản: worker gửi `protocol_version`; VPS từ chối worker lệch protocol (`409 protocol_mismatch`). Khi protocol tăng phiên bản, deploy VPS trước rồi tới worker.

## Thêm node thứ hai

Lặp lại các bước trên với token mới. Node tự báo GPU, engine, model, LoRA của nó; scheduler chỉ giao job phù hợp. Không cần đổi code hay cấu hình VPS.

## Giới hạn đã biết

- Một job GPU tại một thời điểm mỗi node.
- ACE-Step không có API hủy đã xác minh: hủy job đang sinh phải chờ task hiện tại xong.
- Thống kê GPU chỉ lấy GPU đầu tiên.
