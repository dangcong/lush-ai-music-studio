# LoRA Studio P2 – Tạo LoRA từ Catalog: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Người dùng chọn bài trong Catalog, bấm **Tạo LoRA**, điền hộp thoại, bấm **Bắt đầu train**. API chụp lại một bản dataset bất biến, và job `train_lora` trên rtx6000 train từ bản chụp đó, lưu mọi bản lưu (checkpoint) vào Registry.

**Architecture:**
- **API:** module mới `routes/lora_builds.rs`. Một hàm `plan_build` dùng chung cho hai việc:
  - `POST /catalogs/{id}/lora-preview`: chỉ kiểm tra, không ghi gì.
  - `POST /catalogs/{id}/loras`: ghi `dataset_snapshots` và job `train_lora` trong cùng một transaction.
- **Payload job:** mang theo `samples` + `inputs` (artifact của bài). Worker tải về, đổi sang WAV 48 kHz, tự dựng `data/datasets/<lora_key>/dataset.json` theo định dạng Side-Step, rồi chạy các bước train như hiện nay.
- **Kết quả trả về:** thêm danh sách checkpoint. API ghi chúng vào bảng mới `lora_checkpoints`.
- **Web:** hộp `CreateLoraDialog` trên CatalogPage gọi preview mỗi khi đổi tùy chọn. TrainingRunPage hiện thêm "Train từ" và danh sách bản lưu.

**Tech Stack:** Rust (Axum, SQLx runtime query, SQLite), Python 3 worker (pytest, respx), React 19 + TypeScript + React Query (vitest).

**Spec:** `docs/superpowers/specs/2026-11-23-lora-studio-web-design.md`. Các mục P2 cần làm: 6.3, 6.5 (dòng `train_lora`), 6.6 (bản lưu, luồng train cũ), 6.7, 9.2. Mockup: `.superpowers/brainstorm/1672-1795426550/content/create-lora.png` (git-ignored).

## Global Constraints

- Tiếng Việt cho mọi chữ trên giao diện và thông báo lỗi; code, tên API, config key bằng tiếng Anh.
- Chuyển trạng thái job là một câu SQL nguyên tử. SQL viết runtime (`sqlx::query`), không dùng macro. Schema mới = migration mới `0007_lora_builds.sql`, không sửa migration cũ.
- API công khai chỉ lộ Artifact ID, không lộ đường dẫn file. `lora_checkpoints.node_path` không bao giờ trả ra web.
- Model và genre là dữ liệu, không hardcode danh sách.
- Mỗi đổi hành vi phải sửa `docs/API_CONTRACTS.md` hoặc `docs/WORKER_PROTOCOL.md` trong cùng commit.
- Worker không đụng database. Mọi việc audio chạy trên node; VPS không xử lý audio.
- Độ dài tối đa mỗi bài: 240 s, cố định (U7). Base model là model generator `acestep` đang active; web chọn sẵn ACE-Step 1.5 turbo nếu có.
- Không tải model, không chạy GPU thật, không deploy, không push trong plan này. Test chỉ dùng mock hoặc fake.
- Giá trị mặc định của hộp Tạo LoRA (spec mục 3 và W11):
  - rank 32, alpha 64, learning_rate 0.0001, epochs 4, gradient_accumulation 4, **save_every 1**, warmup_steps 2, max_minutes 30.
  - **Giữ lại 2 bài**.
  - Kiểu train: Thử nhanh = 1 epoch, Chuẩn = 4, Kỹ = 8, Tự nhập.
- Quyền:
  - Preview, tạo LoRA, xem training run và xem bản chụp: `admin` và `trainer`, dùng extractor `Trainer`.
  - Tạo run kiểu cũ (`POST /training-runs`): vẫn chỉ `admin`.

## Review Focus

1. **Bài bị xóa khỏi catalog giữa lúc xem trước và lúc bấm train, hoặc ID bài thuộc catalog khác.** Create phải trả 422 "Có bài không còn trong catalog này" và không ghi gì. Test thuộc Task 2.
2. **Sửa mô tả hoặc Tempo của bài sau khi đã tạo LoRA.** Bản chụp và payload job giữ nguyên. Câu `UPDATE` trực tiếp vào `content` bị trigger chặn. Test thuộc Task 2.
3. **Bấm "Bắt đầu train" hai lần.** Lần hai trả 409 "Đang có một training run khác chưa kết thúc", chỉ có một bản chụp. Test thuộc Task 2.
4. **Số bài giữ lại ≥ số bài đã chọn.** Preview báo `ready=false` với lỗi "Phải còn ít nhất 1 bài để train"; create trả 422. Test thuộc Task 1.
5. **Một file nhạc tải về sai SHA-256, hoặc máy không có ffmpeg.** Job lỗi **trước khi** dừng `acestep-api`, nên tạo nhạc vẫn chạy. Không để lại thư mục dataset dở. Test thuộc Task 4.

---

## File Structure

| File | Trách nhiệm |
|---|---|
| `apps/api/migrations/0007_lora_builds.sql` (tạo mới) | Bảng `dataset_snapshots` (có trigger chặn sửa), `lora_checkpoints`, cột `loras.snapshot_id` |
| `apps/api/src/routes/lora_builds.rs` (tạo mới) | `BuildRequest`, `plan_build`, preview, create, `get_snapshot` |
| `apps/api/src/routes/catalogs.rs` | Đổi `load_tracks`/`Scope` thành `pub(crate)` để dùng lại. Không đổi gì khác |
| `apps/api/src/routes/training.rs` | Tách `slug`, `validate_config`, kiểm "đang có run" và kiểm "có node train" ra hàm `pub(crate)` dùng chung. GET đổi sang `Trainer`. Chi tiết run thêm `snapshot` và `checkpoints` |
| `apps/api/src/routes/worker.rs` | Điều kiện claim cho `train_lora` có bản chụp; lúc complete thì ghi `snapshot_id` và checkpoint |
| `apps/api/src/lib.rs` | Route mới |
| `workers/music-worker/src/lush_worker/audio.py` | `to_wav48k` |
| `workers/music-worker/src/lush_worker/training.py` | `build_snapshot_dataset`, `run` nhận payload có `samples`, kết quả checkpoint mở rộng |
| `workers/music-worker/src/lush_worker/runner.py` | `_execute_training` tải inputs khi payload có `samples` |
| `apps/web/src/api/{types,client,hooks,mutations}.ts` | Kiểu và hook cho preview, create, snapshot; chi tiết run có thêm trường |
| `apps/web/src/features/lora/createLoraForm.ts` (tạo mới) | State, kiểm lỗi, dựng request và các phép tính dự kiến. Hàm thuần, có test |
| `apps/web/src/features/lora/CreateLoraDialog.tsx` (tạo mới) | Hộp thoại 3 nhóm và cột phải |
| `apps/web/src/pages/CatalogPage.tsx` | Nút "Tạo LoRA (N bài đã chọn)" |
| `apps/web/src/pages/TrainingRunPage.tsx`, `TrainingPage.tsx` | Phần "Train từ" và danh sách bản lưu; luồng cũ thu gọn |
| `docs/API_CONTRACTS.md`, `docs/WORKER_PROTOCOL.md` | Hợp đồng, sửa cùng task đổi hành vi |
| `docs/tasks/CURRENT.md`, `docs/DECISIONS.md`, `docs/CHANGELOG.md` | Task 9 |

## Các quyết định của tôi (ruling khi viết plan, cần người dùng xem)

- **P1. Kiểm tra qua preview.**
  - Mọi kiểm tra trước train nằm ở `plan_build` phía API. Web chỉ hiện kết quả preview, nên web và API không lệch nhau.
  - Bài giữ lại được chọn bằng `holdout_seed`. Web sinh seed một lần khi mở hộp thoại, nên preview và create chọn cùng bài.
