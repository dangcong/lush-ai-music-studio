# TRAINING_SCHEDULER — Lịch train qua đêm trên Training Node

> **2026-11-13 — đã dừng (D28).** Phương án điều khiển training từ website được thay bằng workspace độc lập `lush-training/`. Tài liệu này giữ lại để tham khảo; phần supervisor, Task Scheduler, API, web và NAS trong đó không được triển khai. Nguồn hướng dẫn hiện hành: `lush-training/AGENTS.md`.

Ngày: 2026-11-13. Trạng thái: **[Đề xuất], chưa có code.** Bối cảnh, nhãn và kiến trúc: `BASE_MODEL_TRAINING.md`. Máy training: Windows 11 + WSL2 Ubuntu (D24). Mọi ngưỡng ghi **[Tạm]** sẽ được thay bằng số đo từ `TRAINING_TEST_PLAN.md`.

## 1. Thành phần

| Thành phần | Chạy ở | Vai trò | Sống khi nào |
|---|---|---|---|
| Task Scheduler | Windows | Gọi `wsl.exe` để khởi động supervisor. Không chứa logic | Luôn |
| `lush-train-supervisor` | WSL2 | Quyết định train hay không, chạy và giám sát trainer, ép hạn giờ, viết báo cáo, sao lưu, đồng bộ VPS | Từ 18:00 tới khi xong việc sau 06:00 |
| `lush-trainer` | WSL2 | Tiến trình con duy nhất giữ GPU | Trong session |

Không phần nào phụ thuộc Claude Code hay phiên terminal. Supervisor không cần VPS để chạy: nó dùng `queue.json` đã lưu từ lần đồng bộ gần nhất.

## 2. WSL2 trên máy của người dùng

Ràng buộc: máy dùng bình thường ban ngày; không cài lại Windows; không đổi phân vùng.

| Việc | Thiết kế | Mức kiểm chứng |
|---|---|---|
| Distro | Một distro Ubuntu **riêng** cho training (tên ví dụ `LushTrain`), không dùng chung distro người dùng đang có. Hệ thống chỉ khởi động và tắt đúng distro này | [Đề xuất] |
| Tắt sau session | `wsl --terminate LushTrain` sau khi sao lưu và đồng bộ xong, để trả RAM cho Windows. **Không bao giờ** `wsl --shutdown`: lệnh đó tắt mọi distro và Docker Desktop của người dùng | [Đề xuất] |
| GPU | CUDA trong WSL2 đi qua driver NVIDIA của Windows; không cài driver GPU trong distro. Torch phải là bản có hỗ trợ RTX 5090 | [Tài liệu]; xác nhận ở F1 |
| Số liệu GPU | `nvidia-smi` trong WSL2 thấy VRAM và utilization của cả máy (kể cả ứng dụng Windows), đủ cho preflight. Danh sách tiến trình theo PID có thể không đầy đủ | [Chưa rõ]; đo ở F0 |
| RAM | WSL2 mặc định chỉ cấp cho VM một phần RAM máy. Nếu F2 cho thấy thiếu, cần sửa `%UserProfile%\.wslconfig` — file này áp cho **mọi** distro của người dùng, nên là thay đổi cần đồng ý riêng | [Tài liệu]; giới hạn thật đọc ở F0 |
| Đĩa | Mọi dữ liệu training trong ext4 của distro (file `.vhdx` trên ổ NTFS sẵn có). Chọn ổ đặt `.vhdx` lúc tạo distro; file chỉ lớn lên, thu gọn là thao tác tay | [Đề xuất] |
| Mạng | Supervisor gọi VPS qua Tailscale. Tailscale chạy trên Windows hay trong distro, và distro có đi được ra tailnet không: thử ở F0 | [Chưa rõ] |
| Đồng hồ | Sau khi Windows sleep rồi thức, đồng hồ WSL2 có thể lệch. Supervisor so giờ với Windows lúc khởi động; lệch quá 60 giây thì không train và báo lỗi | [Đề xuất] |
| Quyền | Bật tính năng WSL2 (nếu chưa có) cần quyền admin và có thể cần khởi động lại máy **một lần** — người dùng tự làm, hệ thống không tự reboot | — |

## 3. Kích hoạt

Hai trigger của Task Scheduler, cùng một lệnh:

```
wsl.exe -d LushTrain --cd /srv/lush-train -- /srv/lush-train/bin/lush-train-supervisor run
```

