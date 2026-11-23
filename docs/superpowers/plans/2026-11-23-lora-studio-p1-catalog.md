# LoRA Studio P1: Catalog và tạo mô tả — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Trang Catalog trên website: tạo catalog, tải nhạc lên, tự đo Tempo/Key trên render node, tạo mô tả bằng model chọn ở từng bài (lưu riêng theo model, có Chạy lại / Quay về / Tới / sửa tay).

**Architecture:**
- API Rust có các bảng mới: `catalogs`, `catalog_tracks`, `caption_revisions`, `caption_heads`.
- Thêm hai job type: `analyze_track` (tạo khi upload) và `caption_tracks` (một job cho một nhóm bài và một model).
- Worker Python chạy hai job này:
  - Đo bằng librosa (port từ `lush-training`, không import).
  - Gọi captioner là tiến trình con trong venv riêng của từng model, giao tiếp bằng JSON lines.
  - Gửi kết quả từng bài về VPS qua endpoint mới.
- Model mô tả là một dòng trong Model Registry có `kind='captioner'`.
- Web dùng React Query như các trang hiện có.

**Tech Stack:**
- Rust (Axum, SQLx runtime query, SQLite).
- Python 3 worker (httpx, numpy; extra `analysis`: librosa, soundfile; test bằng pytest + respx).
- React 19 + TypeScript + @tanstack/react-query v5 (test bằng vitest + testing-library).

**Spec:** `docs/superpowers/specs/2026-11-23-lora-studio-web-design.md` (mục 6.1, 6.2, 6.5, 6.7, 7-P1, 9-P1)

## Global Constraints

- Chuyển trạng thái job là **một câu SQL nguyên tử** có điều kiện `status`/`lease_token`. Không đọc rồi mới ghi.
- SQL dùng `sqlx::query`/`query_as` runtime. Schema mới nằm trong migration mới `0006_catalogs.sql`, không sửa migration cũ.
- API công khai chỉ lộ Artifact ID, không lộ đường dẫn file.
- Danh sách model là **dữ liệu**. Không hardcode tên model mô tả ở API hay web. Tên model chỉ được nêu ở cấu hình của node.
- Đổi hành vi thì sửa `docs/API_CONTRACTS.md` / `docs/WORKER_PROTOCOL.md` **trong cùng commit**.
- Worker không import gì từ `lush-training/` (D28). Logic đo được **sao chép** sang.
- Tempo/Key ở cấp bài, dùng chung mọi model (W3). Giá trị người sửa (`*_source='human'`) không bị kết quả đo ghi đè.
- Tick nhiều bài: bài đã có kết quả của model đó thì bỏ qua (W6). Riêng nút Chạy lại của từng bài thì `force: true`.
- Chạy lại ghi đè bản hiện tại (thêm lượt mới). Quay về / Tới chỉ dời con trỏ, không xóa lượt nào (W7, mục 6.2).
- Quyền: `admin` và `trainer` dùng Catalog. `member` nhận 403 (mục 6.7).
- Ưu tiên job (`priority`, số lớn chạy trước): generate 0, `analyze_track` -10, `caption_tracks` -20.
- Giao diện không hiện thông số model (W4). Chuỗi hiển thị bằng tiếng Việt, code và API bằng tiếng Anh.
- Upload giới hạn bằng `MAX_INPUT_AUDIO_MB` hiện có (mặc định 50). Định dạng nhận: mp3, wav, flac.
- Không commit nhạc, model hay trọng số. Không tải model, không deploy khi chưa được duyệt.

## Review Focus

1. **Upload trùng file** (cùng SHA-256 trong cùng catalog) → 409 kèm tên bài đã có. Bài đã xóa mềm thì cho tải lại. *(Test ở Task 3.)*
2. **Job `caption_tracks` bị hủy hoặc hết lượt giữa chừng** → các bài đã có kết quả vẫn giữ. Bài chưa xong không bị kẹt trạng thái "Đang tạo mô tả…". *(Test ở Task 5.)*
3. **Worker gửi lại kết quả cùng một bài cho cùng một job** (retry mạng) → không tạo hai lượt. *(Test ở Task 5.)*
4. **Captioner trả JSON hỏng hoặc rỗng** → thử lại một lần. Sau đó đưa nguyên văn vào Caption; nếu rỗng thì báo lỗi riêng bài đó, không làm hỏng cả job. *(Test ở Task 7.)*
5. **Model mô tả bị chọn ở form Tạo nhạc** → API trả 422, và web không liệt kê model đó. *(Test ở Task 1 và Task 10.)*

---

## File Structure

| File | Trách nhiệm |
|---|---|
| `apps/api/migrations/0006_catalogs.sql` | Bảng catalog, bài, lượt mô tả, con trỏ; cột `models.kind` |
| `apps/api/src/auth.rs` | Thêm extractor `Trainer` |
| `apps/api/src/routes/catalogs.rs` | Catalog CRUD, upload bài, danh sách bài, sửa bài, mô tả tay, undo/redo, tạo job caption |
| `apps/api/src/routes/inputs.rs` | Tách `store_audio_upload` dùng chung |
| `apps/api/src/routes/worker.rs` | `complete` cho `analyze_track`; endpoint `POST /jobs/{id}/captions`; register đặt `kind` |
| `apps/api/src/routes/jobs.rs` | Chặn model không phải `generator` |
| `workers/music-worker/src/lush_worker/analysis.py` | Đo tempo/key/duration/loudness |
| `workers/music-worker/src/lush_worker/captioning.py` | Tìm captioner, tiến trình con, phân tích kết quả, job `caption_tracks` |
| `workers/music-worker/src/lush_worker/runner.py` | Dispatch theo job type, đăng ký job type và model captioner |
| `workers/music-worker/captioners/moss_music/` | Script chạy MOSS trong venv riêng (chưa chạy khi chưa duyệt tải) |
| `apps/web/src/pages/CatalogsPage.tsx`, `pages/CatalogPage.tsx`, `features/catalog/*` | Giao diện |

