# API_CONTRACTS — REST API cho Website

- Base path: `/api/v1`. JSON UTF-8.
- Xác thực: cookie phiên `lush_session` (HttpOnly, SameSite=Lax) do `POST /auth/login` cấp.
- Lỗi: `{"error": {"code": "...", "message": "..."}}` với HTTP status tương ứng
  (`400 bad_request`, `401 unauthorized`, `403 forbidden`, `404 not_found`, `409 conflict`, `422 validation`).
- Thời gian: chuỗi UTC `YYYY-MM-DDTHH:MM:SS.sssZ`. ID: chuỗi UUID.
- Danh sách trả mảng JSON trực tiếp (V1 chưa phân trang; có `limit`).

## Roles

| Role | Quyền |
|---|---|
| `admin` | Mọi thứ: users, genres, presets, models, nodes, audit, mọi job/track |
| `trainer` | Như `member` + Catalog (🎓) và dataset/training (Phase 3) |
| `member` | Tạo nhạc, thư viện, xem/hủy job của mình |

Ký hiệu bên dưới: 🔒 = cần đăng nhập, 👑 = chỉ admin, 🎓 = admin hoặc trainer (`member` nhận `403`).

## Auth

| Method | Path | Body → Response |
|---|---|---|
| POST | `/auth/login` | `{username, password}` → `User` + Set-Cookie. Sai → `401` |
| POST | `/auth/logout` 🔒 | → `204` |
| GET | `/auth/me` 🔒 | → `User` |
| POST | `/auth/password` 🔒 | `{current_password, new_password}` → `204` |

`User`: `{id, username, display_name, role, is_active, created_at}`

## Users 👑

| Method | Path | Ghi chú |
|---|---|---|
| GET | `/users` | → `User[]` |
| POST | `/users` | `{username, password, display_name?, role}` → `201 User` |
| PATCH | `/users/{id}` | `{display_name?, role?, is_active?, password?}` → `User` |

## Projects 🔒

| Method | Path | Ghi chú |
|---|---|---|
| GET | `/projects` | → `Project[]` (admin thấy tất cả, người khác thấy của mình) |
| POST | `/projects` | `{name, description?}` → `201 Project` |
| PATCH | `/projects/{id}` | `{name?, description?}` |

`Project`: `{id, name, description, owner_id, created_at}`

## Genres

| Method | Path | Ghi chú |
|---|---|---|
| GET | `/genres` 🔒 | → `Genre[]` phẳng; `?include_inactive=true` (chỉ admin) |
| POST | `/genres` 👑 | `GenreInput` → `201 Genre` |
| PATCH | `/genres/{id}` 👑 | `GenreInput` từng phần → `Genre` |

`Genre`: `{id, parent_id, name, slug, description, prompt_template, instruments: string[], moods: string[], bpm_min, bpm_max, duration_min, duration_max, is_active, sort_order}`
`duration_min`/`duration_max`: khoảng Duration gợi ý (giây, 10–300). Hoặc điền cả hai (`min ≤ max`), hoặc `null` cả hai; sai thì `422`. Cùng quy tắc cho `Preset`. Thứ tự ưu tiên ở form Tạo nhạc: Style → Subgenre → Genre, lấy khoảng đầu tiên có giá trị.
Subgenre là Genre có `parent_id`. Chỉ hỗ trợ 2 cấp.

## Style Presets

| Method | Path | Ghi chú |
|---|---|---|
| GET | `/presets` 🔒 | `?genre_id=`; `?include_inactive=true` (admin) → `Preset[]` |
| POST | `/presets` 👑 | `PresetInput` → `201 Preset` |
| PATCH | `/presets/{id}` 👑 | từng phần → `Preset` |

`Preset`: `{id, genre_id, name, description, kind: "genre"|"trained", prompt_template, instruments: string[], moods: string[], bpm, duration_min, duration_max, model_id, lora_id, lora_scale, params: object, version, is_active}`
`kind = "trained"` bắt buộc có `lora_id`.

## Models & LoRAs

| Method | Path | Ghi chú |
|---|---|---|
| GET | `/models` 🔒 | → `Model[]` |
| PATCH | `/models/{id}` 👑 | `{name?, description?, status?}` (`active` \| `disabled`) |
| GET | `/loras` 🔒 | → `Lora[]` |
| PATCH | `/loras/{id}` 👑 | `{name?, status?}`; `status`: `evaluation` \| `approved` \| `archived` |