- **P2. Giới hạn số bài giữ lại.**
  - Từ 0 đến 5, và phải còn ít nhất 1 bài để train.
  - Spec không nêu trần; 5 bài giữ eval ở mức (bản lưu × 6 + 1) bài render.
- **P3. Từ khóa gọi style.**
  - Có thể để trống. Nếu điền thì 1–40 ký tự gồm chữ, số, `_`, `-`, không có dấu phẩy hay khoảng trắng.
  - Khi train, từ khóa được ghép vào caption thành `"<từ khóa>, <caption>"`. `custom_tag` trong `dataset.json` để trống, để không ghép hai lần.
- **P4. Câu lệnh nghe thử.**
  - Bắt buộc, 1–500 ký tự.
  - Web điền sẵn `"<từ khóa>, <tên genre>"`, hoặc chỉ `"<tên genre>"` khi không có từ khóa.
  - P2 chỉ lưu câu này vào bản chụp; P3 mới dùng.
- **P5. Tên LoRA.**
  - Dài 1–80 ký tự.
  - Không được trùng (không phân biệt hoa thường) với `loras.name` có status khác `archived`, hoặc với tiêu đề của job `train_lora` đang `queued`/`preparing`/`running`.
- **P6. Bản lưu dự kiến.** Là các epoch chia hết cho `save_every`, cộng thêm epoch cuối nếu epoch đó chưa có trong danh sách. Worker cũng báo như vậy: khi epoch cuối chưa có checkpoint, worker thêm `out/final` làm bản của epoch cuối.
- **P7. Số bài nghe thử dự kiến.** `bản lưu × (1 + bài giữ lại) + 1`. Chỉ hiện con số, P3 mới render.
- **P8. Thời gian dự kiến.**
  - Lấy run `train_lora` hoàn tất gần nhất có `result.train_seconds` và `result.optimizer_steps > 0`, tính giây mỗi bước rồi nhân với tổng số bước.
  - Không có run nào như vậy thì hiện "—".
- **P9. Lỗi chặn và cảnh báo.** Các trường hợp sau chặn bấm train (create trả 422):
  - Bài chưa có mô tả ở model đang hiện.
  - Bài có cảnh báo giọng người mà chưa xác nhận.
  - Tên trùng.
  - Không còn bài nào để train.

  Thiếu Tempo hoặc Key chỉ là cảnh báo. Khi đó `dataset.json` bỏ trống trường đó, để upstream tự ghi `N/A`.
- **P10. Bản chụp trong job.**
  - Bản chụp nằm trong `dataset_snapshots.content` (JSON) và `payload.samples` của job.
  - Worker tự dựng `dataset.json`, vì định dạng Side-Step thuộc trainer adapter.
  - Dataset key là `lora_key` một phần, nên `list_datasets` (chỉ quét `*/*`) không báo nó lên như dataset kiểu cũ.
- **P11. Luồng train cũ.**
  - Trang `/training` có nút chính "Tạo LoRA từ Catalog" dẫn tới `/catalogs`.
  - Form cũ thu vào `<details>` "Train từ dataset có sẵn trên máy (cách cũ)", chỉ hiện cho admin.
  - Trainer xem được danh sách và chi tiết run.
- **P12. Checkpoint ở P2.**
  - P2 chỉ ghi danh sách checkpoint: epoch, bước, SHA-256, dung lượng, đường dẫn trên node.
  - Trạng thái `kept`, chưa có hạn xóa.
  - Chọn bản, xóa bản và hạn 30 ngày thuộc P3.

---

### Task 1: API – `plan_build` và preview

**Files:**
- Create: `apps/api/src/routes/lora_builds.rs`
- Modify: `apps/api/src/routes/mod.rs`, `apps/api/src/lib.rs`, `apps/api/src/routes/catalogs.rs` (chỉ đổi `load_tracks`, `Scope`, `CatalogTrack` sang `pub(crate)`), `apps/api/src/routes/training.rs` (tách các helper `pub(crate)`)
- Modify: `docs/API_CONTRACTS.md` (mục Catalogs 🎓: `POST /catalogs/{id}/lora-preview`)
- Test: `apps/api/tests/api.rs`

**Interfaces:**
- Consumes:
  - `catalogs::load_tracks(db, Scope::Catalog(id)) -> Vec<CatalogTrack>`. Mỗi bài có `caption: Option<CaptionView>` là bản ở `current_model_key`, kèm `voice_warning`, `duration_s`, `tempo_bpm`, `key_name`, `filename`, `artifact_id`.
  - Bảng `artifacts` (size_bytes, sha256) như P1 dùng trong `TrackAudio`.
- Produces:
  - `training::validate_config(rank, alpha, learning_rate, epochs, gradient_accumulation, save_every, warmup_steps, max_minutes) -> AppResult<()>`: dùng đúng các khoảng hiện có.
  - `training::slug(&str) -> String` (đổi thành `pub(crate)`).
  - `training::ensure_no_active_run(db) -> AppResult<()>`: 409.
  - `training::ensure_training_node(db, dataset_key: Option<&str>) -> AppResult<()>`: `None` nghĩa là không đòi node có dataset.
  - Trong `lora_builds.rs`:

```rust
#[derive(Deserialize, Serialize, Clone)]
pub struct BuildRequest {
    pub name: String,
    #[serde(default)] pub keyword: String,
    pub test_prompt: String,
    pub track_ids: Vec<String>,
    pub holdout_count: i64,
    pub holdout_seed: i64,
    #[serde(default)] pub voice_confirmed: Vec<String>, // track ids
    pub base_model_id: String,
    pub rank: i64, pub alpha: i64, pub learning_rate: f64, pub epochs: i64,
    pub gradient_accumulation: i64, pub save_every: i64, pub warmup_steps: i64,
    #[serde(default)] pub seed: Option<i64>,
    pub max_minutes: i64,
}

#[derive(Serialize)]
pub struct BuildTrack { pub id: String, pub filename: String, pub duration_s: Option<f64>,
    pub model_key: Option<String>, pub vocals: String, pub caption: String }

#[derive(Serialize)]
pub struct ModelCount { pub model_key: String, pub name: String, pub count: i64 }

#[derive(Serialize)]
pub struct BuildPlan {
    pub ready: bool,
    pub errors: Vec<String>,            // chặn train, theo thứ tự hiển thị
    pub warnings: Vec<String>,
    pub train: Vec<BuildTrack>,
    pub held_out: Vec<BuildTrack>,
    pub missing_caption: Vec<BuildTrack>,
    pub voice_unconfirmed: Vec<BuildTrack>,
    pub no_tempo_key: Vec<BuildTrack>,
    pub train_seconds: f64,             // tổng min(duration, 240) của bài train có duration
    pub unknown_duration: i64,          // số bài train chưa có duration
    pub models: Vec<ModelCount>,        // bài train theo model mô tả, nhiều nhất trước
    pub steps_per_epoch: i64,
    pub total_steps: i64,
    pub checkpoint_epochs: Vec<i64>,    // P6
    pub eval_renders: i64,              // P7
    pub name_taken: bool,
}

pub(crate) async fn plan_build(db: &SqlitePool, catalog_id: &str, req: &BuildRequest) -> AppResult<BuildPlan>;
pub async fn preview_build(State, Trainer, Path(id), Json(BuildRequest)) -> AppResult<Json<BuildPlan>>;
```

- [ ] **Step 1: Viết test lỗi**

Thêm `async fn lora_preview_reports_checks()` vào `tests/api.rs`. Chuẩn bị:
- Catalog từ `lofi_catalog` có 4 bài: dùng `three_tracks` rồi upload thêm một bài.
- Model generator `acestep` active (theo `training_body`).
- Bài 1 và 2 có caption model A (`put_caption` với `caption_body("warm rhodes")`), đặt `current_model_key` = A.
- Bài 3 có caption model B, nội dung `"soft humming over piano"`, nên có `voice_warning`. Đặt `current_model_key` = B.
- Bài 4 không có caption.