| Trigger | Mục đích |
|---|---|
| Hằng ngày 18:00 | Bắt đầu đêm train |
| Khi máy khởi động (trễ 2 phút) | Phục hồi sau mất điện / reboot: đang trong khung giờ thì vào luồng bình thường, ngoài khung giờ thì chỉ dọn dẹp (mục 7) rồi thoát |

Cài đặt task [Đề xuất]:
- Chạy bằng tài khoản người dùng sở hữu distro (WSL2 gắn theo tài khoản), **không** quyền cao nhất.
- Mong muốn "Run whether user is logged on or not" để resume được sau mất điện khi chưa ai đăng nhập. WSL2 có chạy ổn ở chế độ này trên máy này hay không: **[Chưa rõ], thử ở F0**. Không được thì dùng "chỉ khi đã đăng nhập" và ghi rõ giới hạn đó.
- "Do not start a new instance" khi đang chạy. Supervisor còn tự giữ khóa `supervisor.lock` để hai trigger không chạy chồng.
- Tắt "Stop the task if it runs longer than…": hạn giờ do supervisor tự ép.
- **Không** bật "Wake the computer to run this task", không đổi cấu hình nguồn/sleep, không đổi Windows Update. Máy đang sleep hoặc tắt lúc 18:00 thì đêm đó không train; báo cáo đêm sau ghi lại. Máy sleep **giữa** session: xử lý như mất điện.
- Múi giờ: supervisor tính bằng `Asia/Ho_Chi_Minh` (không có giờ mùa hè), không phụ thuộc múi giờ của Windows hay của distro.

Tạo distro, tạo và sửa task là thay đổi trên máy người dùng → làm bằng script trong `infrastructure/train-node/`, người dùng chạy hoặc đồng ý trước.

## 4. Preflight — xác nhận máy rảnh

Chạy lúc bắt đầu và lặp lại khi chưa đạt. Nguyên tắc: **chỉ quan sát, không can thiệp.** Không đóng ứng dụng, không kết thúc tiến trình nào ngoài trainer do chính supervisor sinh ra.

| Kiểm tra | Điều kiện đạt | Giá trị |
|---|---|---|
| Trong khung giờ | 18:00 ≤ bây giờ < hạn bắt đầu muộn nhất | mục 5 |
| Không có trainer mồ côi | Không còn tiến trình trainer của session trước (theo `state.json`: PID + thời điểm tạo tiến trình) | — |
| GPU rảnh | VRAM đang dùng của cả máy ≤ ngưỡng **và** GPU utilization ≤ ngưỡng, liên tục trong khoảng quan sát | [Tạm] ≤ 3.000 MiB, ≤ 10 %, 5 phút, lấy mẫu 10 giây |
| RAM | RAM trống trong VM WSL2 ≥ nhu cầu đo ở F2 + dự phòng | [Tạm] 48 GB |
| Đĩa | Trống trong ext4 **và** trên ổ Windows chứa `.vhdx` ≥ 3 × kích thước checkpoint đầy đủ + log | [Tạm] 80 GB |
| Nhiệt độ GPU lúc nghỉ | Dưới ngưỡng | [Tạm] 60 °C |
| Run | Có run `active` hoặc `queued`; `run.json`, dataset (hash), checkpoint gốc (SHA-256), tensor còn khớp | — |
| Checkpoint resume | `LATEST` trỏ tới checkpoint có manifest hợp lệ (nếu run đã từng chạy) | — |

Điều kiện "GPU rảnh" dựa trên VRAM tổng và utilization vì đó là hai số chắc chắn đọc được từ WSL2; ứng dụng Windows nào đang chiếm GPU chỉ được ghi vào báo cáo nếu đọc được tên. Ngưỡng VRAM nền phải đặt theo mức desktop thật của máy này (đo ở F0).

Kết quả:
- **Đạt** → bắt đầu session.
- **Máy bận** → trạng thái `waiting_for_idle`, thử lại mỗi 5 phút [Tạm] tới hạn bắt đầu muộn nhất. Hết hạn → session `machine_busy`, ghi VRAM/utilization đã thấy, không train.
- **Lỗi cần người xem** (dataset không khớp, không còn checkpoint hợp lệ, đĩa đầy) → không thử lại trong đêm, run chuyển `failed` hoặc `paused` kèm lý do, hiện cảnh báo trên web.

Giữa session supervisor **không** tự dừng vì thấy ứng dụng khác dùng GPU (ban đêm GPU thuộc về training). Người dùng muốn lấy lại máy thì dùng "Dừng ngay" (mục 6). Có nên tự nhường khi phát hiện người dùng thao tác hay không: chờ trả lời Q7.