`Model`: `{id, engine, key, name, version, description, capabilities: string[], status, kind: "generator"|"captioner"|"scorer", installed_nodes: [{node_id, node_name, online}]}`
`kind != "generator"` (model mô tả/chấm điểm của Catalog) không dùng để tạo nhạc: `POST /generations` trả 422 "Model này không dùng để tạo nhạc".
`Lora`: `{id, key, name, version, genre_id, base_model_id, status, created_at, job_id, dataset_key, trainer, config: object, adapter_sha256, created_by, installed_nodes: [...]}`. `status`: `training | evaluation | approved | archived`. Các trường từ `job_id` trở đi là nguồn gốc training (`null` nếu LoRA không do hệ thống train). Tạo nhạc cần LoRA `approved`; riêng admin dùng được LoRA `evaluation` để nghe thử.

## AI Training 👑

Train Style LoRA (adapter), không phải train model. Thiết kế và giới hạn: `STYLE_TRAINING.md`.

| Method | Path | Ghi chú |
|---|---|---|
| GET | `/datasets` | → `[{key, node_id, node_name, online, can_train, info}]`: dataset version mà node báo có trên đĩa. `info` = `{key, name, samples, seconds}` |
| POST | `/training-runs` | Tạo run → `201 Job` (`type: train_lora`, `max_attempts: 1`). `409` nếu còn run khác chưa kết thúc; `422` nếu tham số sai, model không phải engine `acestep`, hoặc không node nào vừa train được vừa có dataset |
| GET | `/training-runs` | Trainer/admin. → `Job[]` loại `train_lora`, mới nhất trước |
| GET | `/training-runs/{id}` | Trainer/admin. → `Job` + `log`, `updated_at`, `metrics[]`, `lora`, `snapshot`, `checkpoints[]`. `snapshot` = `{id, catalog_id, catalog_name, samples, held_out: [filename], keyword, test_prompt, models: [{model_key, name, count}]}`, `null` với run kiểu cũ (dataset trên node). `checkpoints` = `[{id, epoch, step, bytes, status}]` sắp theo epoch, `[]` khi chưa có; không lộ đường dẫn trên node (`result.checkpoints` của job cũng đã bỏ `path`). Thêm cho màn hình chọn bản (`null`/`[]` với run kiểu cũ): `lora` = `{id, name, key, base_key, status, chosen_checkpoint_id}` (`base_key` = `key` gốc, bằng `key` khi chưa chọn bản); `eval` = `{job_id, status, progress, stage, error, scorers: {mert, audiobox}}` của job `eval_lora` mới nhất của LoRA; `renders` = `[{id, kind: base|prompt|held_out, checkpoint_id, sample_filename, prompt, artifact_id, scores}]` (nghe qua `/artifacts/{id}/content`); `checkpoints[]` thêm `note, expires_at, delete_requested_at, style_score, train_score, train_match, melody_score, melody_match, quality_score` (điểm chỉ để tham khảo, `null` khi máy chấm chưa cài); `held_out_tracks` = `[{filename, artifact_id}]` từ bản chụp; `style_preset` = `{id, name, lora_scale, is_active}` của Style `trained` gắn LoRA này; `files_job` = `{job_id, status, op, error}` của job `lora_files` mới nhất. Không bao giờ lộ `node_path` hay payload job |
| PATCH | `/lora-checkpoints/{id}` | Trainer/admin. `{note}` (≤ 500 ký tự, nếu không `422 "Ghi chú tối đa 500 ký tự"`) → bản lưu như `checkpoints[]` của chi tiết run (không lộ `node_path`); `404` nếu không có |
| POST | `/loras/{id}/publish` | Trainer/admin. `{checkpoint_id, style_name, lora_scale}` → `202 {job_id, style_preset_id}`. Kiểm theo thứ tự: LoRA không có → `404`, đã `archived` → `422 "LoRA đã bị xóa"`; còn job `lora_files` của LoRA `queued`/`preparing`/`running` → `409 "LoRA đang được cập nhật trên máy, chờ một chút"` (chỉ mục duy nhất `jobs_one_live_lora_files` chặn cả khi bấm đua nhau); bản lưu không thuộc LoRA, đã `deleted` hoặc đã đánh dấu xóa → `422 "Bản lưu này không còn trên máy"`; `style_name` 1–80 ký tự → `422 "Tên Style phải có 1–80 ký tự"`; `lora_scale` ngoài 0–1 → `422 "Độ mạnh phải trong khoảng 0–1"`. Chỉ tạo/cập nhật Style `trained` của LoRA (Style mới `is_active = 0`, genre của catalog, model gốc, `prompt_template` = từ khóa hoặc rỗng; Style đã có giữ nguyên `is_active`, chỉ đổi `name`, `lora_scale`) và tạo job `lora_files` `op=publish`; **không đổi** trạng thái bản lưu, `loras.key`/`chosen_checkpoint_id` hay `lora_installations`. Chỉ khi job xong (node báo đúng key và sha) mới cùng transaction: bản chọn `chosen` (`expires_at` null), `chosen` cũ và mọi bản `kept` thành `kept` với hạn = lúc đó + 30 ngày (bản đã có hạn giữ nguyên), `loras.key` = `<base_key>-e<epoch>` (`base_key` ghi lại khóa gốc), `chosen_checkpoint_id`, `status = 'approved'`, `lora_installations` đặt lại chỉ còn node đó, Style bật. Job hỏng thì bản đang chạy vẫn phục vụ và không có gì đổi dở; gọi lại `publish` là cách thử lại. Nếu bản yêu cầu đang là `chosen` và LoRA đã có `lora_installations` thì chỉ cập nhật Style và trả `200 {job_id: null, style_preset_id}` (không tạo job). Audit `lora.published` | Style mới tạo có `prompt_template = "<keyword>, {prompt}"` (từ khóa đứng đầu như lúc train; không có từ khóa thì rỗng). Khi ghép prompt, `{prompt}` rỗng không để lại dấu phẩy thừa.
| POST | `/loras/{id}/try` | Trainer/admin. `{checkpoint_id, prompt, lora_scale}` → `201 Job` (`type: generate`, chủ là người bấm, `title` = `Thử nhanh · <tên LoRA> · Epoch N`, ra track trong thư viện của họ). Kiểm theo thứ tự: LoRA không có → `404`, đã `archived` → `422 "LoRA đã bị xóa"`; `prompt` (sau khi cắt khoảng trắng) 1–500 ký tự → `422 "Câu lệnh phải có 1–500 ký tự"`; bản lưu không thuộc LoRA, đã `deleted` hoặc đã đánh dấu xóa → `422 "Bản lưu này không còn trên máy"`; `lora_scale` ngoài 0–1 → `422 "Độ mạnh phải trong khoảng 0–1"`. Dựng bằng cùng hàm tạo nhạc thường: genre của catalog, model gốc, 60 giây (kẹp vào khoảng của genre như render nghe thử), không `lora_id`, seed ngẫu nhiên. Đường dẫn bản lưu chỉ nằm trong `payload` gửi node, không có trong `request` (nên không ra ở `GET /jobs`). Audit `lora.tried` |
| POST | `/loras/{id}/cleanup` | Trainer/admin. Xóa ngay các bản `kept`. Chưa có bản `chosen` → `409 "Chọn một bản trước khi xóa các bản còn lại"`; còn job `eval_lora` của LoRA `queued`/`preparing`/`running` → `409 "Đang tạo nhạc nghe thử, chờ xong rồi xóa"`; còn job `lora_files` chưa xong → `409 "LoRA đang được cập nhật trên máy, chờ một chút"`; không còn bản nào để xóa → `200 {job_id: null, count: 0}`; còn lại đánh dấu `delete_requested_at` và tạo job `lora_files` `op=delete` → `202 {job_id, count}`. Bản đã đánh dấu mà không còn job nào đang chạy (job xóa trước đó hỏng) coi là bỏ dở và được chọn lại, nên bấm lần nữa là thử lại. Bản `chosen` không bao giờ bị xóa. Audit `lora.cleanup` |
| DELETE | `/loras/{id}` | Trainer/admin. → `204`. `404` nếu không có, `422 "LoRA đã bị xóa"` nếu đã `archived`. Còn Style Preset (bất kể bật/tắt) gắn `lora_id` này → `409 "Còn Style dùng LoRA này: <tên, cách nhau bởi dấu phẩy>"` (gỡ bằng `PATCH /presets/{id}` đặt `lora_id: null`, `kind: "genre"`); job `train_lora`/`eval_lora` đang `preparing`/`running` hoặc job `lora_files` chưa xong → `409 "LoRA đang có việc chạy trên máy"`. Nếu qua: `loras.status = 'archived'`, xóa `lora_installations`, hủy job `eval_lora` đang `queued`, đánh dấu xóa mọi bản lưu chưa xóa (kể cả `chosen`) và tạo job `lora_files` `op=delete` (nếu có bản để xóa). Audit `lora.deleted`. Bản chụp dataset giữ nguyên |