Request mẫu: `holdout_count: 1, holdout_seed: 7, epochs: 4, save_every: 1, gradient_accumulation: 4` và các giá trị mặc định khác. Assert:

```rust
let p = app.call("POST", &format!("/api/v1/catalogs/{cat}/lora-preview"), &carol, Some(body.clone())).await;
assert_eq!(p.status, 200);
assert_eq!(p.json["ready"], false);
assert_eq!(p.json["missing_caption"].as_array().unwrap().len(), 1);
assert_eq!(p.json["voice_unconfirmed"].as_array().unwrap().len(), 1);
assert_eq!(p.json["checkpoint_epochs"], json!([1, 2, 3, 4]));
// confirm voice for track 3, drop track 4 -> 3 tracks, 1 held out, 2 train
body["voice_confirmed"] = json!([t3]); body["track_ids"] = json!([t1, t2, t3]);
let p = app.call("POST", ..., Some(body.clone())).await;
assert_eq!(p.json["ready"], true);
assert_eq!(p.json["held_out"].as_array().unwrap().len(), 1);
assert_eq!(p.json["train"].as_array().unwrap().len(), 2);
assert_eq!(p.json["steps_per_epoch"], 1); assert_eq!(p.json["total_steps"], 4);
assert_eq!(p.json["eval_renders"], 4 * (1 + 1) + 1);
// same seed -> same held-out track
let again = app.call("POST", ..., Some(body.clone())).await;
assert_eq!(again.json["held_out"], p.json["held_out"]);
// Review Focus 4
body["holdout_count"] = json!(3);
let p = app.call("POST", ..., Some(body.clone())).await;
assert!(p.json["errors"].as_array().unwrap().iter().any(|e| e == "Phải còn ít nhất 1 bài để train"));
// models: train tracks counted per model, highest first
```

Thêm `async fn lora_preview_checkpoints_and_permissions()`:
- `epochs 5, save_every 2` cho ra `checkpoint_epochs == [2, 4, 5]`.
- Member gọi preview thì nhận 403.
- `holdout_count: 6` thì có lỗi `"Số bài giữ lại phải từ 0 đến 5"`.
- `keyword: "lush lofi"` (có khoảng trắng) thì có lỗi `"Từ khóa chỉ gồm chữ, số, _ hoặc -, tối đa 40 ký tự"`.
- `name` trùng tên một LoRA trong registry thì `name_taken == true` và có lỗi `"Tên LoRA đã có, hãy đặt tên khác"`. Chèn LoRA bằng SQL trực tiếp, hoặc chạy lại luồng `training_run_flow` đến bước complete.

- [ ] **Step 2: Chạy test, kiểm lỗi**

Run: `cd apps/api; cargo test --test api lora_preview`
Expected: FAIL, nhận 404 vì route chưa có.

- [ ] **Step 3: Cài đặt**

`plan_build` làm theo thứ tự sau:
1. Kiểm catalog còn sống, nếu không thì 404.
2. Kiểm config bằng `validate_config`. Đây là lỗi 422 ngay, không gom vào `errors`.
3. `track_ids` không được rỗng và không trùng nhau. Mọi ID phải là bài còn sống của đúng catalog này, nếu không thì 422 "Có bài không còn trong catalog này".
4. Phân loại các bài theo thứ tự hiển thị của `load_tracks`:
   - **`missing_caption`:** `caption` là None, hoặc caption chỉ có khoảng trắng.
   - **`voice_unconfirmed`:** có `voice_warning` và ID không nằm trong `voice_confirmed`.
   - **`no_tempo_key`:** thiếu `tempo_bpm` hoặc `key_name`.
5. Chọn bài giữ lại trong số bài đủ điều kiện (không thuộc `missing_caption` hay `voice_unconfirmed`):
   - Sắp các bài đủ điều kiện theo `id`.
   - Xáo bằng splitmix64 (`state = seed as u64`). Fisher–Yates từ cuối, `j = next() % (i+1)`.
   - Lấy `holdout_count` bài đầu. Phần còn lại là `train`, giữ thứ tự catalog.
6. Tên đã có nếu khớp một trong hai điều kiện ở P5: so `lower(trim(name))` với `lower(loras.name)` có `status != 'archived'`, hoặc với `lower(jobs.title)` của job `train_lora` đang sống.
7. Lỗi, theo đúng thứ tự và đúng chữ:
   - `"Tên LoRA phải có 1–80 ký tự"`
   - `"Tên LoRA đã có, hãy đặt tên khác"`
   - `"Từ khóa chỉ gồm chữ, số, _ hoặc -, tối đa 40 ký tự"`
   - `"Câu lệnh nghe thử phải có 1–500 ký tự"`
   - `"Số bài giữ lại phải từ 0 đến 5"`
   - `"{n} bài chưa có mô tả ở model đang hiện"`
   - `"{n} bài có thể có giọng người, cần bỏ hoặc xác nhận"`
   - `"Phải còn ít nhất 1 bài để train"`
8. Cảnh báo:
   - Khi thiếu Tempo/Key: `"{n} bài thiếu Tempo hoặc Key, sẽ ghi N/A khi train"`.
   - Luôn thêm: `"Trong lúc train, máy render tạm không tạo nhạc được"`.
9. `steps_per_epoch = ceil(train.len / gradient_accumulation)`. `ModelCount.name` lấy `models.name` theo key, không có thì dùng key.

Đăng ký route: `.route("/catalogs/{id}/lora-preview", post(lora_builds::preview_build))`.

- [ ] **Step 4: Chạy test, kiểm đạt**

Run: `cargo test --test api lora_preview`, rồi `cargo test` và `cargo clippy --all-targets`.
Expected: tất cả PASS, clippy không cảnh báo mới.

- [ ] **Step 5: Commit**

```bash
git add apps/api docs/API_CONTRACTS.md
git commit -m "feat(api): LoRA build preview from catalog selection"
```

---

### Task 2: API – Tạo LoRA: bản chụp, job, claim

**Files:**
- Create: `apps/api/migrations/0007_lora_builds.sql`
- Modify: `apps/api/src/routes/lora_builds.rs`, `apps/api/src/routes/worker.rs` (chỉ phần claim), `apps/api/src/lib.rs`
- Modify: `docs/API_CONTRACTS.md` (`POST /catalogs/{id}/loras`, `GET /dataset-snapshots/{id}`), `docs/WORKER_PROTOCOL.md` (payload `train_lora` dạng bản chụp)
- Test: `apps/api/tests/api.rs`

**Interfaces:**
- Consumes: `plan_build`, `BuildRequest` (Task 1); `training::{slug, ensure_no_active_run, ensure_training_node, JOB_TYPE}`.
- Produces:
  - Migration:

```sql
CREATE TABLE dataset_snapshots (
  id TEXT PRIMARY KEY, catalog_id TEXT NOT NULL REFERENCES catalogs(id),
  job_id TEXT NOT NULL UNIQUE REFERENCES jobs(id), lora_key TEXT NOT NULL UNIQUE,
  content TEXT NOT NULL, content_sha256 TEXT NOT NULL,
  created_by TEXT NOT NULL REFERENCES users(id), created_at TEXT NOT NULL);
CREATE TRIGGER dataset_snapshots_immutable BEFORE UPDATE ON dataset_snapshots
BEGIN SELECT RAISE(ABORT, 'dataset snapshot is immutable'); END;
ALTER TABLE loras ADD COLUMN snapshot_id TEXT REFERENCES dataset_snapshots(id);
CREATE TABLE lora_checkpoints (
  id TEXT PRIMARY KEY, lora_id TEXT NOT NULL REFERENCES loras(id),
  epoch INTEGER NOT NULL, step INTEGER, node_path TEXT NOT NULL, sha256 TEXT NOT NULL,
  bytes INTEGER, status TEXT NOT NULL DEFAULT 'kept' CHECK (status IN ('kept','chosen','deleted')),
  expires_at TEXT, note TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL,
  UNIQUE (lora_id, epoch));
```

  - `POST /catalogs/{id}/loras` (`Trainer`): body `BuildRequest`. Trả về 201 với `{"job_id", "snapshot_id", "lora_key"}`.
  - `GET /dataset-snapshots/{id}` (`Trainer`): trả `{id, catalog_id, job_id, lora_key, created_at, content}`.
  - `content` (schema 1):

```json
{"schema": 1, "catalog": {"id", "name"}, "genre": {"id", "name"},
 "lora": {"key", "name"}, "keyword": "", "test_prompt": "", "holdout_seed": 7,
 "base_model": {"id", "key"}, "config": {"rank", "alpha", "learning_rate", "epochs",
   "gradient_accumulation", "save_every", "warmup_steps", "seed", "max_duration_s": 240},
 "limits": {"max_seconds"},
 "samples":  [Sample...], "held_out": [Sample...],
 "models": [{"model_key", "name", "count"}]}
```

  `Sample` là `{catalog_track_id, artifact_id, filename, sha256, size_bytes, duration_s, model_key, caption_seq, caption_source, mood, instruments, vocals, caption, train_caption, bpm, keyscale, voice_confirmed}`. Trong đó:
  - `train_caption` là caption đã ghép từ khóa (P3).
  - `bpm` = `round(tempo_bpm)` hoặc null.
  - `keyscale` = `key_name` hoặc null.

  - Payload job `train_lora` dạng bản chụp. Giữ nguyên các key cũ `lora`, `base_model`, `trainer`, `config`, `limits`, và thêm:

```json
"dataset": {"key": "<lora_key>", "snapshot_id": "<id>"},
"genre": "<genre name>",
"samples": [{"input": "t0", "train_caption": "...", "bpm": 80, "keyscale": "A minor", "duration_s": 182.4}],
"inputs": {"t0": {"artifact_id", "filename", "size_bytes", "sha256"}}
```

  - Claim: điều kiện `train_lora` hiện tại đổi thành `NOT EXISTS (queued generate) AND (json_extract(j.payload,'$.dataset.snapshot_id') IS NOT NULL OR EXISTS (node_datasets ...))`.

- [ ] **Step 1: Viết test lỗi**

`async fn create_lora_from_catalog_snapshots_and_queues()`. Dùng catalog đã sẵn sàng như Task 1, với 3 bài, `holdout_count 1`, `keyword "lushlofi"`, và đăng ký node có `job_types` gồm `train_lora` nhưng **không** có dataset:

```rust
let r = app.call("POST", &format!("/api/v1/catalogs/{cat}/loras"), &carol, Some(body.clone())).await;
assert_eq!(r.status, 201);
let snap = app.call("GET", &format!("/api/v1/dataset-snapshots/{}", r.json["snapshot_id"].as_str().unwrap()), &carol, None).await;
let c = &snap.json["content"];
assert_eq!(c["samples"].as_array().unwrap().len(), 2);
assert_eq!(c["held_out"].as_array().unwrap().len(), 1);
assert!(c["samples"][0]["train_caption"].as_str().unwrap().starts_with("lushlofi, "));
assert_eq!(c["config"]["max_duration_s"], 240);
// the node claims it although it has no node_datasets row
let job = claim(&app, &token).await;
assert_eq!(job["type"], "train_lora");
assert_eq!(job["payload"]["dataset"]["snapshot_id"], r.json["snapshot_id"]);
assert_eq!(job["payload"]["samples"].as_array().unwrap().len(), 2);
assert!(job["payload"]["inputs"]["t0"]["artifact_id"].is_string());
// the worker may download a listed input (existing GET /jobs/{id}/inputs/{artifact})
```

`async fn lora_snapshot_is_immutable_and_independent_of_catalog_edits()` kiểm Review Focus 2:
- Sau khi tạo, gọi `put_caption` sửa bài train thành `"changed"`, và PATCH tempo bài đó thành 99.
- GET snapshot vẫn trả caption và bpm cũ.
- `sqlx::query("UPDATE dataset_snapshots SET content = '{}'")` chạy trực tiếp lên `app.db` phải trả `Err`.

`async fn create_lora_rejections()`:
- Plan chưa ready (bài thiếu caption) thì 422, message là lỗi đầu tiên trong `errors`, và bảng `dataset_snapshots` rỗng.
- Một ID bị xóa mềm giữa lúc preview và create (Review Focus 1) thì 422 `"Có bài không còn trong catalog này"`.
- Tạo lần hai khi run đầu còn `queued` (Review Focus 3) thì 409 `"Đang có một training run khác chưa kết thúc"`, và đếm snapshot == 1.
- Không có node nào nhận `train_lora` thì 422 `"Không có render node nào hỗ trợ training"`.

- [ ] **Step 2: Chạy test, kiểm lỗi**

Run: `cargo test --test api lora_`
Expected: các test mới FAIL (404 / thiếu bảng). Test của Task 1 vẫn PASS.

- [ ] **Step 3: Cài đặt**

`create_build`:
1. Chạy `plan_build`. Nếu `!ready` thì trả 422 với `errors[0]`.
2. Kiểm `ensure_training_node(db, None)` và `ensure_no_active_run`.
3. Kiểm base model như `create_training_run`.
4. Tạo `lora_key = format!("{}-{}", slug(name), &job_id[len-8..])`. Nếu không có `seed` thì dùng `random_seed()`.
5. Dựng `content`. `content_sha256` là SHA-256 của chuỗi `content` đúng như được ghi vào DB.
6. Trong **một transaction**: insert job (`max_attempts 1`, `title = name`, `request = body`), rồi insert snapshot, rồi ghi audit `training.created` với `{"snapshot_id", "catalog_id", "lora_key"}`.

Trong cùng transaction, kiểm lại "đang có run" bằng `INSERT ... SELECT ... WHERE NOT EXISTS (live train_lora)`. Nếu `rows_affected == 0` thì trả 409. Như vậy hai request đồng thời không cùng lọt.

Sửa claim trong `worker.rs` như ở khối Interfaces. Đừng đổi các điều kiện khác.

- [ ] **Step 4: Chạy test, kiểm đạt**

Run: `cargo test` (toàn bộ, gồm `training_run_flow` kiểu cũ và `capability_based_scheduling`), rồi `cargo clippy --all-targets`.
Expected: tất cả PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/api docs/API_CONTRACTS.md docs/WORKER_PROTOCOL.md
git commit -m "feat(api): create LoRA from catalog with immutable dataset snapshot"
```

---

### Task 3: API – Hoàn tất run: checkpoint, `snapshot_id`, quyền trainer

**Files:**
- Modify: `apps/api/src/routes/worker.rs` (nhánh `train_lora` trong `complete`), `apps/api/src/routes/training.rs` (`list_training_runs`, `get_training_run`)
- Modify: `docs/API_CONTRACTS.md` (chi tiết training run), `docs/WORKER_PROTOCOL.md` (`result.checkpoints`)
- Test: `apps/api/tests/api.rs`

**Interfaces:**
- Consumes: bảng `lora_checkpoints`, `loras.snapshot_id` (Task 2).
- Produces:
  - Worker gửi `result.checkpoints = [{"epoch": i64, "step": i64|null, "path": str, "sha256": 64-hex, "bytes": i64}]`.
  - `GET /training-runs` và `GET /training-runs/{id}` dùng `Trainer`.
  - Chi tiết run thêm hai trường:
    - `snapshot`: `{id, catalog_id, catalog_name, samples, held_out: [filename], keyword, test_prompt, models: [{model_key, name, count}]}`, hoặc null với run kiểu cũ.
    - `checkpoints`: `[{id, epoch, step, bytes, status}]`, không có `node_path`, sắp theo epoch.

- [ ] **Step 1: Viết test lỗi**

`async fn snapshot_run_completion_records_checkpoints()`. Tạo và claim như Task 2. Gửi complete với:

```json
{"lora": {"key": "<lora_key>", "adapter_sha256": "<64 hex>"},
 "checkpoints": [{"epoch": 1, "step": 1, "path": "/x/epoch_1", "sha256": "<64 hex>", "bytes": 10},
                 {"epoch": 2, "step": 2, "path": "/x/epoch_2", "sha256": "<64 hex>", "bytes": 10}]}