---

### Task 1: Migration, quyền Trainer, loại model

**Files:**
- Create: `apps/api/migrations/0006_catalogs.sql`
- Modify: `apps/api/src/auth.rs`, `apps/api/src/routes/registry.rs` (struct `Model` + `list_models`), `apps/api/src/routes/worker.rs` (register), `apps/api/src/routes/jobs.rs:220-233`
- Modify: `apps/api/tests/api.rs` (thêm user `carol` vai trò `trainer` trong `TestApp::new`)
- Docs: `docs/API_CONTRACTS.md` (Models: trường `kind`), `docs/WORKER_PROTOCOL.md` (register: engine `captioner` → kind `captioner`)

**Interfaces:**
- Produces:
  - `pub struct Trainer(pub AuthUser)`: nhận `admin` hoặc `trainer`, các vai trò khác trả 403 `forbidden`.
  - `Model.kind: String`, một trong `generator` | `captioner` | `scorer`.
  - Bảng như bên dưới.

Migration (các tên cột này là hợp đồng cho mọi task sau):

```sql
ALTER TABLE models ADD COLUMN kind TEXT NOT NULL DEFAULT 'generator'
  CHECK (kind IN ('generator','captioner','scorer'));

CREATE TABLE catalogs (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, genre_id TEXT NOT NULL REFERENCES genres(id),
  created_by TEXT NOT NULL REFERENCES users(id), created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
  deleted_at TEXT);
CREATE UNIQUE INDEX catalogs_name_live ON catalogs(name) WHERE deleted_at IS NULL;

CREATE TABLE catalog_tracks (
  id TEXT PRIMARY KEY, catalog_id TEXT NOT NULL REFERENCES catalogs(id),
  artifact_id TEXT NOT NULL REFERENCES artifacts(id), filename TEXT NOT NULL, sha256 TEXT NOT NULL,
  duration_s REAL, tempo_bpm REAL, tempo_source TEXT CHECK (tempo_source IN ('auto','human')),
  key_name TEXT, key_source TEXT CHECK (key_source IN ('auto','human')),
  analysis TEXT NOT NULL DEFAULT '{}',
  analysis_status TEXT NOT NULL DEFAULT 'pending' CHECK (analysis_status IN ('pending','done','failed')),
  analysis_job_id TEXT REFERENCES jobs(id),
  current_model_key TEXT,
  created_by TEXT NOT NULL REFERENCES users(id), created_at TEXT NOT NULL, deleted_at TEXT);
CREATE UNIQUE INDEX catalog_tracks_sha_live ON catalog_tracks(catalog_id, sha256) WHERE deleted_at IS NULL;

CREATE TABLE caption_revisions (
  id TEXT PRIMARY KEY, catalog_track_id TEXT NOT NULL REFERENCES catalog_tracks(id),
  model_key TEXT NOT NULL, seq INTEGER NOT NULL,
  mood TEXT NOT NULL DEFAULT '', instruments TEXT NOT NULL DEFAULT '', vocals TEXT NOT NULL DEFAULT '',
  caption TEXT NOT NULL DEFAULT '',
  source TEXT NOT NULL CHECK (source IN ('model','human')), source_ref TEXT NOT NULL DEFAULT '',
  job_id TEXT REFERENCES jobs(id), created_by TEXT REFERENCES users(id), created_at TEXT NOT NULL,
  UNIQUE (catalog_track_id, model_key, seq));
CREATE UNIQUE INDEX caption_revisions_job_track ON caption_revisions(job_id, catalog_track_id) WHERE job_id IS NOT NULL;

CREATE TABLE caption_heads (
  catalog_track_id TEXT NOT NULL REFERENCES catalog_tracks(id), model_key TEXT NOT NULL,
  current_seq INTEGER NOT NULL, PRIMARY KEY (catalog_track_id, model_key));
```

- `model_key` lưu `models.key` (ví dụ `moss-music-8b-instruct`), không lưu id, để bền qua lần register lại.
- Register: model có `engine == "captioner"` được upsert với `kind='captioner'`, `engine == "scorer"` với `kind='scorer'`, còn lại là `generator`.

- [ ] **Step 1: Viết test lỗi trước** trong `apps/api/tests/api.rs`:
  - `trainer_role_can_use_trainer_routes`: tạm dùng `GET /api/v1/catalogs`. carol (trainer) nhận 200 `[]`, alice (member) nhận 403, admin nhận 200.
  - `captioner_model_reported_with_kind_and_rejected_for_generation`:
    - Node register thêm `{engine:"captioner", key:"moss-music-8b-instruct", name:"MOSS-Music 8B", capabilities:["caption"]}`.
    - `GET /models` có model đó với `kind == "captioner"`, model ACE-Step có `kind == "generator"`.
    - `POST /generations` với `model_id` của captioner trả 422, thông báo `"Model này không dùng để tạo nhạc"`.
- [ ] **Step 2:** `cargo test --test api trainer_role captioner_model` → FAIL (404 route, không có `kind`).
- [ ] **Step 3:** Viết migration, `Trainer`, `kind` trong `Model` và register, chặn ở `create_generation` (kiểm `kind != 'generator'` trước kiểm capability). Thêm route stub `GET /catalogs` trả `[]` trong `routes/catalogs.rs` (Task 2 thay bằng bản thật). Đăng ký module trong `routes/mod.rs` và `lib.rs`.
- [ ] **Step 4:** `cargo test` → toàn bộ đạt. `cargo clippy --all-targets` sạch.
- [ ] **Step 5:** Commit `feat(api): catalogs schema, trainer role, model kind`.