Body `POST /training-runs`: `{name, dataset_key, base_model_id, rank (1–128), alpha (1–256), learning_rate (0–0.01], epochs (1–1000), gradient_accumulation (1–32), save_every (1–epochs), warmup_steps (0–10000), seed?, max_minutes (1–1440)}`.
`metrics[]`: `{sequence, global_step, epoch, train_loss, learning_rate, elapsed_seconds, gpu_memory_mb, recorded_at}`. `global_step` là optimizer step. Trường trainer không báo là `null` (Side-Step không báo `learning_rate`; không có validation loss).
`stage` khi đang chạy: `preparing | stopping_inference | preprocessing | training | saving | restoring_inference`.
`result` khi xong: `{lora: {key, adapter_sha256, bytes, source}, epochs_completed, optimizer_steps, stop_reason: completed|time_limit, checkpoints: [{epoch, step?, bytes, sha256}] (không có `path`), preprocess_seconds?, train_seconds, total_seconds, vram_peak_mb, inference_restored, adapter_loadable?, acestep_commit, samples, trainer}` (`run_dir` do worker báo nhưng API bỏ khỏi `result` lưu, như `path`).
Hủy: `POST /jobs/{id}/cancel` (mất mọi thứ sau checkpoint cuối). `POST /jobs/{id}/retry` trả `409` với training run.
Khi run xong, LoRA vào Registry ở `evaluation`; không tự `approved`.