```

Assert:
- Trainer `carol` GET `/training-runs/{job}` nhận 200.
- `checkpoints` có 2 dòng `status "kept"` và body không có chuỗi `node_path` hay `/x/epoch_1`.
- `snapshot.samples == 2`, `snapshot.held_out` có 1 tên file.
- `loras` của job có `snapshot_id` khác null và status `evaluation`.

`async fn snapshot_run_completion_validates_checkpoints()`:
- `checkpoints` rỗng thì 422 `"Job train_lora chưa báo checkpoint"`, job vẫn `running`.
- `sha256` không phải 64 ký tự hex thì 422 `"checkpoint không hợp lệ"`.
- Gửi complete lần hai sau khi đã thành công vẫn trả `{"ok": true}` (idempotent như hiện nay).
- Run kiểu cũ (`training_run_flow`) không gửi `checkpoints` vẫn hoàn tất, và chi tiết có `snapshot: null`, `checkpoints: []`.

- [ ] **Step 2: Chạy test, kiểm lỗi**

Run: `cargo test --test api snapshot_run`
Expected: FAIL (403 cho trainer; thiếu trường).

- [ ] **Step 3: Cài đặt**

Trong `complete`, khi job `train_lora` có `payload.dataset.snapshot_id`:
- Kiểm checkpoint **trước** mọi câu ghi: không rỗng; `epoch ≥ 1`; `sha256` khớp `^[0-9a-f]{64}$`; `path` không rỗng.
- Insert `loras` kèm `snapshot_id` và `dataset_key = lora_key`.
- Insert từng checkpoint: `status 'kept'`, `expires_at NULL`, trong cùng `tx`.

Run kiểu cũ giữ nguyên hành vi. `get_training_run` đọc snapshot bằng `job_id` và lấy tóm tắt từ `content`.

- [ ] **Step 4: Chạy test, kiểm đạt**

Run: `cargo test`, `cargo clippy --all-targets`
Expected: tất cả PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/api docs/API_CONTRACTS.md docs/WORKER_PROTOCOL.md
git commit -m "feat(api): record LoRA checkpoints and snapshot on train completion"
```

---

### Task 4: Worker – Dựng dataset từ bản chụp

**Files:**
- Modify: `workers/music-worker/src/lush_worker/audio.py`, `workers/music-worker/src/lush_worker/training.py`, `workers/music-worker/src/lush_worker/runner.py`
- Test: `workers/music-worker/tests/test_training.py`

**Interfaces:**
- Consumes:
  - Payload ở Task 2: `dataset.key`, `dataset.snapshot_id`, `genre`, `samples[{input, train_caption, bpm, keyscale, duration_s}]`.
  - `inputs`, mà `Runner._fetch_inputs` đã đổi thành `{name: {..., "path": "<local file>"}}` (đã kiểm SHA-256).
- Produces:
  - `audio.to_wav48k(src: Path, dest: Path, timeout: float = 600.0) -> None`: chạy `ffmpeg -y -loglevel error -i src -vn -ar 48000 -ac 2 -c:a pcm_s16le dest`. Không có ffmpeg hoặc exit ≠ 0 thì raise `RuntimeError` kèm stderr (tối đa 300 ký tự).
  - `training.build_snapshot_dataset(dataset_dir: Path, payload: dict, convert: Callable[[Path, Path], None] = to_wav48k) -> int`:
    - Trả số mẫu đã ghi.
    - Ghi vào `<dataset_dir>.partial` rồi rename. Lỗi thì xóa `.partial`.
    - Đã có `dataset_dir` thì raise `TrainingError("dataset ... already exists on this node; it is never overwritten")`.
  - `dataset.json` sinh ra có dạng:

```json
{"metadata": {"name": "<key>", "custom_tag": "", "tag_position": "prepend",
              "all_instrumental": true, "genre_ratio": 0},
 "samples": [{"filename": "s000.wav", "audio_path": "s000.wav", "caption": "<train_caption>",
              "genre": "<genre>", "lyrics": "[Instrumental]", "is_instrumental": true,
              "language": "unknown", "duration": 180, "labeled": true,
              "bpm": 80, "keyscale": "A minor"}]}
```

  Quy tắc điền các trường:
  - `duration = int(round(min(probe_wav(dest)[1], 240)))`.
  - Bỏ hẳn `bpm` hoặc `keyscale` khi giá trị null; không ghi `null`.
  - Tên file `s{index:03d}.wav` theo thứ tự `samples`.

- [ ] **Step 1: Viết test lỗi** (fake convert: chép một WAV ngắn do test tự tạo bằng `wave`)

```python
def test_snapshot_dataset_is_built_in_side_step_format(tmp_path):
    payload = snapshot_payload(tmp_path, n=2, bpm=[80, None], keyscale=["A minor", None])
    count = build_snapshot_dataset(tmp_path / "datasets" / "k1", payload, convert=fake_convert)
    data = json.loads((tmp_path / "datasets" / "k1" / "dataset.json").read_text())
    assert count == 2
    assert data["metadata"]["custom_tag"] == ""
    s0, s1 = data["samples"]
    assert s0["audio_path"] == "s000.wav" and s0["caption"].startswith("lushlofi, ")
    assert s0["bpm"] == 80 and s0["keyscale"] == "A minor" and s0["lyrics"] == "[Instrumental]"
    assert "bpm" not in s1 and "keyscale" not in s1
    assert (tmp_path / "datasets" / "k1" / "audio" / "s001.wav").is_file()

def test_snapshot_dataset_failure_leaves_nothing(tmp_path):
    def boom(src, dst): raise RuntimeError("ffmpeg not found")
    with pytest.raises(TrainingError, match="ffmpeg not found"):
        build_snapshot_dataset(tmp_path / "datasets" / "k1", snapshot_payload(tmp_path, n=1), convert=boom)
    assert not (tmp_path / "datasets" / "k1").exists()
    assert not (tmp_path / "datasets" / "k1.partial").exists()

def test_snapshot_run_fails_before_stopping_inference_when_audio_breaks(trainer, service, monkeypatch, tmp_path):
    # Review Focus 5
    monkeypatch.setattr(training, "to_wav48k", lambda s, d: (_ for _ in ()).throw(RuntimeError("ffmpeg not found")))
    with pytest.raises(TrainingError):
        trainer.run(Ctx(), snapshot_payload(tmp_path, n=1))
    assert "stop" not in service.calls

def test_snapshot_run_trains_with_fake_sidestep(trainer, node, service, tmp_path, monkeypatch):
    monkeypatch.setattr(training, "to_wav48k", fake_convert)
    result = trainer.run(Ctx(), snapshot_payload(tmp_path, n=2))
    assert result["samples"] == 2
    assert (node["data"] / "datasets" / "lofi-jazz-0001" / "dataset.json").is_file()
```