### Task 2: Catalog CRUD

**Files:** `apps/api/src/routes/catalogs.rs`, `lib.rs`, `apps/api/tests/api.rs`, `docs/API_CONTRACTS.md` (thêm mục `## Catalogs 🎓` sau "Audio Inputs"; 🎓 = admin/trainer)

**Interfaces:**
- Consumes: `Trainer`.
- Produces: JSON `Catalog {id, name, genre_id, genre_name, track_count, total_seconds, created_at}`.
- Endpoints:
  - `GET /catalogs` (mới nhất trước, bỏ bản đã xóa).
  - `POST /catalogs {name, genre_id}` → 201.
  - `GET /catalogs/{id}`.
  - `PATCH /catalogs/{id} {name?}`.
  - `DELETE /catalogs/{id}` → xóa mềm, trả 204.
- Audit: `catalog.created`, `catalog.updated`, `catalog.deleted`.

- [ ] **Step 1: Test** `catalog_crud`:
  - Tạo `{name:"Lo-fi chill 2026", genre_id:<lofi>}` → 201, `track_count == 0`.
  - Tạo trùng tên → 409 `conflict`. `genre_id` không tồn tại → 422.
  - Tên rỗng hoặc toàn khoảng trắng → 422.
  - PATCH tên → 200.
  - DELETE → 204, sau đó GET trả 404 và tạo lại cùng tên được.
- [ ] **Step 2:** `cargo test --test api catalog_crud` → FAIL.
- [ ] **Step 3:** Viết handler. Tên được trim, tối đa 120 ký tự.
- [ ] **Step 4:** `cargo test` đạt.
- [ ] **Step 5:** Commit `feat(api): catalog CRUD`.

### Task 3: Tải bài vào catalog, job `analyze_track`, danh sách bài

**Files:** `routes/inputs.rs` (tách helper), `routes/catalogs.rs`, `routes/worker.rs` (`complete`), `tests/api.rs`, hai file hợp đồng (`WORKER_PROTOCOL.md`: thêm mục "Job `analyze_track`" sau mục train_lora)

**Interfaces:**
- Produces:
  - `pub async fn store_audio_upload(state: &AppState, user: &AuthUser, multipart: Multipart) -> Result<StoredAudio, AppError>`, với `StoredAudio {artifact_id, filename, content_type, size_bytes, sha256}`. `upload_audio_input` gọi lại helper này, giữ nguyên hành vi.
  - `POST /catalogs/{id}/tracks` (multipart `file`, một file mỗi request) → 201 `CatalogTrack`. Body limit giống `/audio-inputs`.
  - `GET /catalogs/{id}/tracks` → `CatalogTrack[]` theo `created_at` tăng dần.
  - `CatalogTrack`:
    ```
    {id, catalog_id, artifact_id, filename, duration_s, tempo_bpm, tempo_source, key_name, key_source,
     analysis_status, estimates: {tempo: {bpm, confidence, alternatives[]}|null, key: {key, confidence}|null},
     current_model_key, models_done: string[], caption: CaptionView|null,
     captioning_model_key: string|null, voice_warning: bool, created_at}
    ```
    Task 4 và 5 điền các trường caption. Ở task này chúng là `null` / `[]` / `false`.
  - Job `analyze_track`:
    - Payload: `{"catalog_track_id": "...", "inputs": {"track": {artifact_id, filename, size_bytes, sha256}}}`, `priority -10`, `max_attempts 3`, `title = filename`.
    - Kết quả `complete`:
      ```
      {"duration_s": f, "tempo": {"bpm": f|null, "confidence": f, "alternatives": [f]},
       "key": {"key": "D minor"|null, "confidence": f}, "loudness_lufs": f|null,
       "clipping": bool, "silence_ratio": f}
      ```
    - `complete` lưu toàn bộ vào `analysis`, đặt `analysis_status='done'`, `duration_s`. Chỉ ghi `tempo_bpm` / `key_name` (với `*_source='auto'`) khi giá trị khác null **và** source hiện tại không phải `human`. Dùng một câu `UPDATE ... SET tempo_bpm = CASE WHEN tempo_source='human' THEN tempo_bpm ELSE ? END`.
    - Job `failed` hết lượt thì đặt `analysis_status='failed'`, trong cùng chỗ đánh dấu fail ở `worker.rs` (theo type).

- [ ] **Step 1: Test** `catalog_upload_creates_analyze_job`:
  - Tải `WAV` → 201, `analysis_status == "pending"`.
  - Node register `job_types:["generate","analyze_track"]`, claim được job `analyze_track` có `payload.inputs.track.artifact_id` đúng, tải input được (`GET /jobs/{id}/inputs/{aid}` → 200).
  - `complete` với `tempo.bpm 78.0, key "D minor"` → `GET tracks` có `tempo_bpm 78.0`, `tempo_source "auto"`, `estimates.tempo.confidence` đúng.
- [ ] **Step 2: Test** `catalog_upload_duplicate_rejected`: tải lại cùng bytes vào cùng catalog → 409, thông báo chứa tên file cũ. Xóa mềm bài rồi tải lại → 201. Cùng bytes ở catalog khác → 201.
- [ ] **Step 3: Test** `analyze_does_not_overwrite_human_values`: PATCH tempo tay (endpoint thuộc Task 4; ở task này set thẳng bằng SQL trong test) rồi `complete` → `tempo_bpm` giữ giá trị tay.
- [ ] **Step 4: Test** `analyze_track_claim_order`: một job generate queued và một analyze queued → node claim generate trước.
- [ ] **Step 5:** `cargo test` → FAIL. Viết code. Kiểm trùng trước khi lưu file: tính sha trong helper rồi kiểm; nếu trùng thì xóa artifact vừa tạo trong cùng transaction, hoặc kiểm trước insert artifact.
- [ ] **Step 6:** `cargo test`, clippy đạt.
- [ ] **Step 7:** Commit `feat(api): catalog track upload and analyze_track job`.