## Generation & Jobs 🔒

### `POST /generations`
```json
{
  "prompt": "Warm jazz piano, soft brushed drums",
  "title": "Rainy night",
  "project_id": null,
  "genre_id": "…", "subgenre_id": null, "preset_id": null,
  "model_id": "…",
  "lora_id": null, "lora_scale": null,
  "duration": 120,
  "bpm": null, "key": null, "time_signature": null,
  "seed": null,
  "instruments": ["piano"], "moods": ["warm"],
  "instrumental": true,
  "params": {"inference_steps": null, "guidance_scale": null, "audio_cover_strength": null},
  "mode": "text",
  "reference_audio_id": null,
  "source_audio_id": null,
  "idempotency_key": "uuid do client sinh"
}
```
- `mode` (mặc định `text`):
  - `text` — Text to Music.
  - `reference` — bắt buộc `reference_audio_id`. Reference Audio định hướng chất âm, phối khí, không khí chung; không chép giai điệu, không train LoRA. Model phải có capability `reference_audio`.
  - `cover` — bắt buộc `source_audio_id`. Giữ cấu trúc (giai điệu, nhịp, hợp âm) của Source Audio, đổi phong cách theo prompt. Độ dài theo Source Audio (`duration` bị bỏ qua). `params.audio_cover_strength` 0–1 (cao = bám bản gốc hơn; để trống = mặc định engine 1.0). Model phải có capability `cover`.
- `reference_audio_id` / `source_audio_id` là ID trả về từ `POST /audio-inputs`; phải là file của chính người gửi (hoặc người gửi là admin). Sai chế độ, thiếu file, model không có capability → `422`.
- Bắt buộc: `prompt` (1–2000 ký tự), `duration` (10–300 giây), và `model_id` (hoặc preset có `model_id`).
- Giá trị trong request ghi đè giá trị của preset; preset ghi đè mặc định của genre.
- `seed` rỗng → VPS tự chọn. `idempotency_key` trùng (cùng user) → trả lại job cũ với `200`.
- Model `disabled` hoặc LoRA không tương thích base model → `422`.
- Trả `201 Job`. Job vẫn được nhận khi không có node online (nằm chờ trong hàng đợi).