Ngoài ra, test cho `to_wav48k`:
- Monkeypatch `audio.ffmpeg_path` trả None thì raise `RuntimeError` chứa `"ffmpeg"`. Không cần ffmpeg thật.

Test runner: `test_runner_downloads_snapshot_inputs_before_training`. Dùng respx, giống `test_runner_trains_completes_and_registers_again`:
- Job có `samples` + `inputs`.
- Route download input trả bytes khớp `sha256` của input.
- Assert trainer nhận payload có đường dẫn local.
- Khi SHA sai thì job bị `fail` và trainer không chạy.

- [ ] **Step 2: Chạy test, kiểm lỗi**

Run: `cd workers/music-worker; .venv\Scripts\python -m pytest tests/test_training.py -q`
Expected: FAIL, ImportError `build_snapshot_dataset` / `to_wav48k`.

- [ ] **Step 3: Cài đặt**

Trong `SideStepTrainer.run`, khi `payload.get("samples")`:
- Kiểm `dataset_key` theo `LORA_KEY_RE`.
- Dataset là `self.dataset_dir / dataset_key`.
- Gọi `build_snapshot_dataset` **trước** `self.service.stop()`, và trước bước kiểm `dataset.json`.
- Trước khi dựng, gọi `self._report(ctx, 0.01, f"preparing {n} samples", "preparing_dataset")`. Web đặt nhãn cho phase này ở Task 8.
- Có `ctx.raise_if_aborted()` sau mỗi file.
- Bọc `RuntimeError`/`OSError` thành `TrainingError`.

Payload không có `samples` thì đi đường cũ, không đổi gì.

Trong `Runner._execute_training`, khi payload có `samples` thì gọi `self._fetch_inputs(ctx, payload)` (đã kiểm SHA-256), rồi truyền payload trả về vào `trainer.run`.

- [ ] **Step 4: Chạy test, kiểm đạt**

Run: `.venv\Scripts\python -m pytest -q`
Expected: toàn bộ PASS (161 cũ + test mới).

- [ ] **Step 5: Commit**

```bash
git add workers/music-worker
git commit -m "feat(worker): build Side-Step dataset from LoRA snapshot payload"
```

---

### Task 5: Worker – Báo checkpoint đầy đủ

**Files:**
- Modify: `workers/music-worker/src/lush_worker/training.py`, `workers/music-worker/tests/fake_sidestep.py` (nếu cần in thêm checkpoint)
- Modify: `docs/WORKER_PROTOCOL.md` (mục Job `train_lora`: `result.checkpoints`)
- Test: `workers/music-worker/tests/test_training.py`

**Interfaces:**
- Consumes: `RunState`, `_checkpoints`, `CHECKPOINT_RE`.
- Produces: `result["checkpoints"] = [{"epoch", "step", "path", "sha256", "bytes"}]`, sắp theo epoch, cho **mọi** run (cả kiểu cũ):
  - `step` là `state.step` tại dòng `[OK] Checkpoint saved at epoch N`. Không biết thì để None.
  - `sha256` là SHA-256 của `adapter_model.safetensors` trong thư mục đó.
  - Run `completed` mà epoch cuối chưa có checkpoint (P6): thêm `out/final` với `epoch = state.epoch`, `step = state.step`.
  - Run dừng vì `time_limit` chỉ báo các checkpoint đã có.

- [ ] **Step 1: Viết test lỗi**

```python
def test_result_lists_every_checkpoint_with_step_and_sha(trainer, node, service):
    result = trainer.run(Ctx(), payload(epochs=4, save_every=1))
    eps = [c["epoch"] for c in result["checkpoints"]]
    assert eps == [1, 2, 3, 4]
    assert all(len(c["sha256"]) == 64 and c["path"] and c["bytes"] > 0 for c in result["checkpoints"])
    assert all(c["step"] is not None for c in result["checkpoints"])

def test_final_adapter_is_reported_when_last_epoch_has_no_checkpoint(trainer, node, service):
    result = trainer.run(Ctx(), payload(epochs=3, save_every=2))
    assert [c["epoch"] for c in result["checkpoints"]] == [2, 3]
    assert result["checkpoints"][-1]["path"].endswith("final")
```

Kiểm cách `fake_sidestep.py` in checkpoint và có ghi `out/final` hay không. Nếu fake chưa làm đúng như Side-Step thật (dòng `[OK] Checkpoint saved at epoch N`, thư mục `checkpoints/epoch_N_loss_X`, `final/`), sửa fake cho khớp thật. Không sửa test để chiều theo fake.

- [ ] **Step 2: Chạy test, kiểm lỗi**

Run: `.venv\Scripts\python -m pytest tests/test_training.py -q -k checkpoint`
Expected: FAIL (thiếu `step`/`sha256`/`path`).

- [ ] **Step 3: Cài đặt**

- Thêm `checkpoint_steps: dict[int, Optional[int]]` vào `RunState`, ghi trong `_parse` khi gặp `CHECKPOINT_RE`.
- `_checkpoints` trả thêm `sha256` và `step`.
- Đường dẫn trả về là đường dẫn tuyệt đối (`resolve()`).

- [ ] **Step 4: Chạy test, kiểm đạt**

Run: `.venv\Scripts\python -m pytest -q`
Expected: toàn bộ PASS.

- [ ] **Step 5: Commit**

```bash
git add workers/music-worker docs/WORKER_PROTOCOL.md
git commit -m "feat(worker): report every LoRA checkpoint with step and sha256"
```

---

### Task 6: Web – Kiểu, API client, logic hộp Tạo LoRA

**Files:**
- Modify: `apps/web/src/api/types.ts`, `apps/web/src/api/client.ts` (hoặc `api.ts`, theo nơi P1 đặt các hàm catalog), `apps/web/src/api/hooks.ts`, `apps/web/src/api/mutations.ts`
- Create: `apps/web/src/features/lora/createLoraForm.ts`, `apps/web/src/features/lora/createLoraForm.test.ts`

**Interfaces:**
- Consumes: các endpoint ở Task 1–3.
- Produces:
  - Types: `LoraBuildRequest`, `BuildTrack`, `BuildPlan`, `CreateLoraResult {job_id, snapshot_id, lora_key}`, `DatasetSnapshot`. `TrainingRunDetail` thêm `snapshot: SnapshotSummary | null` và `checkpoints: Checkpoint[]`. Tên trường khớp đúng JSON của API.
  - Hooks:
    - `useLoraPreview(catalogId: string, body: LoraBuildRequest | null)`: query key `['lora-preview', catalogId, body]`, `placeholderData: keepPreviousData`, `enabled: body !== null`.
    - `useCreateLora(catalogId)`: mutation.
    - `useDatasetSnapshot(id)`.
  - Trong `createLoraForm.ts`:

```ts
export type TrainKind = 'quick' | 'standard' | 'thorough' | 'custom'
export const TRAIN_KIND_EPOCHS: Record<Exclude<TrainKind, 'custom'>, number> = { quick: 1, standard: 4, thorough: 8 }
export const CREATE_LORA_DEFAULTS = { rank: 32, alpha: 64, learning_rate: 0.0001, epochs: 4,
  gradient_accumulation: 4, save_every: 1, warmup_steps: 2, max_minutes: 30, holdout_count: 2 } as const
export interface CreateLoraState { name: string; keyword: string; testPrompt: string; testPromptEdited: boolean
  kind: TrainKind; epochs: string; saveEvery: string; holdoutCount: string; rank: string; alpha: string
  learningRate: string; gradientAccumulation: string; warmupSteps: string; seed: string; maxMinutes: string
  baseModelId: string; holdoutSeed: number; voiceConfirmed: string[]; excluded: string[] }
export function initialCreateLoraState(baseModelId: string, genreName: string, random?: () => number): CreateLoraState
export function setKind(s: CreateLoraState, kind: TrainKind): CreateLoraState      // đặt epochs theo kiểu train
export function setKeyword(s: CreateLoraState, keyword: string, genreName: string): CreateLoraState // tự điền lại testPrompt khi !testPromptEdited
export function defaultTestPrompt(keyword: string, genreName: string): string    // P4
export function localProblem(s: CreateLoraState): string | null                  // lỗi số liệu trước khi gọi preview
export function buildLoraRequest(s: CreateLoraState, trackIds: string[]): LoraBuildRequest | null // null khi localProblem
export function estimateMinutes(runs: Job[], totalSteps: number): number | null  // P8
```