### Task 4: Sửa bài, mô tả tay, Quay về / Tới

**Files:** `routes/catalogs.rs`, `tests/api.rs`, `API_CONTRACTS.md`

**Interfaces:**
- Produces:
  - `PATCH /catalog-tracks/{id} {tempo_bpm?: number|null, key_name?: string|null, current_model_key?: string}`. Sửa tempo/key đặt `*_source='human'`; gửi `null` thì xóa giá trị và đặt source `human`. `current_model_key` phải là key của một model `kind='captioner'`, không thì 422.
  - `PUT /catalog-tracks/{id}/captions/{model_key} {mood, instruments, vocals, caption}` → thêm lượt `source='human'`, `seq = max+1`, head trỏ tới lượt mới. Trả `CaptionView`.
  - `POST /catalog-tracks/{id}/captions/{model_key}/undo` và `/redo`:
    - Undo: head sang lượt **nhỏ hơn gần nhất**. Không có thì 409 `"Không còn bản trước"`.
    - Redo: head sang lượt lớn hơn gần nhất. Không có thì 409 `"Không còn bản sau"`.
  - `DELETE /catalog-tracks/{id}` → xóa mềm, 204.
  - `POST /catalog-tracks/{id}/reanalyze` → tạo job `analyze_track` mới (payload như Task 3), đặt `analysis_status='pending'`, `analysis_job_id` mới. Đang có job đo chạy cho bài này thì trả 409. Dùng cho nút **Thử lại** khi đo lỗi (spec 6.1). Test: `reanalyze_after_failure`.
  - `CaptionView {model_key, seq, mood, instruments, vocals, caption, source, source_ref, created_at, can_undo, can_redo}`.
  - Trong `GET tracks`:
    - `caption` = bản head của `current_model_key`.
    - `models_done` = các `model_key` có head.
    - `voice_warning` = `vocals` của bản head hiện tại khác rỗng và khác `instrumental` (không phân biệt hoa thường), **hoặc** caption chứa từ trong `VOICE_TERMS`.
  - `pub const VOICE_TERMS: &[&str] = &["vocal", "vocals", "voice", "singing", "singer", "sung", "hum", "humming", "chant", "spoken", "speech", "rap", "choir", "lyrics"]`. So khớp theo từ nguyên, không phân biệt hoa thường.
- Lượt mới tạo ra khi head đang lùi (sau undo) vẫn lấy `seq = max+1`, không xóa các lượt "sau" (spec 6.2: không xóa lượt nào).

- [ ] **Step 1: Test** `caption_revisions_undo_redo`:
  - PUT bản A, PUT bản B → head = B, `can_undo`.
  - Undo → A, `can_redo`. Redo → B. Undo thêm hai lần → 409.
  - PUT C lúc head = A → head = C (seq 3), undo → B (seq 2). Đây là ví dụ lượt sau không bị xóa.
- [ ] **Step 2: Test** `switching_model_keeps_results`:
  - Thêm lượt cho model X và Y. PATCH `current_model_key=Y` → `caption.model_key == "Y"`. PATCH về X → bản X còn nguyên. `models_done` chứa cả hai.
  - `current_model_key` không phải captioner → 422.
- [ ] **Step 3: Test** `voice_warning_flags`:
  - `vocals:"Instrumental"` và caption không có từ giọng → false.
  - Caption "soft hummed vocal line" → true.
  - Caption "humanized drums" → false (khớp nguyên từ).
- [ ] **Step 4: Test** `member_cannot_edit_catalog`: alice PATCH → 403.
- [ ] **Step 5:** FAIL → viết code → `cargo test` đạt.
- [ ] **Step 6:** Commit `feat(api): manual captions with undo/redo and track edits`.

### Task 5: Job `caption_tracks` và kết quả từng bài

**Files:** `routes/catalogs.rs`, `routes/worker.rs`, `lib.rs` (route worker mới), `tests/api.rs`, hai file hợp đồng (`WORKER_PROTOCOL.md`: mục "Job `caption_tracks`" và endpoint `POST /jobs/{id}/captions`)

**Interfaces:**
- Produces:
  - `POST /catalogs/{id}/caption-jobs {model_key, track_ids: string[], force?: bool}` → 200 `{job_id: string|null, queued: string[], skipped: string[]}`. Không phải 201, vì có thể không tạo job.
    - `model_key` phải là captioner `active` **có cài trên ít nhất một node**, không thì 422 `"Model chưa cài trên máy render"`.
    - Bỏ qua (khi `force` không bật) bài đã có head cho model đó, **và** bài đang nằm trong một job `caption_tracks` cùng model đang queued/preparing/running.
    - Bài không thuộc catalog hoặc đã xóa → 422.
    - `queued` rỗng → `job_id: null`, không tạo job.
    - Job: `type='caption_tracks'`, `model_id` = id model (để claim kiểm `model_installations`), `priority -20`, `max_attempts 2`, `title = "Mô tả N bài · <model name>"`.
    - Payload:
      ```json
      {"catalog_id": "…", "model": {"id": "…", "key": "…"},
       "tracks": [{"catalog_track_id": "…", "input": "t0"}],
       "inputs": {"t0": {"artifact_id": "…", "filename": "…", "size_bytes": 1, "sha256": "…"}}}
      ```
    - Khi tạo job, đặt `current_model_key = model_key` cho các bài trong `queued`. Đây là lựa chọn của tôi: bấm tạo bằng model nào thì cột Tech stack hiện model đó.
  - Worker: `POST /api/worker/v1/jobs/{id}/captions` với body `{lease_token, catalog_track_id, mood, instruments, vocals, caption, source_ref}` → 200 `{revision_seq}`.
    - Kiểm lease như `progress`. `catalog_track_id` phải có trong `payload.tracks`, không thì 404.
    - Insert lượt `source='model'`, `job_id` = job, `created_by` = owner job. Head trỏ tới lượt mới.
    - Gửi lại cùng (job, bài) → trả lại `revision_seq` cũ, không thêm lượt (index `caption_revisions_job_track`).
    - Thêm `{lease_token, catalog_track_id, error}` → chỉ ghi vào log job (`message`), không tạo lượt. Body có `error` thì là báo lỗi.
  - `complete` cho `caption_tracks`: không có xử lý riêng, chỉ lưu `result` (`{done: n, failed: [ids]}`).
  - `GET tracks`: `captioning_model_key` = `json_extract(j.payload,'$.model.key')` của job `caption_tracks` đang queued/preparing/running có chứa bài (`json_each(j.payload,'$.tracks')`) **mà bài chưa có lượt nào với job_id đó**. Bài đã có kết quả thì không còn hiện "Đang tạo".
  - Claim: không cần đổi SQL. Job type đã lọc theo `job_types`, model theo `model_installations`.

