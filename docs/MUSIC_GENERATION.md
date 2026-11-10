# MUSIC_GENERATION

## Ba mức kiểm soát tham số

Không phải tham số nào cũng được model tuân thủ chính xác. Giao diện và metadata phân biệt:

| Mức | Tham số | Ghi chú |
|---|---|---|
| Engine-native | duration, seed, model, LoRA + scale, inference steps, guidance scale | Engine nhận trực tiếp |
| Prompt guidance | genre, subgenre, mood, instruments, và BPM / key / time signature | ACE-Step nhận `bpm`, `key_scale`, `time_signature` làm điều kiện nhưng **không bảo đảm** kết quả đúng tuyệt đối |
| Post-generation analysis | BPM / key đo lại từ audio | Chưa làm (Phase 4) |

## Engine và capability

Mỗi engine là một adapter (`workers/music-worker/src/lush_worker/engines/`) khai báo capability: `generate`, `extend`, `remix`, `inpaint`, `reference_audio`, `lora`… Job chỉ được giao cho node có model tương ứng; tính năng chỉ bật trên web khi capability tồn tại.

| Engine | Trạng thái | Capability đang dùng |
|---|---|---|
| `mock` | Chạy thật trên máy dev | `generate` — WAV tổng hợp theo seed, mono 44.1 kHz. Chỉ để phát triển/kiểm thử, không phải nhạc AI |
| `acestep` | Đã viết, **chưa chạy với GPU thật** | `generate` (text2music), LoRA |

## ACE-Step 1.5

Worker gọi REST server `acestep-api` trên cùng máy (`ACESTEP_URL`, mặc định `http://127.0.0.1:8001`). Các endpoint dùng đều lấy từ adapter đã kiểm chứng của repo cũ: `/health`, `/v1/model_inventory`, `/release_task`, `/query_result`, `/v1/audio`, `/v1/lora/{status,load,scale,toggle}`.

- Mọi job hiện gửi `lyrics: "[Instrumental]"` và không dùng LM (`thinking: false`).
- Số bước mặc định khi không chỉ định: 8 cho model tên chứa `turbo`, 32 cho model khác (giá trị của repo cũ).
- Model worker báo lên là các model ACE-Step **đang nạp**. Model mục tiêu V1: ACE-Step 1.5 Turbo.
- Hủy: ACE-Step không có API hủy đã xác minh → worker chờ task hiện tại xong rồi mới báo `cancelled`.

## Độ dài 1–3 phút

API cho phép 10–240 giây. ACE-Step nhận `audio_duration` 10–600 giây theo adapter cũ, nên 1–3 phút đi theo đường **direct generation**. Chất lượng cấu trúc ở 2–3 phút và VRAM/thời gian trên RTX 3060 12GB **chưa được đo** — đây là mục benchmark đầu tiên khi có Render Node. Extend / continuation / ghép đoạn để dành Phase 5, và không được coi là bảo đảm cấu trúc bài.

## LoRA

Tối đa một LoRA mỗi job. LoRA phải `approved` và cùng base model. Weights nằm ở `LORA_DIR/<key>` trên Render Node; node báo danh sách LoRA nó có và scheduler chỉ giao job cho node có LoRA đó. Tạo nhạc không LoRA luôn được hỗ trợ. Giá trị `lora_scale` hợp lý cần benchmark sau khi có LoRA đầu tiên (Phase 3).

## Tái lập

Mỗi track lưu: prompt gốc, prompt cuối, model, LoRA + scale, seed, BPM/key yêu cầu, duration thật, và tham số engine. Nút "Tạo lại với thiết lập này" dùng đúng các giá trị đó.