Cách ghép prompt cuối (`payload.prompt`): template (preset → subgenre → genre) có `{prompt}` thì thay vào, không thì nối sau prompt người dùng; rồi nối `instruments`, `moods`, và `instrumental, no vocals` khi `instrumental`.

### Jobs

| Method | Path | Ghi chú |
|---|---|---|
| GET | `/jobs` | `?status=&type=&mine=true&limit=` (`type` nhận nhiều giá trị cách nhau bằng dấu phẩy, vd. `type=analyze_track,caption_tracks`) → `Job[]` mới nhất trước. Non-admin chỉ thấy job của mình |
| GET | `/jobs/{id}` | → `Job` kèm `log` |
| POST | `/jobs/{id}/cancel` | `queued` → `cancelled` ngay; `preparing`/`running` → đặt `cancel_requested` |
| POST | `/jobs/{id}/retry` | Chỉ cho job `generate` đã `failed`/`cancelled`: tạo job mới cùng payload → `201 Job` |
| GET | `/queue/summary` | → `{queued, preparing, running, nodes_online, training}`; `training` = `{job_id, title, stage, started_at, node_name}` khi một node đang train (lúc đó node không tạo nhạc), ngược lại `null` |

`Job`: `{id, type, status, priority, progress, stage, message, error, attempt, max_attempts, cancel_requested, owner_id, owner_name, node_id, node_name, track_id, title, request: object, result: object|null, created_at, started_at, finished_at}` (+ `log` ở endpoint chi tiết)
`status`: `queued | preparing | running | completed | failed | cancelled`. `request` là nguyên body người dùng đã gửi.

## Tracks (Music Library) 🔒

| Method | Path | Ghi chú |
|---|---|---|
| GET | `/tracks` | `?q=&genre_id=&project_id=&favorite=true&mine=true&limit=` → `Track[]` |
| GET | `/tracks/{id}` | → `Track` |
| PATCH | `/tracks/{id}` | `{title?, favorite?, rating?, project_id?}` (rating 0–5) |
| DELETE | `/tracks/{id}` | Xóa mềm (chủ sở hữu hoặc admin) → `204` |
| GET | `/tracks/{id}/archive` | ZIP mọi artifact đang có của track (không nén lại, tên file trong zip = `Artifact.filename`), `Content-Disposition: attachment; filename="lush-<8 ký tự cuối id>.zip"`. Track đã xóa hoặc không có file → `404` |

`Track`: `{id, title, owner_id, owner_name, project_id, job_id, parent_track_id, version, prompt, prompt_final, genre_id, genre_name, subgenre_id, preset_id, model_id, model_name, lora_id, lora_scale, seed, bpm, key, time_signature, duration, favorite, rating, params: object, artifacts: Artifact[], created_at}`
`Artifact`: `{id, kind, filename, content_type, size_bytes, sha256}`; `kind`: `audio_master` | `audio_preview` (track); `audio_input` chỉ xuất hiện qua Audio Inputs.

## Audio Inputs 🔒

File âm thanh người dùng tải lên để làm Reference Audio hoặc Source Audio. Một file không gắn sẵn vai trò; vai trò do `mode` của job quyết định. Đây **không** phải Training Dataset (dataset LoRA đi thẳng tới Render Node ở Phase 3).

| Method | Path | Ghi chú |
|---|---|---|
| POST | `/audio-inputs` | multipart, field `file`. Chỉ `.wav` / `.mp3` / `.flac`, kiểm tra cả chữ ký đầu file; tối đa `MAX_INPUT_AUDIO_MB` (mặc định 50). → `201 AudioInput`. Sai định dạng → `422`, quá lớn → `413` |
| GET | `/audio-inputs/{id}` | → `AudioInput` (chủ file hoặc admin) |

`AudioInput`: `{id, filename, content_type, size_bytes, sha256, created_at}`. Nghe lại bằng `GET /artifacts/{id}/content`.
Track tạo từ chế độ `reference` / `cover` lưu `params.mode` và `params.inputs` = `{"reference_audio" | "source_audio": {artifact_id, filename, size_bytes, sha256}}`.

## Catalogs 🎓