- [ ] **Step 1: Test** `caption_job_skips_done_tracks`:
  - 3 bài, bài 1 đã có head cho `moss-music-8b-instruct`.
  - POST với cả 3 → `queued` = [2, 3], `skipped` = [1], payload có 2 tracks và 2 inputs.
  - POST lại ngay → `skipped` = [2, 3] (đang chờ), `job_id` null.
  - `force: true` với [1] → tạo job.
- [ ] **Step 2: Test** `caption_results_per_track_idempotent`:
  - Node có captioner claim job, POST kết quả bài 2 hai lần → một lượt duy nhất.
  - `GET tracks`: bài 2 có caption và `captioning_model_key` null; bài 3 vẫn `captioning_model_key == "moss-music-8b-instruct"`.
  - Bài không thuộc job → 404. Lease sai → 409.
- [ ] **Step 3: Test** `caption_job_cancel_keeps_finished_tracks`: sau khi bài 2 có kết quả, cancel job và worker xác nhận → bài 2 còn caption, bài 3 có `captioning_model_key` null.
- [ ] **Step 4: Test** `caption_job_requires_installed_model`: model captioner đã đăng ký rồi node register lại không có nó → 422.
- [ ] **Step 5:** FAIL → viết code → `cargo test`, clippy đạt.
- [ ] **Step 6:** Commit `feat(api): caption_tracks job with per-track results`.

### Task 6: Worker đo Tempo/Key (`analyze_track`)

**Files:**
- Create: `workers/music-worker/src/lush_worker/analysis.py`, `tests/test_analysis.py`
- Modify: `runner.py` (dispatch + job_types), `pyproject.toml` (extra `analysis = ["librosa>=0.10", "soundfile>=0.12", "pyloudnorm>=0.1"]`)

**Interfaces:**
- Consumes: payload Task 3.
- Produces:
  - `estimate_tempo(mono: np.ndarray, rate: int, min_conf: float = 0.3) -> dict`, trả `{"bpm": float|None, "confidence": float, "alternatives": [float, float]}`.
  - `estimate_key(mono, rate, min_conf=0.05, min_corr=0.6) -> dict`, trả `{"key": str|None, "confidence": float}`.
  - Hai hàm trên sao chép thuật toán từ `lush-training/src/lush_training/analysis.py:47,65`: librosa `beat_track` + onset strength; Krumhansl-Schmuckler trên `chroma_cqt`. Ngưỡng giữ nguyên.
  - `analyze_file(path: Path) -> dict`, trả đúng schema kết quả của Task 3. Giải mã bằng librosa `load(sr=22050, mono=True)`. Duration lấy từ `probe_duration` có sẵn. LUFS dùng pyloudnorm nếu có, không có thì `None`. `clipping` = có mẫu `abs >= 0.999`. `silence_ratio` = tỉ lệ khung RMS < -60 dBFS.
  - `analysis_available() -> bool`: import được librosa.
  - Runner chỉ thêm `"analyze_track"` vào `job_types` khi `analysis_available()`. `_execute` với type `analyze_track` gọi `_fetch_inputs` rồi `analyze_file(inputs["track"]["path"])` rồi `api.complete(job_id, lease, result)`.

- [ ] **Step 1: Test** (`pytest.importorskip("librosa")`):
  - `test_tempo_of_click_track`: click 120 BPM dài 20 s tạo bằng numpy → `bpm` trong 118–122 hoặc trong `alternatives`.
  - `test_key_of_d_minor_chord_loop`: chuỗi hợp âm Dm–Gm–A tổng hợp → `key == "D minor"`, hoặc `None` khi dưới ngưỡng (test ghi rõ chấp nhận cả hai, assert không trả key sai khác).
  - `test_silence_returns_unknown`: im lặng → `bpm is None`, `key is None`, `silence_ratio > 0.9`.
- [ ] **Step 2: Test** `test_runner_analyze_track_completes` (respx, không cần librosa: monkeypatch `analyze_file`): claim payload `analyze_track` → tải input, kiểm sha, gọi `complete` với kết quả. Kèm `test_registration_lists_analyze_track_only_when_available`.
- [ ] **Step 3:** `.venv\Scripts\python -m pip install -e ".[dev,analysis]"` rồi `pytest tests/test_analysis.py tests/test_runner.py -q` → FAIL.
- [ ] **Step 4:** Viết code.
- [ ] **Step 5:** `pytest -q` toàn bộ đạt.
- [ ] **Step 6:** Commit `feat(worker): analyze_track job (tempo, key, loudness)`.

### Task 7: Worker tạo mô tả (`caption_tracks`)