## 5. Các mốc giờ

```
18:00            05:00              05:30                06:00
  |-- preflight --|                   |                    |
  |      train ...|...................| dừng an toàn       | hạn cứng
                  ^ hạn bắt đầu       ^ stop_at            ^ GPU phải trống
                    muộn nhất [Tạm]     [Tạm]
```

- `stop_at` = 06:00 − `stop_margin`. Công thức sau khi có số đo F4:
  `stop_margin = 2 × (thời gian một optimizer step dài nhất + thời gian ghi và kiểm checkpoint dài nhất + thời gian tiến trình thoát và trả VRAM) + 5 phút`, không nhỏ hơn 15 phút. Trước khi có số đo: **30 phút [Tạm]**.
- Hạn bắt đầu muộn nhất = `stop_at` − thời gian session tối thiểu (30 phút [Tạm]). Sau mốc này không bắt đầu session mới, kể cả resume sau mất điện.
- Trainer nhận `--stop-at` lúc khởi động và tự dừng ở ranh giới step; supervisor không cần gửi gì thêm. Vì vậy supervisor chết giữa chừng cũng không làm lỡ hạn.
- Trainer không bắt đầu ghi checkpoint định kỳ nếu thời gian còn lại trước `stop_at` nhỏ hơn thời gian ghi dài nhất đã đo: bỏ lần đó, chờ checkpoint lúc dừng.

## 6. Dừng

| Tình huống | Hành vi |
|---|---|
| Tới `stop_at` | Trainer kết thúc step đang chạy → ghi checkpoint → kiểm → thoát mã `10` |
| "Dừng ngay" từ web, hoặc lệnh `stop` trên máy (mục 9) | Supervisor tạo file `STOP` trong thư mục run → như trên, mã `11`. Run chuyển `paused` cho tới khi Admin bấm tiếp tục |
| Hủy run | Như "Dừng ngay", run chuyển `cancelled`; checkpoint giữ nguyên, không tự xóa |
| Trainer không thoát trước hạn cứng 06:00 | Supervisor gửi `SIGKILL` tới **process group của trainer do chính nó tạo** (trainer chạy trong session riêng), không chạm tiến trình nào khác. Checkpoint đang ghi dở nằm trong `.partial` nên bị bỏ; resume từ checkpoint trước. Ghi sự cố `hard_deadline` |
| Supervisor chết | Trainer vẫn tự dừng theo `--stop-at`. Lần khởi động sau xử lý theo mục 7 |
| Distro bị tắt từ bên ngoài (`wsl --shutdown` do người dùng hoặc ứng dụng khác) | Như mất điện: mất phần sau checkpoint gần nhất, GPU được trả ngay |

Xác nhận GPU đã trả sau mỗi lần dừng: tiến trình trainer không còn tồn tại **và** VRAM của cả máy trở về gần mức nền đã ghi lúc preflight (sai lệch ≤ 500 MiB [Tạm]), kiểm trong tối đa 60 giây. Không đạt → ghi sự cố `gpu_not_released`; supervisor không kill gì thêm.

Sau khi GPU đã trả: viết báo cáo → sao lưu NAS → đồng bộ VPS → supervisor thoát → task tắt distro training (`wsl --terminate`). Các việc này không dùng GPU.

## 7. Phục hồi

Lúc khởi động, trước mọi việc khác, supervisor đọc `state.json`:

1. Có session ghi là đang chạy nhưng tiến trình không còn (so PID **và** thời điểm tạo tiến trình, vì PID bị dùng lại sau khi distro khởi động lại) → đánh dấu session `interrupted`, ghi thời điểm phát hiện.
2. Tiến trình trainer cũ vẫn còn (supervisor chết, trainer sống) → trong khung giờ thì gắn lại theo dõi; ngoài khung giờ thì tạo `STOP`, chờ, rồi áp hạn cứng như mục 6.
3. Xóa thư mục `*.partial` trong `checkpoints/`.
4. Kiểm checkpoint theo `LATEST`: manifest + SHA-256. Hỏng → lùi về checkpoint đầy đủ liền trước và ghi lại. Không còn cái nào → run `failed` (`no_valid_checkpoint`); **không train lại từ đầu**.
5. Còn trong khung giờ và trước hạn bắt đầu muộn nhất → preflight → session mới, resume.

