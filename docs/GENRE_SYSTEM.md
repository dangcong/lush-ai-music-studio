# GENRE_SYSTEM

Thể loại là **dữ liệu trong database**, quản lý qua trang Quản trị → Genres. Thêm thể loại mới không cần sửa code, không cần deploy.

## Mô hình

- **Genre** (`genres`, `parent_id = NULL`): name, slug, description, `prompt_template`, gợi ý `instruments` / `moods`, `bpm_min`–`bpm_max`, `default_duration`, `is_active`, `sort_order`.
- **Subgenre**: genre có `parent_id`. Chỉ hai cấp.
- **Style Preset** (`style_presets`): cấu hình tạo nhạc tái sử dụng, thuộc một genre.
  - `kind = genre`: prompt template, instruments, moods, bpm, duration, model, params. Không cần LoRA.
  - `kind = trained`: bắt buộc `lora_id` (+ `lora_scale`). Nhiều trained preset có thể thuộc cùng một genre (vd. *Lush Warm Lo-fi*, *Lush Rainy Lo-fi*).
  - `version` để ghi nhận thay đổi preset.

Dữ liệu ban đầu (migration `0002_seed_genres.sql`): Lo-fi, Jazz, Dark Ambient, Dark Academia và 19 subgenre. Đó là lần duy nhất genre được thêm bằng migration.

## Thứ tự ưu tiên khi tạo job

1. Giá trị trong request của người dùng.
2. Giá trị của preset (model, LoRA, scale, bpm, instruments, moods, params).
3. `prompt_template`: preset → subgenre → genre (lấy cái đầu tiên không rỗng).

Prompt cuối = template (chèn prompt người dùng vào `{prompt}` nếu template có chỗ đó, không thì đặt prompt người dùng lên trước) + instruments + moods + `instrumental, no vocals`. Logic: `compose_prompt` trong `apps/api/src/routes/jobs.rs`.

Gợi ý `instruments`/`moods`/BPM của genre chỉ để giao diện hiển thị lựa chọn; chúng **không** tự động được thêm vào prompt nếu người dùng không chọn.

## Tắt genre

`is_active = false` ẩn genre khỏi form tạo nhạc. Track và preset cũ vẫn giữ liên kết. Genre không bị xóa cứng.

## Tương thích model / LoRA

Không có bảng nối genre ↔ model. Tương thích được thể hiện qua preset (preset chỉ định model/LoRA) và qua `loras.base_model_id` (API từ chối LoRA không cùng base model với model của job).