**Files:**
- Create: `src/lush_worker/captioning.py`, `tests/test_captioning.py`, `tests/fake_captioner.py`
- Modify: `runner.py`, `config.py` (`CAPTIONER_DIR`), `api_client.py` (`post_caption`)

**Interfaces:**
- Consumes: payload và endpoint Task 5.
- Produces:
  - **Thư mục cấu hình:** `CAPTIONER_DIR/<tên>/captioner.json` có dạng `{"key","name","version","python","script","mode": "json"|"text","prompt"?: str}`. Đường dẫn `python` và `script` là tuyệt đối hoặc tương đối với thư mục đó.
  - `discover_captioners(root: Path) -> list[CaptionerSpec]`: bỏ qua file hỏng và ghi log cảnh báo. Mỗi spec báo khi register như model `{engine:"captioner", key, name, version, capabilities:["caption"]}`. `"caption_tracks"` chỉ có trong `job_types` khi có ít nhất một spec.
  - **Giao thức tiến trình con** (tài liệu hóa trong `WORKER_PROTOCOL.md`, mục "Captioner subprocess"):
    - Worker khởi động `python script` **một lần mỗi job**.
    - Mỗi bài, worker gửi một dòng JSON vào stdin: `{"id": "<track_id>", "audio": "<path>", "prompt": "<prompt|null>"}`.
    - Script trả một dòng JSON trên stdout: `{"id": "...", "text": "..."}` hoặc `{"id": "...", "error": "..."}`.
    - Kết thúc bằng việc đóng stdin. Mọi log của script đi stderr.
    - Timeout mỗi bài 600 s.
  - `parse_output(text: str, mode: str) -> CaptionFields`, với `CaptionFields = {mood, instruments, vocals, caption}`:
    - `mode="json"`: tìm object JSON đầu tiên trong text, lấy 4 khóa.
    - `mode="text"`: caption = text.strip(), ba ô kia từ `extract_fields`.
    - Trả `None` khi JSON hỏng hoặc rỗng.
  - `extract_fields(caption: str) -> dict`: so khớp với `INSTRUMENT_TERMS` và `MOOD_TERMS` (danh sách tiếng Anh cố định trong file, mỗi danh sách khoảng 40 từ, nối bằng ", " theo thứ tự xuất hiện). `vocals` = `"Instrumental"` nếu caption có "instrumental" và không có từ giọng; `"Vocals"` nếu có từ giọng (`VOICE_TERMS` y như API Task 4); còn lại là `""`.
  - **Prompt mặc định cho `mode="json"`:**
    ```
    Listen to this track and answer with JSON only: {"mood": "...", "instruments": "...", "vocals": "Instrumental or describe any voice", "caption": "one English paragraph under 80 words describing genre, instruments, mood and texture; do not state BPM or key"}
    ```
  - `run_caption_job(ctx, api, spec, payload) -> dict`, trả `{"done": n, "failed": [ids]}`:
    - Lần lượt từng bài. JSON hỏng thì gửi lại đúng bài đó **một lần**. Vẫn hỏng thì dùng nguyên văn làm caption (nếu không rỗng), rỗng thì ghi failed.
    - Mỗi bài xong gọi `api.post_caption(job_id, lease, track_id, fields, source_ref=f"{key}@{version}")`. Lỗi gọi `api.post_caption(..., error=msg)`.
    - Gọi `ctx.report` sau mỗi bài với `phase="caption"`.
    - `ctx.raise_if_aborted()` giữa các bài: hủy thì dừng tiến trình con.
    - Tiến trình con chết thì khởi động lại một lần rồi đi tiếp bài sau.
  - `ControlPlaneClient.post_caption(job_id, lease, track_id, fields: dict | None, source_ref: str = "", error: str | None = None) -> dict`.

- [ ] **Step 1: Test** với `tests/fake_captioner.py`. Đây là script Python thật, đọc stdin và trả theo tên file: `good` → JSON hợp lệ, `bad` → "not json", `empty` → "", `crash` → thoát code 1.
  - `test_parse_output_json_and_text_modes`.
  - `test_extract_fields_from_caption`: "A mellow lo-fi instrumental with warm Rhodes, upright bass and brushed drums, nostalgic" → instruments chứa "Rhodes", "bass", "drums"; mood chứa "mellow", "nostalgic"; vocals "Instrumental".
  - `test_run_caption_job_posts_each_track` (respx): 2 bài `good` → 2 lần POST captions, `complete` có `done == 2`.
  - `test_bad_json_retried_then_raw_text`: bài `bad` → fake nhận 2 lần, POST caption = "not json".
  - `test_empty_output_reports_error_and_continues`: bài `empty` rồi `good` → một POST error, một POST caption.
  - `test_crash_restarts_once`.
  - `test_discover_captioners_skips_broken_config`.
  - `test_registration_reports_captioner_models`.
- [ ] **Step 2:** `pytest tests/test_captioning.py -q` → FAIL.
- [ ] **Step 3:** Viết code. Đọc stdout bằng thread và queue để có timeout trên Windows lẫn Linux.
- [ ] **Step 4:** `pytest -q` toàn bộ đạt.
- [ ] **Step 5:** Commit `feat(worker): caption_tracks job via per-model subprocess`.

### Task 8: Web: API client, danh sách catalog, sidebar

**Files:**
- Modify: `apps/web/src/api/types.ts`, `api/api.ts`, `api/hooks.ts`, `api/mutations.ts`, `App.tsx`, `components/Sidebar.tsx:192`
- Create: `pages/CatalogsPage.tsx`, `pages/CatalogsPage.test.tsx`; thêm `makeCatalog`, `makeCatalogTrack` trong `test/utils.tsx`