| Sự cố | Mất tối đa | Tự phục hồi |
|---|---|---|
| Mất điện / reboot / Windows Update / máy sleep giữa đêm | Từ checkpoint gần nhất (`save_every_minutes`) | Cùng đêm nếu máy tự bật lại và task chạy được khi chưa đăng nhập (Q8, F0); nếu không thì 18:00 hôm sau |
| Trainer crash không rõ lý do | Như trên | Thử lại **một lần** trong đêm từ checkpoint gần nhất; crash lần hai → dừng, run `paused` kèm log |
| OOM, loss không hữu hạn | Như trên | **Không.** Run `failed`, chờ người xem. Không tự giảm batch, không tự đổi optimizer, không tự hạ learning rate |
| Mất mạng tới VPS | Không mất gì | Train tiếp; đồng bộ khi có mạng |
| NAS không tới được | Không mất gì trên máy | Thử lại đêm sau |
| Đĩa đầy khi ghi checkpoint | Checkpoint đang ghi | Dừng session, giữ checkpoint cũ, run `paused` |

## 8. Cấu hình

Một file `train-node.toml` trong distro (không chứa bí mật). Giá trị dưới đây là khởi điểm; mục nào [Tạm] phải được thay sau kiểm thử.

```toml
[node]
train_root = "/srv/lush-train"              # ext4 của distro, không dưới /mnt/c
control_plane_url = "http://100.113.164.7:8081"   # qua Tailscale — đường đi từ WSL2 kiểm ở F0
node_token_file = "/srv/lush-train/config/node.token"   # bí mật ở file riêng (quyền 600), không commit
sync_interval_seconds = 60

[window]
timezone = "Asia/Ho_Chi_Minh"
start = "18:00"
hard_end = "06:00"
stop_margin_minutes = 30                    # [Tạm] → công thức mục 5 sau F4
min_session_minutes = 30                    # [Tạm]
days = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"]

[preflight]
recheck_minutes = 5
observe_seconds = 300
max_vram_used_mib = 3000                    # [Tạm] → mức nền F0 + dự phòng
max_gpu_util_percent = 10                   # [Tạm]
min_free_ram_gb = 48                        # [Tạm] → RAM đỉnh F2 + dự phòng
min_free_disk_gb = 80                       # [Tạm] → 3 × checkpoint F4
max_idle_gpu_temp_c = 60                    # [Tạm]
max_clock_skew_seconds = 60

[checkpoint]
save_every_minutes = 30                     # [Tạm] cân giữa thời gian ghi (F4) và phần mất khi mất điện
save_every_steps = 0                        # 0 = chỉ theo thời gian
keep_resumable = 2
milestone_every_steps = 0                   # đặt theo từng run

[recovery]
max_crash_retries_per_night = 1
gpu_release_timeout_seconds = 60
gpu_release_tolerance_mib = 500             # [Tạm]

[backup]
enabled = false                             # bật ở P4
target = ""                                 # share NAS — Q6
credentials_file = ""                       # file quyền 600 trong distro, ngoài repo
allow_after_window = false                  # Q6
```

Cấu hình của từng run (dataset, base model, optimizer, learning rate, `total_steps`, batch, độ dài segment, seed, tần suất validation) nằm trong `run.json`, do Admin tạo, bất biến sau khi run bắt đầu. Khung giờ là cấu hình của node, sửa trên máy; web chỉ hiển thị (đổi khung giờ từ web: xét sau, vì đó là quyền quyết định khi nào máy của người dùng bị chiếm).

## 9. Lệnh trên máy training

Dùng được khi không có web và không có Claude Code. Trong distro:

```
lush-train-supervisor run        # Task Scheduler gọi; chạy tay cũng được
lush-train-supervisor status     # trạng thái, run, step, mốc giờ đêm nay
lush-train-supervisor stop       # dừng an toàn ngay, run chuyển paused
lush-train-supervisor preflight  # chỉ kiểm tra, in kết quả, không train
lush-train-supervisor verify <run_id>   # kiểm mọi checkpoint theo manifest
```

Từ Windows (PowerShell hoặc lối tắt trên desktop cho nút "trả máy cho tôi"): `wsl -d LushTrain -- lush-train-supervisor stop`.

## 10. Trạng thái supervisor

`idle` (ngoài khung giờ) → `preflight` → `waiting_for_idle` ⇄ `preflight` → `training` → `stopping` → `reporting` → `backing_up` → `idle`. Mọi chuyển trạng thái ghi vào `state.json` (ghi nguyên tử) và `logs/supervisor.log`; web hiển thị đúng trạng thái này, quá 3 chu kỳ đồng bộ không có tin thì hiện "Mất cập nhật từ …" và không suy đoán trạng thái.