Catalog là bộ bài hát dùng làm dữ liệu train LoRA, gắn với một thể loại.

| Method | Path | Ghi chú |
|---|---|---|
| GET | `/catalogs` | → `Catalog[]`, mới nhất trước, bỏ catalog đã xóa |
| POST | `/catalogs` | `{name, genre_id}` → `201 Catalog`. Tên rỗng/toàn khoảng trắng hoặc dài hơn 120 ký tự, hoặc `genre_id` không tồn tại → `422`. Trùng tên (với catalog chưa xóa) → `409 conflict` |
| GET | `/catalogs/{id}` | → `Catalog`; không có hoặc đã xóa → `404` |
| PATCH | `/catalogs/{id}` | `{name?}` → `Catalog` (cùng quy tắc tên như khi tạo) |
| DELETE | `/catalogs/{id}` | Xóa mềm → `204`; sau đó tên được dùng lại |
| POST | `/catalogs/{id}/tracks` | multipart, field `file` (một file mỗi request; WAV/MP3/FLAC, giới hạn `MAX_INPUT_AUDIO_MB`) → `201 CatalogTrack` với `analysis_status: "pending"` và tạo job `analyze_track`. Catalog không có/đã xóa → `404`; file sai định dạng/rỗng → `422`; quá lớn → `413`; cùng SHA-256 với bài chưa xóa trong catalog → `409` (thông báo nêu tên bài đã có; bài đã xóa mềm thì tải lại được; catalog khác thì không trùng) |
| GET | `/catalogs/{id}/tracks` | → `CatalogTrack[]`, `created_at` tăng dần, bỏ bài đã xóa |
| POST | `/catalogs/{id}/caption-jobs` | `{model_key, track_ids: string[], force?: bool}` → `200 {job_id: string\|null, queued: string[], skipped: string[]}` (200, không phải 201, vì có thể không tạo job). Tạo **một** job `caption_tracks` cho các bài trong `queued`; `queued` rỗng thì `job_id: null` và không tạo job. Không có `force`: bỏ qua (`skipped`) bài đã có head cho model đó và bài đang nằm trong job `caption_tracks` cùng model còn `queued`/`preparing`/`running`; `force: true` không bỏ qua bài nào. Khi tạo job, `current_model_key` của các bài trong `queued` được đặt thành `model_key`. `model_key` không phải captioner `active` → `422 "Model mô tả không tồn tại"`; captioner chưa cài trên node `active` nào → `422 "Model chưa cài trên máy render"`; có bài không thuộc catalog hoặc đã xóa → `422`; catalog không có → `404` |
| POST | `/catalogs/{id}/lora-preview` | Trainer/admin. Body `BuildRequest`: `{name, keyword?, test_prompt, track_ids, holdout_count, holdout_seed, voice_confirmed?: string[], base_model_id, rank, alpha, learning_rate, epochs, gradient_accumulation, save_every, warmup_steps, seed?, max_minutes}` → `200 BuildPlan` (không ghi gì). `BuildPlan`: `{ready, errors[], warnings[], train[], held_out[], missing_caption[], voice_unconfirmed[], no_tempo_key[], train_seconds, unknown_duration, models[{model_key,name,count}], steps_per_epoch, total_steps, checkpoint_epochs[], eval_renders, name_taken}` (`eval_renders = min(số bản lưu, 10) × (1 + số bài giữ lại) + 1`: chỉ 10 bản lưu cuối được nghe thử); mỗi bài `{id, filename, duration_s, model_key, vocals, caption}`. Bài giữ lại chọn trong số bài đủ điều kiện (có mô tả ở model đang hiện, không chờ xác nhận giọng) bằng xáo trộn theo `holdout_seed` nên cùng seed cho cùng kết quả. `errors` (chặn train, theo thứ tự hiển thị): tên 1–80 ký tự, tên trùng, từ khóa, câu nghe thử 1–500 ký tự, số bài giữ lại 0–5, bài thiếu mô tả, bài nghi có giọng chưa xác nhận, không còn bài để train; `ready` = `errors` rỗng. Config sai khoảng như tạo training run, `track_ids` rỗng/trùng hoặc có bài không thuộc catalog → `422`; catalog không có → `404`; member → `403` | `warnings` có thêm dòng "N bài có mô tả dài quá 850 ký tự … phần cuối sẽ bị bỏ khi train" khi có bài bị cắt; `no_tempo_key` tính cả key không chuẩn hóa được.
| POST | `/catalogs/{id}/loras` | Trainer/admin. Body `BuildRequest` như `lora-preview` → `201 {job_id, snapshot_id, lora_key}`. Chạy lại `plan_build`: `ready=false` → `422` với lỗi đầu tiên của `errors`; có bài đã xóa giữa lúc xem trước và tạo → `422 "Có bài không còn trong catalog này"`; `base_model_id` không có/đang tắt/không có trainer → `422` như training run; không node `active` nào nhận `train_lora` → `422 "Không có render node nào hỗ trợ training"`; đang có run `queued`/`preparing`/`running` → `409 "Đang có một training run khác chưa kết thúc"` (kiểm lại nguyên tử khi chèn job). Trong một transaction tạo job `train_lora` (`title` = tên LoRA, `max_attempts` 1, payload dạng bản chụp, xem WORKER_PROTOCOL) và dòng `dataset_snapshots`; `lora_key = slug(tên)-<8 ký tự cuối job id>`; không có `seed` thì sinh ngẫu nhiên. Tên trùng (không phân biệt hoa thường, theo Unicode) với LoRA chưa lưu trữ hoặc job `train_lora` đang chạy → `name_taken`. Audit `training.created` `{snapshot_id, catalog_id, lora_key}`; member → `403` |
| GET | `/dataset-snapshots/{id}` | Trainer/admin. → `{id, catalog_id, job_id, lora_key, created_at, content}`. `content` (schema 1) là bản chụp bất biến của lần chọn: `{schema, catalog, genre, lora, keyword, test_prompt, holdout_seed, base_model, config{…, max_duration_s: 240}, limits, samples[], held_out[], models[]}`; mỗi sample `{catalog_track_id, artifact_id, filename, sha256, size_bytes, duration_s, model_key, caption_seq, caption_source, mood, instruments, vocals, caption, train_caption, bpm, keyscale, voice_confirmed}`. Sửa caption/tempo của catalog về sau không đổi bản chụp; DB từ chối `UPDATE`. Không có → `404` | `train_caption` = `keyword, caption`, rồi các mục của Instruments và Mood (tách theo dấu phẩy) mà caption chưa có (không phân biệt hoa thường), cắt còn tối đa 850 ký tự ở cuối câu (không có thì dấu phẩy, rồi khoảng trắng; chỉ điểm cắt ở nửa sau) vì ACE-Step chỉ đọc 256 token đầu của prompt train và bpm/key nằm sau caption. `keyscale` là key đã chuẩn hóa; key cũ không chuẩn hóa được gửi như thiếu (N/A).
| PATCH | `/catalog-tracks/{id}` | `{tempo_bpm?: number\|null, key_name?: string\|null, current_model_key?: string}` → `CatalogTrack`. Sửa tempo/key đặt `*_source='human'`; `null` xóa giá trị (source vẫn `human`). `tempo_bpm` ngoài 20..=300, `key_name` không chuẩn hóa được thành dạng keyscale của ACE-Step (`A minor`, `C# major`, `Bb minor`; chấp nhận viết tắt `Am`, `C#m`, `Cmaj`, `CM`, nốt trần = major, `♯`/`♭`, hoa/thường tự do; lưu ở dạng chuẩn), hoặc `current_model_key` không phải key của model `kind='captioner'` → `422` |
| DELETE | `/catalog-tracks/{id}` | Xóa mềm → `204`; bài biến mất khỏi `GET tracks`, mọi endpoint `/catalog-tracks/{id}/…` trả `404` |
| POST | `/catalog-tracks/{id}/reanalyze` | Tạo job `analyze_track` mới (cùng payload khi upload), đặt `analysis_status='pending'` và `analysis_job_id` mới → `CatalogTrack`. Job đo cũ còn `queued`/`preparing`/`running` → `409`. Dùng cho nút **Thử lại** |
| PUT | `/catalog-tracks/{id}/captions/{model_key}` | `{mood, instruments, vocals, caption}` → `CaptionView`. Thêm lượt `source='human'` với `seq = max+1` và đặt head vào đó (không xóa lượt nào, kể cả các lượt "sau" khi head đang lùi). `mood`/`instruments`/`vocals` tối đa 500 ký tự, `caption` tối đa 4000 (vượt → `422`). `model_key` không phải captioner → `422`; hai lần lưu đồng thời va nhau → `409` |
| POST | `/catalog-tracks/{id}/captions/{model_key}/undo` | Dời head sang lượt nhỏ hơn gần nhất → `CaptionView`. Không có → `409 "Không còn bản trước"`; chưa có mô tả cho model → `404` |
| POST | `/catalog-tracks/{id}/captions/{model_key}/redo` | Dời head sang lượt lớn hơn gần nhất → `CaptionView`. Không có → `409 "Không còn bản sau"` |