**Interfaces:**
- Produces:
  - Types `Catalog`, `CatalogTrack`, `CaptionView`, `CaptionJobResult` (đúng field Task 2–5). `Model.kind: 'generator' | 'captioner' | 'scorer'`.
  - `api.catalogs.{list, create, get, update, remove, tracks, upload(catalogId, file), createCaptionJob(catalogId, body)}`.
  - `api.catalogTracks.{update, remove, saveCaption(id, modelKey, body), undo(id, modelKey), redo(id, modelKey)}`.
  - Hook `useCatalogs()`, `useCatalog(id)`, `useCatalogTracks(id)` có `refetchInterval` 2000 ms khi còn bài `analysis_status === 'pending'` hoặc `captioning_model_key !== null`, còn lại `false`. `useCaptioners()` = `useModels()` lọc `kind === 'captioner' && status === 'active'`.
  - Route `/datasets` → `CatalogsPage`, route `/catalogs/:id` → `CatalogPage` (Task 9). Sidebar: nhãn **"Catalog"**, bỏ badge "Sắp có", chỉ hiện với `admin` / `trainer`.
  - `CatalogsPage`:
    - Bảng gồm Tên, Genre, Số bài, Tổng phút.
    - Nút **Tạo catalog** mở form (tên + chọn genre từ `useGenres`).
    - `member` thấy `Notice` "Chỉ admin và trainer dùng được Catalog".
- [ ] **Step 1: Test** `CatalogsPage.test.tsx`:
  - Liệt kê catalog từ `mockFetch`.
  - Tạo catalog gửi `POST /catalogs` đúng body rồi điều hướng tới `/catalogs/<id>`.
  - Lỗi 409 hiện "Tên catalog đã có".
  - Member thấy thông báo.
  - Thêm test sidebar: trainer thấy "Catalog", member không thấy.
- [ ] **Step 2:** `npx vitest run src/pages/CatalogsPage.test.tsx` → FAIL.
- [ ] **Step 3:** Viết code.
- [ ] **Step 4:** `npm test` đạt, `npm run build` sạch.
- [ ] **Step 5:** Commit `feat(web): catalog list page and API client`.

### Task 9: Web: trang Catalog (bảng, tải lên, chọn, tạo mô tả)

**Files:**
- Create: `pages/CatalogPage.tsx`, `features/catalog/CatalogTable.tsx`, `features/catalog/SelectionBar.tsx`, `features/catalog/UploadButton.tsx`, và test tương ứng
- Modify: `styles/components.css` (lớp `.catalog-*`)

**Interfaces:**
- Consumes: hook và api Task 8.
- Produces: giao diện theo `library-v3.png`.
  - **Đầu trang:**
    - Tên catalog và pill "Genre: <tên>".
    - Nút **⬆ Tải nhạc lên**: input nhiều file, kiểm bằng `checkAudioFile` có sẵn. Mỗi file một request, chạy lần lượt. Hiện "Đang tải 3/10". Lỗi 409 hiện "<file>: đã có trong catalog (<tên cũ>)" và tiếp tục file sau.
  - **Thanh chọn** (khi có bài được tick):
    - "Đã chọn N bài", **Chọn tất cả**, ô số **Chọn N bài đầu**.
    - Menu **Model tạo mô tả** (từ `useCaptioners`, chỉ tên). Không có model nào thì menu mờ, ghi "Chưa cài model mô tả trên máy render".
    - Nút **Tạo mô tả cho N bài** gọi `createCaptionJob({model_key, track_ids})`. Xong thì báo "Đã xếp hàng X bài, bỏ qua Y bài đã có mô tả của model này", và nếu xếp hàng > 0 thì kèm "Trong lúc tạo mô tả, nhạc mới tạo sẽ phải chờ".
    - Nút **Xóa khỏi catalog** (có xác nhận).
  - **Bảng:** cột chọn | Bài (tên, `<audio controls preload="none" src={artifactUrl(artifact_id)}>`, thời lượng, "train 4 phút đầu" khi `duration_s > 240`) | Tech stack (Task 10) | Tempo | Key | Mood | Instruments | Vocals | Music caption | Trạng thái.
    - Tempo/Key hiện giá trị hoặc ô trống viền đứt. Dưới ô là dòng xanh "Máy: 78 (tin cậy cao|khá|thấp)", với ngưỡng confidence ≥ 0.6 cao, ≥ 0.3 khá, còn lại thấp. Tempo có `bpm` null thì hiện "Máy: 82 hoặc 164" từ `alternatives`. Key null thì hiện "Máy: không đủ căn cứ".
    - **Trạng thái:**
      - `analysis_status pending` → "Đang đo".
      - `captioning_model_key` → "Đang tạo mô tả…".
      - Có `caption` → "Có mô tả".
      - Còn lại → "Mới tải lên".
      - `analysis_status failed` → "Lỗi đo".
    - `voice_warning` → pill cảnh báo "Có thể có giọng".
- [ ] **Step 1: Test** `CatalogPage.test.tsx`:
  - Hiện 4 bài với đúng trạng thái như mockup.
  - Tick 2 bài, chọn model, bấm tạo → body đúng, hiện thông báo bỏ qua.
  - "Chọn N bài đầu" với 3 → tick 3 bài đầu.
  - Upload 2 file, file thứ nhất 409 → hiện lỗi file đó, vẫn tải file thứ hai.
  - Không có captioner → menu mờ.
  - Dòng chữ máy đo hiển thị đúng 3 trường hợp.
- [ ] **Step 2:** FAIL → viết code → `npm test`, build đạt.
- [ ] **Step 3:** Commit `feat(web): catalog page with upload, selection and caption jobs`.

### Task 10: Web: cột Tech stack, sửa ô, Quay về / Tới; ẩn model mô tả ở form Tạo nhạc

