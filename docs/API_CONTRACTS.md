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
| `trainer` | Như `member` + dataset/training (Phase 3) |
| `member` | Tạo nhạc, thư viện, xem/hủy job của mình |

Ký hiệu bên dưới: 🔒 = cần đăng nhập, 👑 = chỉ admin.

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

`Genre`: `{id, parent_id, name, slug, description, prompt_template, instruments: string[], moods: string[], bpm_min, bpm_max, default_duration, is_active, sort_order}`
Subgenre là Genre có `parent_id`. Chỉ hỗ trợ 2 cấp.

## Style Presets

| Method | Path | Ghi chú |
|---|---|---|
| GET | `/presets` 🔒 | `?genre_id=`; `?include_inactive=true` (admin) → `Preset[]` |
| POST | `/presets` 👑 | `PresetInput` → `201 Preset` |
| PATCH | `/presets/{id}` 👑 | từng phần → `Preset` |

`Preset`: `{id, genre_id, name, description, kind: "genre"|"trained", prompt_template, instruments: string[], moods: string[], bpm, duration, model_id, lora_id, lora_scale, params: object, version, is_active}`
`kind = "trained"` bắt buộc có `lora_id`.

## Models & LoRAs

| Method | Path | Ghi chú |
|---|---|---|
| GET | `/models` 🔒 | → `Model[]` |
| PATCH | `/models/{id}` 👑 | `{name?, description?, status?}` (`active` \| `disabled`) |
| GET | `/loras` 🔒 | → `Lora[]` (Phase 3 mới có dữ liệu) |

`Model`: `{id, engine, key, name, version, description, capabilities: string[], status, installed_nodes: [{node_id, node_name, online}]}`
`Lora`: `{id, key, name, version, genre_id, base_model_id, status, installed_nodes: [...]}`

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
- Bắt buộc: `prompt` (1–2000 ký tự), `duration` (10–240 giây), và `model_id` (hoặc preset có `model_id`).
- Giá trị trong request ghi đè giá trị của preset; preset ghi đè mặc định của genre.
- `seed` rỗng → VPS tự chọn. `idempotency_key` trùng (cùng user) → trả lại job cũ với `200`.
- Model `disabled` hoặc LoRA không tương thích base model → `422`.
- Trả `201 Job`. Job vẫn được nhận khi không có node online (nằm chờ trong hàng đợi).

Cách ghép prompt cuối (`payload.prompt`): template (preset → subgenre → genre) có `{prompt}` thì thay vào, không thì nối sau prompt người dùng; rồi nối `instruments`, `moods`, và `instrumental, no vocals` khi `instrumental`.

### Jobs

| Method | Path | Ghi chú |
|---|---|---|
| GET | `/jobs` | `?status=&type=&mine=true&limit=` → `Job[]` mới nhất trước. Non-admin chỉ thấy job của mình |
| GET | `/jobs/{id}` | → `Job` kèm `log` |
| POST | `/jobs/{id}/cancel` | `queued` → `cancelled` ngay; `preparing`/`running` → đặt `cancel_requested` |
| POST | `/jobs/{id}/retry` | Chỉ cho `failed`/`cancelled`: tạo job mới cùng payload → `201 Job` |
| GET | `/queue/summary` | → `{queued, preparing, running, nodes_online}` |

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
