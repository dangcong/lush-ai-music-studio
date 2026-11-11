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
  "params": {"inference_steps": null, "guidance_scale": null},
  "idempotency_key": "uuid do client sinh"
}
```
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
`Artifact`: `{id, kind, filename, content_type, size_bytes, sha256}`; `kind`: `audio_master` | `audio_preview`.

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