`CaptionView`: `{model_key, seq, mood, instruments, vocals, caption, source: "model"|"human", source_ref, created_at, can_undo, can_redo}`.

`Catalog`: `{id, name, genre_id, genre_name, track_count, total_seconds, created_at}`. Tên được cắt khoảng trắng hai đầu. `track_count` / `total_seconds` (tổng `duration_s`, `0` khi chưa có bài) chỉ tính bài chưa xóa. Audit: `catalog.created`, `catalog.updated`, `catalog.deleted`, `catalog_track.uploaded`.

`CatalogTrack`: `{id, catalog_id, artifact_id, filename, duration_s, tempo_bpm, tempo_source, key_name, key_source, analysis_status, estimates: {tempo: {bpm, confidence, alternatives[]} | null, key: {key, confidence} | null}, current_model_key, models_done: string[], caption: CaptionView | null, captioning_model_key: string | null, voice_warning: bool, created_at}`.
- `tempo_source` / `key_source`: `auto` (đo) hoặc `human` (người sửa); giá trị `human` không bị kết quả đo ghi đè. `estimates` luôn là kết quả đo gần nhất, kể cả khi bị giá trị tay che.
- `analysis_status`: `pending` | `done` | `failed`. `failed` được suy ra khi đọc từ job phân tích (job `failed` hoặc `cancelled`), không lưu trong bảng.
- `caption` = bản head của `current_model_key` (`null` nếu chưa có); `models_done` = các `model_key` đã có head; `voice_warning` = `vocals` của bản head khác rỗng và khác `instrumental` (không phân biệt hoa thường), hoặc caption chứa một từ nguyên trong `VOICE_TERMS` (`vocal, vocals, voice, singing, singer, sung, hum, humming, chant, spoken, speech, rap, choir, lyrics`; so khớp theo từ, không phân biệt hoa thường). `captioning_model_key` = `model.key` của job `caption_tracks` đang `queued`/`preparing`/`running` có chứa bài này **mà bài chưa có lượt nào do job đó ghi**; `null` nếu không có. Job bị hủy/hết lượt/đã xong thì bài chưa xong cũng về `null`, bài đã có kết quả giữ nguyên caption.
- Audit thêm: `catalog_track.updated`, `catalog_track.deleted`, `catalog_track.reanalyzed`, `catalog_track.caption_edited`, `catalog.caption_job_created`.

## Artifacts 🔒

`GET /artifacts/{id}/content` — trả file (hỗ trợ `Range` để tua). `?download=true` thêm `Content-Disposition: attachment`.
API không bao giờ lộ đường dẫn file hệ thống; chỉ dùng Artifact ID.

## Render Nodes 👑

| Method | Path | Ghi chú |
|---|---|---|
| GET | `/nodes` | → `Node[]` |
| POST | `/nodes` | `{name}` → `201 {node: Node, token: "…"}` (token chỉ trả một lần) |
| PATCH | `/nodes/{id}` | `{name?, status?, maintenance?}`; `status`: `active` \| `disabled` |
| POST | `/nodes/{id}/rotate-token` | → `{token}` |

`Node`: `{id, name, status, maintenance, online, worker_version, protocol_version, job_types: string[], engines: object[], gpu: object, models: [{id, key, engine}], loras: [{id, key}], current_job_id, last_seen, registered_at, created_at}`

## Audit Logs 👑

`GET /audit-logs?limit=` → `[{id, actor_id, actor_name, action, target_type, target_id, detail: object, created_at}]`

## Health

`GET /api/health` (không cần đăng nhập) → `{status: "ok", version, db: "ok"}`