**Files:**
- Create: `features/catalog/TechStackCell.tsx`, `features/catalog/EditableCell.tsx`, và test
- Modify: `features/create/GenerateForm.tsx`, `pages/admin/StylesPage.tsx`, `features/training/TrainingRunForm.tsx` (lọc `kind === 'generator'`)

**Interfaces:**
- Produces:
  - **`TechStackCell({track, captioners})`:**
    - Menu chọn model, gọi `catalogTracks.update({current_model_key})`.
    - Mỗi model trong menu có dấu chấm xanh nếu nằm trong `models_done` (W8).
    - Nút dưới menu: **Tạo** (model hiện tại chưa có kết quả) hoặc **Chạy lại** (đã có). Cả hai gọi `createCaptionJob({model_key, track_ids:[id], force:true})`.
    - **Chạy lại** cảnh báo bằng `window.confirm`: "Chạy lại sẽ thay bản hiện tại, kể cả phần bạn đã sửa. Bấm Quay về để lấy lại."
    - **Quay về** (khi `caption.can_undo`) và **Tới** (khi `can_redo`).
  - **`EditableCell({value, onSave})`:** bấm vào để sửa, Enter hoặc blur thì lưu, Esc thì hủy.
    - Mood, Instruments, Vocals, Caption lưu bằng `saveCaption(id, current_model_key, {…đủ 4 ô, đổi 1 ô})`.
    - Tempo/Key lưu bằng `catalogTracks.update`.
    - Chưa có `current_model_key` thì ô mô tả không sửa được, tooltip "Chọn model trước".
- [ ] **Step 1: Test**:
  - Đổi model sang model chưa chạy → gọi PATCH, sau refetch thì các ô trống và nút đổi thành "Tạo".
  - Chấm hiện cho model trong `models_done`.
  - Chạy lại hiện confirm, bấm hủy thì không gọi API.
  - Quay về gọi `/undo`.
  - Sửa ô Mood và Enter → PUT với đủ 4 trường.
  - `GenerateForm` không liệt kê model `kind: 'captioner'`.
- [ ] **Step 2:** FAIL → viết code → `npm test`, `npm run build`, lint đạt.
- [ ] **Step 3:** Commit `feat(web): tech stack column, inline edits and undo/redo`.

### Task 11: Script captioner MOSS-Music (chưa chạy, chờ duyệt tải model)

**Files:**
- Create: `workers/music-worker/captioners/moss_music/run.py`, `captioner.json.example`, `README.md`
- Docs: `docs/RENDER_NODE_DEPLOYMENT.md` (mục "Cài model mô tả")

**Interfaces:**
- Consumes: giao thức tiến trình con ở Task 7.
- Produces:
  - `run.py`:
    - Nạp MOSS-Music-8B-Instruct bằng transformers theo `infer.py` của tác giả.
    - Đọc từng dòng stdin, chạy `prompt` với audio, ghi `{"id","text"}` ra stdout.
    - Model được nạp một lần khi khởi động.
  - `captioner.json.example`: `{"key": "moss-music-8b-instruct", "name": "MOSS-Music 8B", "version": "<hf revision>", "python": "/media/ai-rtx6000/AI_DATA/AiStudio/models/moss-music/.venv/bin/python", "script": "run.py", "mode": "json"}`.
  - README: lệnh tạo venv, tải model (**chỉ chạy sau khi người dùng duyệt**), cách tự kiểm bằng `echo '{"id":"x","audio":"/path.mp3","prompt":null}' | python run.py`.
- [ ] **Step 1:** Trước khi viết, đọc model card và `infer.py` trên Hugging Face/GitHub của OpenMOSS để lấy đúng tên class, processor và cách đưa audio vào. Ghi revision đã đọc vào README.
- [ ] **Step 2:** Viết `run.py`. Phần không cần model (đọc/ghi dòng, bắt lỗi từng bài để trả `{"id","error"}`) có test `tests/test_moss_runner_protocol.py`: monkeypatch hàm `generate`, không import torch.
- [ ] **Step 3:** `pytest -q` đạt.
- [ ] **Step 4:** Commit `feat(worker): MOSS-Music captioner script (not yet run on GPU)`.
- **Ghi chú trung thực:** task này xong khi test giao thức đạt. Việc chạy thật với GPU nằm ngoài plan và cần người dùng duyệt tải model.

### Task 12: Tài liệu dự án

**Files:** `docs/tasks/CURRENT.md` (T-015), `docs/DECISIONS.md` (D31), `docs/CHANGELOG.md`, `README.md` (biến `CAPTIONER_DIR`, extra `analysis`)

- [ ] **Step 1:** CURRENT.md thêm **T-015 LoRA Studio P1**, gồm:
  - Đã làm.
  - Đã xác minh (chỉ test tự động, mock).
  - Chưa xác minh: chạy thật trên rtx6000, librosa trên node, MOSS thật.
  - Bước tiếp theo cần duyệt: deploy (migration `0006`), cài extra `analysis` trên node, tải MOSS.
  - Ghi rõ T-014 CLI factory bị thay ở phần giao diện.
- [ ] **Step 2:** D31: "Catalog trên website thay trang duyệt localhost; model mô tả là Model Registry `kind=captioner`, chạy tiến trình con trong venv riêng; dùng cả model phi thương mại cho thử nghiệm, lưu nguồn từng mô tả". Ghi lý do và cái giá.
- [ ] **Step 3:** Chạy lại cả ba bộ test:
  - `cargo test` và `cargo clippy --all-targets` trong `apps/api`.
  - `npm test` và `npm run build` trong `apps/web`.
  - `.venv\Scripts\python -m pytest -q` trong `workers/music-worker`.
  - Ghi số đạt vào CURRENT.md.
- [ ] **Step 4:** Commit `docs: T-015 LoRA Studio P1 status and D31`.