- [ ] **Step 1: Viết test lỗi** (`createLoraForm.test.ts`)

```ts
it('defaults match the approved dialog', () => {
  const s = initialCreateLoraState('m1', 'Lo-fi', () => 0.5)
  expect(s.epochs).toBe('4'); expect(s.saveEvery).toBe('1'); expect(s.holdoutCount).toBe('2')
  expect(s.kind).toBe('standard'); expect(s.testPrompt).toBe('Lo-fi')
})
it('train kind sets epochs; typing epochs switches to custom', () => {
  expect(setKind(initialCreateLoraState('m', 'G'), 'quick').epochs).toBe('1')
  expect(setKind(initialCreateLoraState('m', 'G'), 'thorough').epochs).toBe('8')
})
it('keyword refills the test prompt until the user edits it', () => {
  let s = setKeyword(initialCreateLoraState('m', 'Lo-fi'), 'lushlofi', 'Lo-fi')
  expect(s.testPrompt).toBe('lushlofi, Lo-fi')
  s = { ...s, testPrompt: 'mine', testPromptEdited: true }
  expect(setKeyword(s, 'x', 'Lo-fi').testPrompt).toBe('mine')
})
it('builds the request without excluded tracks and with numbers', () => {
  const s = { ...initialCreateLoraState('m', 'G'), name: ' A ', excluded: ['t2'] }
  const r = buildLoraRequest(s, ['t1', 't2', 't3'])!
  expect(r.track_ids).toEqual(['t1', 't3']); expect(r.name).toBe('A'); expect(r.epochs).toBe(4)
  expect(r.seed).toBeNull()
})
it('estimateMinutes uses the latest completed run with steps, else null', () => {
  expect(estimateMinutes([], 16)).toBeNull()
  // run: status completed, result { train_seconds: 120, optimizer_steps: 8 } -> 15 s/step -> 16 steps = 4 min
  expect(estimateMinutes([job({ status: 'completed', result: { train_seconds: 120, optimizer_steps: 8 } })], 16)).toBe(4)
})
```

Thêm test `localProblem`: save_every lớn hơn epochs thì trả về chuỗi lỗi; seed không phải số thì trả lỗi. Dùng lại câu chữ của `validateTrainingForm`.

Thêm một test client: `createLora` gửi `POST /api/v1/catalogs/{id}/loras`, `encodeURIComponent` id như P1.

- [ ] **Step 2: Chạy test, kiểm lỗi**

Run: `cd apps/web; npx vitest run src/features/lora`
Expected: FAIL, module chưa có.

- [ ] **Step 3: Cài đặt**

- Dùng lại kiểm lỗi số từ `features/training/trainingForm.ts`: `integerProblem` chuyển sang export, không sao chép.
- `estimateMinutes` làm tròn lên số phút nguyên.

- [ ] **Step 4: Chạy test, kiểm đạt**

Run: `npm test`, `npx tsc --noEmit`
Expected: tất cả PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/web/src
git commit -m "feat(web): create-LoRA form logic, types and API hooks"
```

---

### Task 7: Web – Hộp Tạo LoRA trên trang Catalog

**Files:**
- Create: `apps/web/src/features/lora/CreateLoraDialog.tsx`, `apps/web/src/features/lora/CreateLoraDialog.test.tsx`
- Modify: `apps/web/src/pages/CatalogPage.tsx`, `apps/web/src/pages/CatalogPage.test.tsx`, CSS của trang (cùng file P1 đã dùng cho `catalog-*`)

**Interfaces:**
- Consumes:
  - `useLoraPreview`, `useCreateLora`, `createLoraForm.*` (Task 6).
  - `useCreateCaptionJob` và `modelKey` của CatalogPage (P1).
  - Danh sách model generator, theo bộ lọc mà P1 đã thêm cho `GenerateForm`.
  - `useTrainingRuns()` (đã có trong `api/hooks.ts`) cho `estimateMinutes`.
- Produces: `<CreateLoraDialog catalogId genreName trackIds onClose onDeselect(ids) captionModelKey />`. Khi tạo thành công, `navigate('/training/' + job_id)`.

Bố cục theo mockup `create-lora.png`. Cột trái có 3 nhóm, đúng nhãn và chú thích:

1. **Cần điền**
   - "Tên LoRA *", kèm chú thích "Tên hiện trong Style Preset khi tạo nhạc."
   - "Từ khóa gọi style", kèm chú thích "Tự thêm vào đầu mọi mô tả khi train. Khi tạo nhạc, gõ từ này để gọi đúng style."
2. **Nên chỉnh**
   - "Kiểu train", dạng 4 nút: "Thử nhanh · 1 epoch", "Chuẩn · 4 epoch", "Kỹ · 8 epoch", "Tự nhập".
   - "Epoch".
   - "Lưu bản mỗi N epoch".
   - "Bài giữ lại để nghe thử", liệt kê tên các bài trong `held_out`.
   - "Câu lệnh nghe thử sau khi train".
3. **Mặc định · Nâng cao**: `<details>` thu gọn, gồm:
   - Rank, Alpha, Learning rate, Gradient accumulation, Warmup steps, Seed ("Ngẫu nhiên" khi trống), Giới hạn thời gian.
   - "Độ dài tối đa mỗi bài: 4 phút đầu", chỉ đọc.
   - Base model: select các generator `acestep` đang active.

Cột phải lấy số liệu từ preview:

- **"Bài đưa vào LoRA":**
  - `N bài đã chọn`, `X train · Y nghe thử`, `Z phút tổng nhạc train`.
  - Khi `unknown_duration > 0`, thêm "(N bài chưa rõ thời lượng)".
- **"Mô tả lấy từ model":** dạng thanh màu, mỗi model một màu, kèm `name · count bài`.
- **Danh sách chặn:**
  - Mỗi bài `missing_caption` có nút "Tạo mô tả" (chạy `useCreateCaptionJob` với `captionModelKey` cho các bài đó; disable khi không có model cài sẵn) và "Bỏ bài này" (thêm vào `excluded` và gọi `onDeselect`).
  - Mỗi bài `voice_unconfirmed` hiện `vocals` hoặc caption ngắn, kèm nút "Bỏ bài này" và "Xác nhận không có giọng" (thêm vào `voiceConfirmed`).
- **"Dự kiến":**
  - "Bước train": `{steps_per_epoch} bước/epoch × {epochs} epoch = {total_steps} bước`.
  - "Bản lưu": `{n} bản (epoch …)`.
  - "Nhạc nghe thử": `{eval_renders - 1} bài LoRA + 1 bài gốc`.
  - "Thời gian": `~{estimateMinutes} phút` hoặc `"— (đo sau lần train đầu)"`.
- **"Kiểm tra trước khi train":** hiện `errors` (đỏ) và `warnings`.
- **Dòng chót:** "Bấm “Bắt đầu train” là chạy GPU thật. Có thể hủy giữa chừng; các bản đã lưu vẫn giữ."
- **Nút:**
  - "Hủy".
  - "Bắt đầu train": disable khi `localProblem` khác null, `!preview.ready`, preview đang tải lần đầu, hoặc mutation đang chạy.
  - Lỗi từ create hiện ngay trong hộp.

Gọi preview với body debounce 300 ms. Nếu repo đã có hook debounce thì dùng lại; nếu chưa có thì viết `useDebounced` nhỏ ngay trong file dialog.

CatalogPage:
- Thêm nút đầu trang "Tạo LoRA ({n} bài đã chọn)", disable khi chưa chọn bài nào. Bấm thì mở dialog với `selectedIds`.
- Tên genre truyền vào dialog lấy từ `catalogQuery.data.genre_name`.

- [ ] **Step 1: Viết test lỗi** (`CreateLoraDialog.test.tsx`; mock fetch theo cách `CatalogPage.test.tsx` làm)

Mỗi mục dưới đây là một test:
- Trả preview `ready:false` với 1 bài `voice_unconfirmed`. Kiểm "Bắt đầu train" bị disable. Bấm "Xác nhận không có giọng", rồi kiểm request preview kế tiếp có `voice_confirmed: [id]`.
- Bấm "Bỏ bài này" trên bài `missing_caption`. Kiểm request kế tiếp không còn id đó, và `onDeselect` được gọi với `[id]`.
- Bấm "Thử nhanh · 1 epoch". Kiểm request có `epochs: 1`.
- Preview `ready:true`, điền tên, bấm "Bắt đầu train". Kiểm có `POST /catalogs/c1/loras` và router chuyển tới `/training/j1`.
- Create trả 409. Kiểm message "Đang có một training run khác chưa kết thúc" hiện trong hộp.
- Mở "Mặc định · Nâng cao". Kiểm giá trị mặc định rank 32, alpha 64, warmup 2.

`CatalogPage.test.tsx`: khi chọn 2 bài, nút "Tạo LoRA (2 bài đã chọn)" bật, và bấm thì mở dialog có heading "Tạo LoRA".

- [ ] **Step 2: Chạy test, kiểm lỗi**

Run: `npx vitest run src/features/lora src/pages/CatalogPage.test.tsx`
Expected: FAIL.

- [ ] **Step 3: Cài đặt** theo mô tả ở trên. Dùng lại các component trong `components/ui` (Notice, ErrorState) và lớp `.btn` sẵn có.

- [ ] **Step 4: Chạy test, kiểm đạt**

Run: `npm test`, `npm run build`
Expected: tất cả PASS, build không lỗi.

- [ ] **Step 5: Commit**

```bash
git add apps/web/src
git commit -m "feat(web): Create LoRA dialog on the catalog page"
```

---

### Task 8: Web – Trang training: "Train từ", bản lưu, luồng cũ

**Files:**
- Modify: `apps/web/src/pages/TrainingRunPage.tsx` (+ test), `apps/web/src/pages/TrainingPage.tsx` (+ test)

**Interfaces:**
- Consumes: `TrainingRunDetail.snapshot`, `TrainingRunDetail.checkpoints`, `useDatasetSnapshot` (Task 6).
- Produces: không có gì cho task sau.

TrainingRunPage, khi `snapshot` khác null:
- Thêm khung **"LoRA này được train từ"**, gồm:
  - Tên catalog, có link `/catalogs/{catalog_id}`.
  - `samples` bài train, các bài giữ lại (liệt kê tên).
  - Từ khóa, hoặc "Không có" khi trống.
  - Câu lệnh nghe thử.
  - Thanh model như ở dialog.
- Nút **"Xem danh sách bài và mô tả"** mở bảng `useDatasetSnapshot`, gồm các cột: Bài, Model, Mô tả khi train (`train_caption`), Tempo, Key. Bài giữ lại có nhãn "Giữ lại".

Khi `checkpoints` không rỗng, thêm khung **"Bản lưu"**:
- Mỗi dòng ghi `Epoch {epoch} · bước {step ?? '—'} · {MB} MB`.
- Có ghi chú "Nghe thử và chọn bản sẽ có ở bước sau."

Nhãn stage: thêm `preparing_dataset: 'Chuẩn bị dữ liệu'` vào `TRAINING_STAGE_LABELS`. Worker báo phase này ở Task 4.

TrainingPage:
- Nút chính "Tạo LoRA từ Catalog" (link `/catalogs`).
- Form cũ nằm trong `<details><summary>Train từ dataset có sẵn trên máy (cách cũ)</summary>`, chỉ render khi `user.role === 'admin'`.
- Trainer vẫn thấy danh sách run.

- [ ] **Step 1: Viết test lỗi**

Mỗi mục dưới đây là một test:
- TrainingRunPage nhận detail có `snapshot`. Kiểm có heading "LoRA này được train từ", có link tới `/catalogs/c1`, và mục "Giữ lại" có tên bài.
- Detail có `checkpoints` 2 dòng. Kiểm có chữ "Epoch 1" và "Epoch 2", và không có chuỗi đường dẫn nào. Dữ liệu test cũng không chứa đường dẫn.
- Run kiểu cũ (`snapshot: null`). Kiểm không có khung "LoRA này được train từ".
- TrainingPage với trainer. Kiểm có link "Tạo LoRA từ Catalog" và không có summary "cách cũ".
- TrainingPage với admin. Kiểm có summary "cách cũ".

- [ ] **Step 2: Chạy test, kiểm lỗi**

Run: `npx vitest run src/pages/TrainingRunPage.test.tsx src/pages/TrainingPage.test.tsx`
Expected: FAIL.

- [ ] **Step 3: Cài đặt** theo mô tả ở trên.

- [ ] **Step 4: Chạy test, kiểm đạt**

Run: `npm test`, `npm run build`
Expected: tất cả PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/web/src
git commit -m "feat(web): training run shows dataset snapshot and checkpoints"
```

---

### Task 9: Tài liệu dự án và chạy toàn bộ test

**Files:**
- Modify: `docs/tasks/CURRENT.md`, `docs/DECISIONS.md`, `docs/CHANGELOG.md`, `README.md` (chỉ khi README đang liệt kê tính năng Catalog)

- [ ] **Step 1: Ghi tài liệu**

`docs/tasks/CURRENT.md`: thêm task **T-016 🟡 LoRA Studio P2 – Tạo LoRA từ Catalog**, ghi các mục sau:
- Nhánh `feat/lora-studio-web`.
- Những gì đã chạy: số test thật của 3 bộ test.
- Những gì **chưa** chạy thật:
  - Chưa train trên rtx6000.
  - Chưa kiểm ffmpeg trên rtx6000.
  - Chưa đo VRAM và thời gian của run "Thử nhanh".
- Các bước cần người dùng duyệt:
  - Deploy migration 0006 và 0007.
  - Chạy thật một run "Thử nhanh" trên rtx6000 theo nghiệm thu 9.2 của spec.
- Các phần để lại cho P3: eval, chọn bản, xóa bản, Style Preset.
- Sửa số test cũ của T-015 cho đúng (đây là việc tồn từ P1).

`docs/DECISIONS.md`: thêm **D32**. Nội dung: bản chụp dataset bất biến (trigger SQLite); payload train mang `samples` và `inputs`; worker dựng `dataset.json`; checkpoint ghi vào Registry. Ghi kèm lý do và cái giá. Liệt kê các ruling P1–P12 của plan này.

`docs/CHANGELOG.md`: thêm một dòng.

- [ ] **Step 2: Chạy toàn bộ test**

Run:
- `cd apps/api; cargo test; cargo clippy --all-targets`
- `cd workers/music-worker; .venv\Scripts\python -m pytest -q`
- `cd apps/web; npm test; npm run build`

Expected: tất cả PASS. Ghi đúng số test vào CURRENT.md.

- [ ] **Step 3: Commit**

```bash
git add docs README.md
git commit -m "docs: LoRA Studio P2 status, D32 and changelog"
```
