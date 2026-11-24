# LoRA Studio P3 – Nghe thử, chấm điểm, chọn bản: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Sau khi `train_lora` (từ bản chụp) xong, hệ thống tự tạo nhạc nghe thử cho từng bản lưu và chấm điểm. Người dùng nghe, ghi chú, chọn **một** epoch, đưa vào Style Preset. Các bản không chọn tự xóa sau 30 ngày, hoặc xóa ngay.

**Architecture:**
- **Ba job mới, đều ghim vào node đã train** (`payload.node_id`):
  - `eval_lora`: tạo tự động khi `train_lora` có bản chụp hoàn tất. Render nhạc nghe thử bằng ACE-Step, chấm điểm, tải mp3 lên làm artifact.
  - `lora_files`: thao tác file trên node, không dùng GPU. `publish` chép bản lưu đã chọn vào `LORA_DIR/<key>`; `delete` xóa thư mục bản lưu.
  - Thử nhanh là một job `generate` thường, mang theo đường dẫn bản lưu.
- **API:** module mới `routes/lora_review.rs` cho ghi chú, chọn bản/đưa vào Style, xóa ngay, xóa LoRA, thử nhanh. Chi tiết training run mở rộng để trang chọn bản đọc từ một endpoint.
- **Worker:** `evaluation.py` (render + chấm), `scoring.py` (hàm chấm thuần), `lora_files.py`. Máy chấm MERT và Audiobox chạy như captioner ở P1: tiến trình con trong venv riêng (`SCORER_DIR/<tên>/scorer.json`), nói JSON lines. Chưa cài thì vẫn render, điểm để trống.
- **Web:** TrainingRunPage thêm màn hình sau train theo `after-train.png`; CatalogPage nhận "Train lại với tùy chọn khác".

**Tech Stack:** Rust (Axum, SQLx runtime, SQLite), Python 3 worker (pytest, numpy, librosa extra), React 19 + TypeScript + React Query (vitest).

**Spec:** `docs/superpowers/specs/2026-11-23-lora-studio-web-design.md`. Các mục P3: 6.4, 6.5 (dòng `eval_lora`, chấm điểm, cấu hình render), 6.6, 6.7, 9.3. Mockup: `.superpowers/brainstorm/1672-1795426550/content/after-train.png` (git-ignored). Plan P2 đã làm: `docs/superpowers/plans/2026-11-24-lora-studio-p2-create-lora.md`.

## Global Constraints

- Tiếng Việt cho mọi chữ trên giao diện và thông báo lỗi; code, tên API, config key bằng tiếng Anh.
- Chuyển trạng thái job là một câu SQL nguyên tử. SQL runtime (`sqlx::query`). Schema mới = migration mới `0008_lora_review.sql`, không sửa migration cũ.
- API công khai chỉ lộ Artifact ID. `lora_checkpoints.node_path` không bao giờ trả ra web; chỉ đi vào payload job gửi cho node.
- Model chấm là dữ liệu: loại `scorer` trong Model Registry, node báo khi đã cài. Không hardcode danh sách trên web.
- Mỗi đổi hành vi phải sửa `docs/API_CONTRACTS.md` hoặc `docs/WORKER_PROTOCOL.md` trong cùng commit.
- Worker không đụng database. Mọi việc audio và model chạy trên node.
- Một job một node, một lúc. Ưu tiên (spec 6.5): `generate` (0) > `analyze_track` (-10) > `caption_tracks` (-20) > `eval_lora` (-30) > `train_lora` (-40, chỉ áp cho job tạo mới từ P3). `lora_files` = 10 (thao tác file vài giây).
- Cấu hình render khi đánh giá: 60 giây (kẹp vào khoảng độ dài cho phép của genre nếu 60 nằm ngoài; `payload.duration` ghi giá trị thật dùng), seed = `config.seed` của bản chụp, độ mạnh LoRA 0.7. Số render = `(số bản lưu được đánh giá) × (1 + số bài giữ lại) + 1`.
- Điểm chỉ để tham khảo. Ngưỡng ban đầu ghi rõ "chưa hiệu chỉnh" trong code và tài liệu.
- Hạn 30 ngày tính từ lúc người dùng chọn bản (`expires_at`). LoRA chưa chọn bản nào thì không tự xóa gì. Bản `chosen` không bao giờ bị xóa.
- Quyền: mọi thao tác P3 dùng extractor `Trainer` (admin và trainer). Style đã đưa vào dùng được cho mọi người (W18).
- Không tải model (MERT, Audiobox), không chạy GPU thật, không deploy, không push trong plan này. Test chỉ dùng mock hoặc fake.

## Review Focus

1. **Bấm "Đưa vào Style Preset" khi đang có job `lora_files` của LoRA đó chưa xong (bấm hai lần, hoặc đổi epoch liên tục).** Lần hai trả 409 "LoRA đang được cập nhật trên máy, chờ một chút", không tạo Style thứ hai. Test thuộc Task 3.
2. **"Xóa ngay" khi chưa chọn bản nào.** Trả 409 "Chọn một bản trước khi xóa các bản còn lại"; không đánh dấu xóa gì. Test thuộc Task 3.
3. **Đường dẫn bản lưu trong payload nằm ngoài thư mục train của node** (payload hỏng hoặc bị sửa). Worker từ chối, không xóa hay nạp file nào. Test thuộc Task 5.
4. **Máy chấm chưa cài (MERT/Audiobox).** `eval_lora` vẫn render đủ bài, giai điệu (librosa) vẫn có điểm, hai điểm còn lại `null`; web hiện "Chưa cài máy chấm". Test thuộc Task 6 và Task 9.
5. **Train bị hủy hoặc lỗi.** Không tạo `eval_lora`. Run cũ (không bản chụp) hoàn tất cũng không tạo `eval_lora`. Test thuộc Task 1.

---

## File Structure

| File | Trách nhiệm |
|---|---|
| `apps/api/migrations/0008_lora_review.sql` (mới) | Bảng `lora_eval_renders`; cột điểm, `delete_requested_at` ở `lora_checkpoints`; `loras.chosen_checkpoint_id`, `loras.base_key` |
| `apps/api/src/routes/lora_review.rs` (mới) | Enqueue `eval_lora`/`lora_files`, hoàn tất hai job đó, ghi chú, publish, xóa ngay, xóa LoRA, thử nhanh, dọn theo hạn |
| `apps/api/src/routes/jobs.rs` | Tách `build_generation` dùng chung cho generate thường, render đánh giá và thử nhanh |
| `apps/api/src/routes/worker.rs` | Claim: ghim node, job type mới; complete: gọi `lora_review`; heartbeat: dọn theo hạn; kind artifact `eval_NN` |
| `apps/api/src/routes/training.rs` | Chi tiết run thêm `lora`, `eval`, `renders`, điểm và ghi chú của bản lưu, `style_preset`, `held_out_tracks` |
| `apps/api/src/routes/lora_builds.rs` | `eval_renders` theo giới hạn 10 bản lưu; job mới ưu tiên -40 |
| `workers/music-worker/src/lush_worker/engines/acestep.py` | LoRA theo `path` (chỉ dưới thư mục train) |
| `workers/music-worker/src/lush_worker/lora_files.py` (mới) | Job `lora_files` |
| `workers/music-worker/src/lush_worker/scoring.py` (mới) | Hàm chấm thuần + client tiến trình con máy chấm |
| `workers/music-worker/src/lush_worker/evaluation.py` (mới) | Job `eval_lora` |
| `workers/music-worker/scorers/{mert,audiobox}/` (mới) | Script máy chấm, viết theo protocol, chưa chạy thật |
| `workers/music-worker/src/lush_worker/{runner,config}.py` | Đăng ký job type mới, `SCORER_DIR`, báo scorer khi register |
| `apps/web/src/api/*`, `apps/web/src/features/lora/review.ts` (mới) | Kiểu, hook, hàm thuần (bước tiến độ, nhận xét điểm) |
| `apps/web/src/features/lora/{EvalTable,PublishPanel,QuickTryPanel}.tsx` (mới) | Các khối của màn hình chọn bản |
| `apps/web/src/pages/TrainingRunPage.tsx`, `CatalogPage.tsx`, `features/lora/CreateLoraDialog.tsx`, `features/training/LossChart.tsx` | Ghép màn hình; Train lại; vạch epoch |
| Tài liệu | Hợp đồng sửa cùng task; `CURRENT.md`, `DECISIONS.md` (D33), `CHANGELOG.md` ở Task 11 |

## Các quyết định của tôi (ruling khi viết plan, cần người dùng xem)

- **R1. Khóa adapter theo epoch.** Mỗi bản lưu có khóa `<lora_key>-e<epoch>`, dùng chung cho render đánh giá, thử nhanh và bản đã đưa vào Style. ACE-Step nạp mỗi adapter một lần theo tên và `unload` bị hỏng (đã ghi trong `acestep.py`), nên đổi epoch phải là **khóa mới**, không ghi đè thư mục cũ. Khi chọn bản, `loras.key` đổi thành khóa đó; `loras.base_key` giữ `lora_key` gốc.
- **R2. Đưa vào Style xong mới dùng được.** Bấm "Đưa vào Style Preset" tạo Style `is_active = 0` và job `lora_files publish`. Khi job xong: LoRA `approved`, Style bật, node được ghi là đã cài LoRA. Đổi epoch về sau: cập nhật Style cũ (không tạo mới); job tạo nhạc đang chờ sẽ đợi node cài xong bản mới. Thư mục của bản đưa vào trước **không xóa** (một job tạo nhạc cũ có thể đang chờ dùng nó).
- **R3. Xóa file qua job `lora_files`, không qua heartbeat** (spec 6.6 ghi heartbeat là lựa chọn của tôi khi thiết kế). Job có lease, có kết quả, có lỗi hiện trên web. Việc dọn theo hạn vẫn khởi động từ heartbeat: mỗi lần node gửi heartbeat mà không có job `lora_files` nào đang chờ của node đó, API tìm bản lưu đã quá hạn và tạo một job `delete`.
- **R4. Đánh giá tối đa 10 bản lưu**, là 10 epoch cuối (để số render ≤ 61, artifact kind `eval_00`…`eval_99`). Hộp Tạo LoRA tính `eval_renders` theo giới hạn này.
- **R5. Không cần tải nhạc cho eval.** Bài train đã nằm trên node ở `datasets/<lora_key>/sNNN.wav` (P2). Payload `eval_lora` liệt kê `dataset_file` và tên bài. Bài giữ lại thật nghe trên web từ artifact catalog có sẵn.
- **R6. Câu lệnh render:** bài mẫu không LoRA và mỗi bản lưu dùng `test_prompt`. "LoRA tạo lại bài giữ lại" dùng `train_caption` của bài đó (đã có từ khóa nếu có). Mỗi render đi qua `build_generation` để giống hệt job tạo nhạc thường (model, bước, guidance theo genre của catalog).
- **R7. Điểm:**
  - **Giống style:** cosine giữa trung bình embedding MERT các render của bản lưu và trung bình embedding các bài train.
  - **Giống bài train:** max cosine MERT giữa từng render và từng bài train, kèm tên bài.
  - **Giai điệu:** chroma (librosa `chroma_cqt`, 22050 Hz, hop 2048, 60 s đầu) + DTW (metric cosine). Độ giống = `1 - cost / len(path)`, cắt về [0, 1]. Lấy max, kèm tên bài.
  - **Chất lượng âm thanh:** trung bình điểm PQ của Audiobox trên các render của bản lưu (thang 1–10).
  - **Cảnh báo học thuộc (chưa hiệu chỉnh):** MERT ≥ 0.95 hoặc giai điệu ≥ 0.80 → "Có dấu hiệu học thuộc: gần giống <tên bài>".
- **R8. Ghi chú thay cho chấm sao** (W13; mockup còn cột sao, spec đã chỉnh). Tối đa 500 ký tự, lưu khi rời ô.
- **R9. Thử nhanh** là job `generate` của người bấm, ra track trong thư viện của họ như tạo nhạc thường. Không gắn `jobs.lora_id` (bản lưu chưa "cài" theo nghĩa Registry), nên ghim node bằng `payload.node_id`. Chỉ dùng được với bản lưu chưa bị xóa.
- **R10. Xóa LoRA:** bị chặn (409, kèm tên Style) khi còn Style Preset dùng LoRA đó, hoặc khi còn job train/eval/lora_files chưa xong. Nếu được: `loras.status = 'archived'`, mọi bản lưu chưa xóa được đánh dấu xóa (kể cả bản chosen), tạo job `delete`. Bản chụp dataset giữ nguyên: trigger P2 chặn mọi UPDATE nên "archived" suy ra từ LoRA.
- **R11. Sau `eval_lora`, worker khởi động lại `acestep-api`** khi job đã nạp ít nhất một adapter, để các adapter đánh giá không nằm lại trong VRAM. Trong lúc đó tạo nhạc phải chờ (như lúc train).
- **R12. Tên Style mặc định** là tên LoRA; độ mạnh mặc định 0.7; `prompt_template` = từ khóa (nếu có); genre = genre của catalog; model = base model.

---

### Task 1: API – Migration, ghim node, tự tạo `eval_lora`

**Files:**
- Create: `apps/api/migrations/0008_lora_review.sql`, `apps/api/src/routes/lora_review.rs`
- Modify: `apps/api/src/routes/jobs.rs`, `worker.rs`, `lora_builds.rs`, `routes/mod.rs`
- Modify: `docs/WORKER_PROTOCOL.md` (job `eval_lora`, `payload.node_id`)
- Test: `apps/api/tests/api.rs`

**Interfaces:**
- Consumes: `lora_checkpoints`, `dataset_snapshots.content` (schema 1: `config.seed`, `test_prompt`, `samples[]`, `held_out[]`, `genre.id`, `base_model.id`, `lora.key`) từ P2; completion `train_lora` ở `worker.rs`.
- Produces:
  - Migration:

```sql
ALTER TABLE lora_checkpoints ADD COLUMN delete_requested_at TEXT;
ALTER TABLE lora_checkpoints ADD COLUMN style_score REAL;
ALTER TABLE lora_checkpoints ADD COLUMN train_score REAL;
ALTER TABLE lora_checkpoints ADD COLUMN train_match TEXT;
ALTER TABLE lora_checkpoints ADD COLUMN melody_score REAL;
ALTER TABLE lora_checkpoints ADD COLUMN melody_match TEXT;
ALTER TABLE lora_checkpoints ADD COLUMN quality_score REAL;
ALTER TABLE loras ADD COLUMN chosen_checkpoint_id TEXT REFERENCES lora_checkpoints(id);
ALTER TABLE loras ADD COLUMN base_key TEXT;
CREATE TABLE lora_eval_renders (
  id TEXT PRIMARY KEY, lora_id TEXT NOT NULL REFERENCES loras(id),
  job_id TEXT NOT NULL REFERENCES jobs(id), checkpoint_id TEXT REFERENCES lora_checkpoints(id),
  kind TEXT NOT NULL CHECK (kind IN ('base','prompt','held_out')),
  sample_filename TEXT, prompt TEXT NOT NULL, artifact_id TEXT NOT NULL REFERENCES artifacts(id),
  scores TEXT NOT NULL DEFAULT '{}', created_at TEXT NOT NULL,
  UNIQUE (job_id, artifact_id));
```

  - `jobs::build_generation(state: &AppState, user: &User, req: &GenerationRequest) -> AppResult<BuiltGeneration>` với `pub(crate) struct BuiltGeneration { payload: Value, request: Value, model_id: String, lora_id: Option<String>, title: String }`. `create_generation` gọi nó; hành vi không đổi.
  - `lora_review::enqueue_eval(tx, lora_id: &str, train_job_id: &str, node_id: &str) -> AppResult<String>` (job id). Gọi trong transaction complete `train_lora`, chỉ khi run có `snapshot_id`.
  - Payload `eval_lora`:

```json
{"node_id": "...", "lora": {"id": "...", "base_key": "<lora_key>"},
 "dataset_key": "<lora_key>", "scale": 0.7, "duration": 60, "seed": 123,
 "render_template": { "...": "payload generate từ build_generation, prompt rỗng" },
 "test_prompt": "...",
 "checkpoints": [{"id": "...", "epoch": 4, "key": "<lora_key>-e4", "path": "<node_path>", "sha256": "..."}],
 "train": [{"dataset_file": "s000.wav", "filename": "late_bus_home.mp3"}],
 "held_out": [{"filename": "rainy_window.mp3", "prompt": "<train_caption>"}]}
```

  - `checkpoints` = tối đa 10 bản lưu chưa xóa có epoch lớn nhất, sắp theo epoch.
  - `build_generation` cho `render_template` dùng `GenerationRequest { prompt: test_prompt, genre_id: genre của catalog, model_id: base model, duration: 60 (kẹp vào khoảng của genre), seed: Some(config.seed), lora_id: None, .. }`, chủ job = người tạo bản chụp.
  - Claim (`worker.rs`): thêm điều kiện `(json_extract(j.payload,'$.node_id') IS NULL OR json_extract(j.payload,'$.node_id') = ?1)`, vẫn trong một câu SQL.
  - Hằng: `EVAL_PRIORITY = -30`, `FILES_PRIORITY = 10`, `TRAIN_PRIORITY = -40` (job `train_lora` mới từ `create_build`).
  - `plan_build`: `eval_renders = min(saves, 10) × (1 + held_out) + 1`.

- [ ] **Step 1: Viết test lỗi** trong `apps/api/tests/api.rs`, dùng các helper P2 (`lofi_catalog`, `three_tracks`, `put_caption`, node train giả):
  - `eval_job_is_queued_when_snapshot_run_completes`: complete `train_lora` có bản chụp với 2 checkpoint → có đúng 1 job `eval_lora` `queued`, `priority = -30`, `payload.node_id` = node đã train, `payload.checkpoints[*].key` = `["<lora_key>-e1","<lora_key>-e2"]`, `payload.duration = 60`, `payload.scale = 0.7`, `payload.held_out[0].prompt` = `train_caption` của bài giữ lại.
  - `no_eval_job_for_legacy_or_failed_runs`: run kiểu cũ complete → 0 job `eval_lora`; run có bản chụp bị `fail`/cancel → 0 job.
  - `eval_payload_keeps_last_ten_checkpoints`: 12 checkpoint → payload có 10, epoch 3..12.
  - `pinned_job_is_claimed_only_by_its_node`: job có `payload.node_id = A`; node B claim → 204/không có job; node A claim → nhận.
  - `new_train_jobs_have_priority_minus_40` và preview `eval_renders` với `epochs=12, save_every=1, holdout=2` = `10*3+1 = 31`.
  - Test sẵn có của generate vẫn qua (đảm bảo `build_generation` không đổi hành vi).

- [ ] **Step 2: Chạy để thấy lỗi** — `cd apps/api; cargo test --test api eval_ pinned_ new_train_` → FAIL (chưa có job/cột).

- [ ] **Step 3: Viết migration, `build_generation`, `enqueue_eval`, điều kiện claim, hằng ưu tiên** như Interfaces. Cập nhật `WORKER_PROTOCOL.md` (payload `eval_lora`, `node_id`).

- [ ] **Step 4: Chạy lại** — `cargo test` (toàn bộ) → PASS; `cargo clippy --all-targets` sạch.

- [ ] **Step 5: Commit** — `feat(api): queue eval_lora after snapshot training; pin jobs to a node`.

---

### Task 2: API – Hoàn tất `eval_lora`, chi tiết run cho màn hình chọn bản

**Files:**
- Modify: `apps/api/src/routes/lora_review.rs`, `worker.rs` (complete + `artifact_type`), `training.rs` (chi tiết run)
- Modify: `docs/WORKER_PROTOCOL.md`, `docs/API_CONTRACTS.md`
- Test: `apps/api/tests/api.rs`

**Interfaces:**
- Consumes: bảng/cột Task 1.
- Produces:
  - Upload artifact: kind khớp `^eval_[0-9]{2}$`, chỉ đuôi `mp3` (`audio/mpeg`), chỉ cho job `eval_lora`.
  - Kết quả worker gửi khi complete:

```json
{"renders": [{"artifact_kind": "eval_00", "kind": "base", "checkpoint_id": null, "sample_filename": null,
              "prompt": "...", "scores": {"mert_style": null, "mert_train": null, "train_match": null,
              "melody": 0.41, "melody_match": "late_bus_home.mp3", "quality": null}}],
 "checkpoints": [{"id": "...", "style": 0.82, "train": 0.71, "train_match": "a.mp3",
                  "melody": 0.44, "melody_match": "b.mp3", "quality": 6.9}],
 "scorers": {"mert": false, "audiobox": false}}
```

  - `lora_review::complete_eval(tx, job, result) -> AppResult<()>`: mọi `artifact_kind` phải là artifact đã upload của job; `checkpoint_id` phải thuộc LoRA của job; sai → 422 "kết quả đánh giá không hợp lệ" và không ghi gì. Ghi `lora_eval_renders`, cập nhật cột điểm ở `lora_checkpoints`. Điểm `null` giữ `null`.
  - `GET /training-runs/{id}` thêm:
    - `lora`: `{id, name, key, base_key, status, chosen_checkpoint_id}` hoặc null.
    - `eval`: `{job_id, status, progress, stage, error, scorers}` hoặc null (job `eval_lora` mới nhất của LoRA).
    - `renders`: `[{id, kind, checkpoint_id, sample_filename, prompt, artifact_id, scores}]`.
    - `checkpoints[]` thêm `note, expires_at, delete_requested_at, style_score, train_score, train_match, melody_score, melody_match, quality_score`.
    - `held_out_tracks`: `[{filename, artifact_id}]` lấy từ `held_out` của bản chụp.
    - `style_preset`: `{id, name, lora_scale, is_active}` hoặc null (Style `kind='trained'` gắn LoRA này).
    - `files_job`: `{job_id, status, op, error}` của job `lora_files` mới nhất, hoặc null.

- [ ] **Step 1: Viết test lỗi:**
  - `eval_complete_records_renders_and_scores`: claim job `eval_lora`, upload `eval_00`..`eval_04` (mp3 giả), complete với 1 base + 2 checkpoint × 2 render → detail có 5 `renders` với `artifact_id`, checkpoint epoch 2 có `style_score = 0.82`, `train_match = "a.mp3"`.
  - `eval_complete_rejects_unknown_artifact_or_checkpoint`: `artifact_kind` chưa upload → 422, không có dòng `lora_eval_renders`; `checkpoint_id` của LoRA khác → 422.
  - `eval_upload_kind_rules`: `eval_7` → 422; `eval_07` với `.wav` → 422; `eval_07` vào job `generate` → 422.
  - `run_detail_exposes_review_fields_without_paths`: detail có `lora`, `eval`, `held_out_tracks[0].artifact_id`; body không chứa `node_path` hay chuỗi đường dẫn checkpoint.
  - `eval_with_missing_scorers_keeps_nulls`: complete với `quality: null` → `quality_score` null, `eval.scorers.audiobox = false`.

- [ ] **Step 2: Chạy để thấy lỗi** — `cargo test --test api eval_ run_detail_exposes` → FAIL.
- [ ] **Step 3: Viết** `artifact_type` cho `eval_NN`, `complete_eval`, các trường mới của chi tiết run; sửa hai hợp đồng.
- [ ] **Step 4: Chạy lại** — `cargo test`, `cargo clippy --all-targets` → PASS/sạch.
- [ ] **Step 5: Commit** — `feat(api): record eval_lora renders and scores; review fields on run detail`.

---

### Task 3: API – Ghi chú, đưa vào Style, xóa ngay, dọn theo hạn, xóa LoRA

**Files:**
- Modify: `apps/api/src/routes/lora_review.rs`, `worker.rs` (complete `lora_files`, heartbeat), `lib.rs`
- Modify: `docs/API_CONTRACTS.md`, `docs/WORKER_PROTOCOL.md`
- Test: `apps/api/tests/api.rs`

**Interfaces:**
- Consumes: Task 1–2.
- Produces (mọi route dùng `Trainer`):
  - `PATCH /lora-checkpoints/{id}` body `{note}`: ≤ 500 ký tự, nếu không 422 "Ghi chú tối đa 500 ký tự". Trả checkpoint.
  - `POST /loras/{id}/publish` body `{checkpoint_id, style_name, lora_scale}` → 202 `{job_id, style_preset_id}`. Kiểm theo thứ tự:
    1. LoRA tồn tại, không `archived` (404 / 422 "LoRA đã bị xóa").
    2. Có job `lora_files` của LoRA đang `queued|preparing|running` → 409 "LoRA đang được cập nhật trên máy, chờ một chút".
    3. Checkpoint thuộc LoRA, `status != 'deleted'`, `delete_requested_at IS NULL` → nếu không 422 "Bản lưu này không còn trên máy".
    4. `style_name` 1–80 ký tự (422 "Tên Style phải có 1–80 ký tự"); `lora_scale` trong 0–1 (422 "Độ mạnh phải trong khoảng 0–1").
  - Trong một transaction:
    - Bản chọn: `status='chosen'`, `expires_at=NULL`. Bản `chosen` cũ (nếu khác) và mọi bản `kept`: `status='kept'`, `expires_at = now + 30 ngày` (bản `kept` đã có `expires_at` giữ nguyên).
    - `loras`: `key = <base_key>-e<epoch>` (`base_key` lấy từ cột, rơi về `key` hiện tại nếu NULL và ghi lại vào `base_key`), `chosen_checkpoint_id`. Xóa `lora_installations` của LoRA.
    - Style `kind='trained'` có `lora_id` này: có thì cập nhật `name`, `lora_scale`; chưa có thì tạo mới với `is_active=0`, genre của catalog, `model_id` = base model, `prompt_template` = từ khóa hoặc `''`.
    - Job `lora_files` ưu tiên 10, `payload = {node_id, lora_id, op: "publish", checkpoint: {id, path, sha256}, target_key}`.
  - `POST /loras/{id}/cleanup` → 202 `{job_id, count}`. Chưa có bản `chosen` → 409 "Chọn một bản trước khi xóa các bản còn lại". Không có bản nào để xóa → 200 `{job_id: null, count: 0}`. Còn lại: đánh dấu `delete_requested_at` cho mọi bản `kept` chưa đánh dấu, tạo job `op: "delete"` với `checkpoints: [{id, path}]`.
  - `DELETE /loras/{id}` → 204. Theo R10: 409 "Còn Style dùng LoRA này: <tên, cách nhau bởi dấu phẩy>" hoặc 409 "LoRA đang có việc chạy trên máy". Nếu qua: `status='archived'`, đánh dấu xóa mọi bản chưa xóa, tạo job `delete` (nếu có bản để xóa).
  - Hoàn tất `lora_files` (`lora_review::complete_files`):
    - `op=publish`, result `{published: {key, sha256}}`: `key` phải bằng `target_key` và `sha256` bằng của checkpoint, nếu không 422. Ghi `lora_installations(node, lora)`, `loras.status='approved'`, Style của LoRA `is_active=1`.
    - `op=delete`, result `{deleted: [checkpoint_id], missing: [checkpoint_id]}`: cả hai nhóm chuyển `status='deleted'`.
  - Heartbeat: nếu node không có job `lora_files` nào `queued|preparing|running`, tìm bản lưu `status='kept'`, `expires_at < now`, `delete_requested_at IS NULL` của LoRA train trên node đó (`loras.job_id → jobs.node_id`), đánh dấu và tạo một job `delete`. Chủ job = người tạo LoRA.

- [ ] **Step 1: Viết test lỗi:**
  - `publish_sets_chosen_creates_inactive_style_and_files_job`: publish epoch 2 → 202; checkpoint epoch 2 `chosen`, epoch 1 `kept` với `expires_at` ≈ now+30d; `loras.key` = `<lora_key>-e2`; Style mới `is_active=false`, `lora_scale=0.7`; job `lora_files` payload `op="publish"`, `target_key="<lora_key>-e2"`.
  - `publish_twice_while_files_job_live_is_409` (Review Focus 1): không có Style thứ hai.
  - `files_publish_complete_approves_and_activates`: complete với đúng key/sha → `loras.status='approved'`, Style `is_active=true`, có `lora_installations`; sha sai → 422, LoRA vẫn `evaluation`.
  - `changing_epoch_updates_same_style`: publish e2, complete, publish e1 → vẫn 1 Style, `loras.key` = `-e1`, e2 thành `kept` có `expires_at`, installations rỗng đến khi complete.
  - `cleanup_requires_a_chosen_checkpoint` (Review Focus 2) và `cleanup_marks_kept_and_queues_delete`: complete `op=delete` → các bản `deleted`, bản `chosen` không đổi.
  - `heartbeat_queues_delete_for_expired_checkpoints`: đặt `expires_at` về quá khứ, gửi heartbeat → 1 job `delete`; heartbeat lần hai khi job còn chờ → không tạo thêm.
  - `delete_lora_blocked_by_style_then_archives`: còn Style → 409 có tên Style; xóa Style (`is_active` không đủ — xóa bằng `PATCH /presets/{id}` gỡ `lora_id`), rồi `DELETE` → 204, `status='archived'`, job `delete` gồm cả bản chosen.
  - `note_is_saved_and_limited`: 501 ký tự → 422.

- [ ] **Step 2: Chạy để thấy lỗi** — `cargo test --test api publish_ cleanup_ heartbeat_queues delete_lora note_is` → FAIL.
- [ ] **Step 3: Viết** các route, `complete_files`, nhánh heartbeat; sửa hai hợp đồng.
- [ ] **Step 4: Chạy lại** — `cargo test`, clippy → PASS.
- [ ] **Step 5: Commit** — `feat(api): choose a checkpoint into a Style Preset, cleanup and archive`.

---

### Task 4: API – Thử nhanh

**Files:**
- Modify: `apps/api/src/routes/lora_review.rs`, `lib.rs`, `docs/API_CONTRACTS.md`, `docs/WORKER_PROTOCOL.md`
- Test: `apps/api/tests/api.rs`

**Interfaces:**
- Consumes: `build_generation` (Task 1).
- Produces:
  - `POST /loras/{id}/try` body `{checkpoint_id, prompt, lora_scale}` → 201 `Job` (job `generate`, chủ là người bấm).
  - Kiểm: prompt 1–500 ký tự ("Câu lệnh phải có 1–500 ký tự"); checkpoint thuộc LoRA, chưa xóa ("Bản lưu này không còn trên máy"); `lora_scale` 0–1.
  - Payload = `build_generation(GenerationRequest { prompt, genre_id: genre catalog, model_id: base model, duration: 60, lora_id: None, .. })`, rồi thêm `payload.lora = {key: "<base_key>-e<epoch>", scale, path: node_path}` và `payload.node_id`. `jobs.lora_id` để NULL. Tiêu đề: "Thử nhanh · <tên LoRA> · Epoch N".

- [ ] **Step 1: Viết test lỗi:** `quick_try_creates_pinned_generate_job` (payload `lora.path` = node_path, `node_id` đúng, `jobs.lora_id` NULL; claim bởi node khác → không nhận); `quick_try_rejects_deleted_checkpoint`; `quick_try_job_detail_hides_path` (body của `GET /jobs/{id}` và `GET /jobs` của chủ job không chứa chuỗi node_path; nếu payload đang được trả ra web thì bỏ `lora.path` khỏi phần trả về).
- [ ] **Step 2: Chạy để thấy lỗi** — `cargo test --test api quick_try` → FAIL.
- [ ] **Step 3: Viết** route.
- [ ] **Step 4: Chạy lại** — `cargo test`, clippy → PASS.
- [ ] **Step 5: Commit** — `feat(api): quick try a checkpoint as a pinned generate job`.

---

### Task 5: Worker – LoRA theo đường dẫn, job `lora_files`

**Files:**
- Modify: `workers/music-worker/src/lush_worker/engines/acestep.py`, `runner.py`, `config.py`
- Create: `workers/music-worker/src/lush_worker/lora_files.py`
- Modify: `docs/WORKER_PROTOCOL.md`
- Test: `workers/music-worker/tests/test_acestep.py` (hoặc file test engine hiện có), `tests/test_lora_files.py`

**Interfaces:**
- Consumes: payload Task 1/3/4; `training_dir` của `SideStepTrainer` (`<DATA_DIR>/training/<job_id>/out/...`, xác nhận trong `training.py`).
- Produces:
  - `AceStepEngine(..., checkpoint_root: Optional[Path] = None)`. Khi `payload.lora.path` có mặt: path resolve phải nằm dưới `checkpoint_root` và có `adapter_config.json`, nếu không raise `AceStepError("LoRA path is outside the training directory")`. Adapter name vẫn từ `key`.
  - `lora_files.run(payload, ctx, *, checkpoint_root: Path, lora_dir: Path) -> dict`:
    - `publish`: kiểm path dưới `checkpoint_root`, SHA-256 của `adapter_model.safetensors` = `checkpoint.sha256`, rồi chép `adapter_config.json` + `adapter_model.safetensors` vào `lora_dir/<target_key>` qua thư mục `.partial` + rename. `target_key` đã tồn tại với cùng SHA → coi như xong; khác SHA → lỗi, không ghi đè. Trả `{"published": {"key", "sha256"}}`.
    - `delete`: với mỗi checkpoint, path phải dưới `checkpoint_root` → `shutil.rmtree`; không tồn tại → vào `missing`. Path ngoài root → lỗi cả job, không xóa gì (kiểm hết trước rồi mới xóa). Trả `{"deleted": [...], "missing": [...]}`.
  - Runner: báo job type `lora_files` khi có trainer; dispatch.

- [ ] **Step 1: Viết test lỗi:**
  - `test_lora_path_inside_root_is_loaded` (fake client ghi nhận `lora_load` với path đó) và `test_lora_path_outside_root_is_rejected` (Review Focus 3: không gọi `lora_load`).
  - `test_publish_copies_adapter_atomically`, `test_publish_same_sha_is_idempotent`, `test_publish_refuses_different_existing_target`, `test_publish_bad_sha_fails_without_target`.
  - `test_delete_removes_dirs_and_reports_missing`, `test_delete_with_outside_path_deletes_nothing`.
- [ ] **Step 2: Chạy để thấy lỗi** — `.venv/Scripts/python -m pytest -q tests/test_lora_files.py -k "lora_path or publish or delete"` → FAIL.
- [ ] **Step 3: Viết** như Interfaces; cập nhật `WORKER_PROTOCOL.md`.
- [ ] **Step 4: Chạy lại** — `pytest -q` toàn bộ → PASS.
- [ ] **Step 5: Commit** — `feat(worker): load LoRA checkpoints by path; lora_files publish/delete`.

---

### Task 6: Worker – Hàm chấm và client máy chấm

**Files:**
- Create: `workers/music-worker/src/lush_worker/scoring.py`, `tests/test_scoring.py`, `tests/fake_scorer.py`
- Modify: `config.py` (`SCORER_DIR`), `runner.py` (báo model `kind: "scorer"` khi register, như captioner), `docs/WORKER_PROTOCOL.md` (mục "Scorer subprocess")

**Interfaces:**
- Produces:
  - Protocol tiến trình con: `SCORER_DIR/<name>/scorer.json` = `{"key", "name", "command": [...], "provides": "embedding" | "quality"}`. Stdin/stdout JSON lines: `{"path": "<wav>"}` → `{"embedding": [float...]}` hoặc `{"quality": float}`; lỗi → `{"error": "..."}`. Stdout chỉ chứa JSON (như captioner P1).
  - `class Scorer` (`start()`, `score(path) -> list[float] | float`, `close()`, timeout mỗi file 300 s) và `load_scorers(scorer_dir) -> dict[str, ScorerSpec]`.
  - Hàm thuần (numpy):
    - `cosine(a, b) -> float`
    - `style_score(render_embs, train_embs) -> float` (R7)
    - `best_match(render_emb, train_embs, names) -> tuple[float, str]`
    - `melody_similarity(chroma_a, chroma_b) -> float` (DTW cosine, `1 - cost/len(path)`, cắt [0,1]); `chroma(path, seconds=60) -> np.ndarray` (librosa, import lười như `analysis.py`)
    - `memorized(train: float | None, melody: float | None) -> bool` với `MERT_MEMORIZE = 0.95`, `MELODY_MEMORIZE = 0.80` (comment "chưa hiệu chỉnh").

- [ ] **Step 1: Viết test lỗi:** `test_cosine_and_style_score` (vector tay, giá trị đúng tới 1e-6), `test_best_match_returns_name`, `test_melody_similarity_identical_is_one` và `..._unrelated_is_lower` (chroma tổng hợp), `test_memorized_thresholds` (0.95/0.80 biên), `test_scorer_subprocess_roundtrip` (dùng `fake_scorer.py`: trả embedding cố định), `test_scorer_error_line_raises`, `test_register_reports_installed_scorers`.
- [ ] **Step 2: Chạy để thấy lỗi** — `pytest -q tests/test_scoring.py` → FAIL.
- [ ] **Step 3: Viết** `scoring.py`, `SCORER_DIR`, báo scorer khi register.
- [ ] **Step 4: Chạy lại** — `pytest -q` → PASS.
- [ ] **Step 5: Commit** — `feat(worker): scoring helpers and scorer subprocess protocol`.

---

### Task 7: Worker – Job `eval_lora`

**Files:**
- Create: `workers/music-worker/src/lush_worker/evaluation.py`, `tests/test_evaluation.py`
- Modify: `runner.py`, `docs/WORKER_PROTOCOL.md`

**Interfaces:**
- Consumes: payload Task 1, kết quả Task 2, `AceStepEngine.generate` + `checkpoint_root` (Task 5), `scoring` (Task 6), `audio.make_mp3_preview`, `InferenceService` (restart, R11), `api.upload_artifact`.
- Produces: `run_eval(ctx, payload, *, engine, api, dataset_dir: Path, scorers: dict, service) -> dict` (kết quả đúng dạng Task 2). Thứ tự:
  1. Kiểm mọi `train[].dataset_file` có trong `dataset_dir/<dataset_key>/` và mọi checkpoint path dưới `checkpoint_root`, **trước** khi render.
  2. Render theo thứ tự: base (không `lora`) → với mỗi checkpoint: `test_prompt` rồi từng `held_out`. Mỗi render: copy `render_template`, đặt `prompt`, `seed`, `duration`, `lora = {key, scale, path}` (base thì bỏ `lora`), gọi `engine.generate` với context con có `workdir` riêng. Đổi sang mp3, upload kind `eval_%02d` theo thứ tự. Tiến độ: `stage="rendering"`, `progress = i / tổng`, message "Tạo nhạc nghe thử i/N".
  3. Chấm (`stage="scoring"`): giai điệu luôn chấm; MERT/Audiobox nếu có trong `scorers`, nếu không thì `null` và `scorers.{mert,audiobox} = false`.
  4. Nếu đã nạp ít nhất một adapter: `service.stop()` rồi `service.start()` trong `finally` (R11), như `SideStepTrainer` đang làm.
  - Hủy giữa chừng (`ctx.raise_if_aborted()` giữa các render) → job dừng, vẫn restart service.

- [ ] **Step 1: Viết test lỗi** (engine giả ghi nhận payload, ghi WAV ngắn; api giả ghi nhận upload):
  - `test_eval_renders_in_order_and_uploads`: 2 checkpoint, 1 held_out → 5 render; kind `eval_00`..`eval_04`; render 0 không có `lora`; render 1 `lora.key` = `...-e1`, `scale=0.7`, `prompt` = test_prompt; render 2 `prompt` = held_out prompt; mọi render `seed` và `duration=60`.
  - `test_eval_without_scorers_has_melody_only` (Review Focus 4): `checkpoints[*].melody` là số, `style/train/quality` `None`, `scorers == {"mert": False, "audiobox": False}`.
  - `test_eval_with_fake_scorers_computes_scores` (fake MERT trả embedding theo tên file).
  - `test_eval_missing_dataset_file_fails_before_rendering`.
  - `test_eval_restarts_inference_after_loading_adapters` và `test_eval_cancel_between_renders_still_restarts`.
- [ ] **Step 2: Chạy để thấy lỗi** — `pytest -q tests/test_evaluation.py` → FAIL.
- [ ] **Step 3: Viết** `evaluation.py`, đăng ký job type `eval_lora` khi node có trainer (và engine acestep).
- [ ] **Step 4: Chạy lại** — `pytest -q` → PASS.
- [ ] **Step 5: Commit** — `feat(worker): eval_lora renders listening samples and scores them`.

---

### Task 8: Worker – Script máy chấm MERT và Audiobox (chưa chạy thật)

**Files:**
- Create: `workers/music-worker/scorers/mert/{scorer.py,scorer.json,README.md}`, `workers/music-worker/scorers/audiobox/{scorer.py,scorer.json,README.md}`, `tests/test_scorer_scripts.py`

**Interfaces:**
- Consumes: protocol Task 6.
- Produces:
  - `mert/scorer.py`: model `m-a-p/MERT-v1-330M` (transformers, `trust_remote_code`), resample 24 kHz, 60 s đầu, trung bình hidden state lớp cuối theo thời gian → embedding. `provides: "embedding"`.
  - `audiobox/scorer.py`: gói `audiobox_aesthetics` (`initialize_predictor`), trả `PQ`. `provides: "quality"`.
  - Cả hai: tách stdout như captioner P1 (in của thư viện đi stderr), mỗi dòng vào → đúng một dòng ra. README ghi lệnh cài venv riêng dưới `AI_DATA/AiStudio/models/<tên>/`, dung lượng ước tính, LICENSE cần người dùng duyệt, và dòng "Chưa chạy thật".
- [ ] **Step 1: Viết test lỗi:** `test_scripts_speak_protocol_with_stub_model` — chạy mỗi script bằng subprocess với biến môi trường `LUSH_SCORER_STUB=1` (script dùng model giả, không import torch), gửi 2 dòng → nhận 2 dòng JSON đúng khóa; một dòng path không tồn tại → `{"error": ...}` và tiến trình vẫn sống.
- [ ] **Step 2: Chạy để thấy lỗi** — `pytest -q tests/test_scorer_scripts.py` → FAIL.
- [ ] **Step 3: Viết** hai script + `scorer.json` + README. Phần nạp model thật ghi comment "chưa kiểm chứng".
- [ ] **Step 4: Chạy lại** — `pytest -q` → PASS.
- [ ] **Step 5: Commit** — `feat(worker): MERT and Audiobox scorer scripts (unverified on GPU)`.

---

### Task 9: Web – Kiểu, hook, hàm thuần

**Files:**
- Modify: `apps/web/src/api/{types,client,hooks,mutations}.ts`
- Create: `apps/web/src/features/lora/review.ts`, `review.test.ts`

**Interfaces:**
- Consumes: JSON Task 2–4 (tên trường đúng như Rust).
- Produces:
  - Types: `EvalRender`, `EvalInfo`, `ReviewLora`, `HeldOutTrack`, `StylePresetRef`, `FilesJob`; `Checkpoint` thêm các trường Task 2; `TrainingRunDetail` thêm `lora, eval, renders, held_out_tracks, style_preset, files_job`.
  - Mutations (invalidate `['training-run', jobId]`): `useSaveCheckpointNote()`, `usePublishCheckpoint(loraId)`, `useCleanupCheckpoints(loraId)`, `useDeleteLora(loraId)`, `useQuickTry(loraId)`.
  - `review.ts`:

```ts
export type StepState = 'done' | 'active' | 'waiting' | 'failed'
export interface ReviewStep { title: string; state: StepState; detail: string }
export function reviewSteps(run: TrainingRunDetail): ReviewStep[] // 4 bước, tiêu đề đúng spec 6.4
export function checkpointVerdict(c: Checkpoint, all: Checkpoint[]): { text: string; tone: 'warn' | 'good' | 'muted' }
export function rendersFor(run: TrainingRunDetail, checkpointId: string | null): EvalRender[]
export const MEMORIZE = { mert: 0.95, melody: 0.8 } // khớp worker, "chưa hiệu chỉnh"
```

  - Tiêu đề bước: "Chuẩn bị dữ liệu", "Train", "Tạo nhạc nghe thử", "Bạn nghe và chọn bản". Bước 4 `detail`: "Chờ bạn" khi chưa chọn; "Đã đưa Epoch N vào Style Preset" khi Style `is_active`.
  - `checkpointVerdict`: học thuộc → `"Có dấu hiệu học thuộc: gần giống <match>"` (warn); điểm style cao nhất trong các bản → `"Giống style nhất"` (good); không có điểm MERT → `"Chưa cài máy chấm"` (muted); còn lại `""`.
- [ ] **Step 1: Viết test lỗi** cho `reviewSteps` (train xong + eval đang chạy → bước 3 `active` với "i/N bài"; eval failed → bước 3 `failed`), `checkpointVerdict` (3 trường hợp + ngưỡng biên), `rendersFor`, và client: `publishCheckpoint` POST `/loras/{id}/publish`.
- [ ] **Step 2: Chạy để thấy lỗi** — `npx vitest run src/features/lora/review.test.ts` → FAIL.
- [ ] **Step 3: Viết** như Interfaces.
- [ ] **Step 4: Chạy lại** — `npm test`, `npm run build` → PASS.
- [ ] **Step 5: Commit** — `feat(web): review types, hooks and pure helpers`.

---

### Task 10: Web – Màn hình chọn bản trên TrainingRunPage

**Files:**
- Create: `apps/web/src/features/lora/{EvalTable,PublishPanel,QuickTryPanel}.tsx` (+ test)
- Modify: `apps/web/src/pages/TrainingRunPage.tsx` (+ test), `features/training/LossChart.tsx`, CSS trang

**Interfaces:**
- Consumes: Task 9; `SnapshotPanel` (P2) giữ ở cột phải; `ModelBar`.
- Produces: bố cục theo `after-train.png`, áp dụng khi `run.lora` khác null (run kiểu cũ giữ nguyên trang hiện tại):
  - **Đầu trang:** tên LoRA + badge trạng thái; nút "Train lại với tùy chọn khác" (link `/catalogs/{catalog_id}?retrain={snapshot_id}`) và "Xóa LoRA" (confirm "Xóa LoRA <tên>? Hồ sơ được lưu trữ, các bản lưu trên máy sẽ bị xóa."; lỗi 409 hiện nguyên thông báo API).
  - **4 thẻ bước** từ `reviewSteps`.
  - **Bảng bản lưu** (`EvalTable`): dòng "Bài mẫu" đầu bảng (render base + bài thật của từng bài giữ lại, phát từ `held_out_tracks[].artifact_id`); mỗi epoch một dòng: "Epoch N · bước S", render "LoRA" của câu lệnh nghe thử, các render "LoRA tạo" cho bài giữ lại (hiện bài đầu, "+ N bài nữa" mở rộng), ba thanh "Giống style", "Giống bài train", "Chất lượng âm thanh" (thanh trống + "—" khi `null`), dòng nhận xét `checkpointVerdict`, ô ghi chú (lưu khi blur, placeholder "Ghi chú…"). Bấm dòng để chọn; dòng chosen có badge "Đang chọn". Bản đã xóa: mờ, ghi "Đã xóa khỏi máy", không chọn được. Dưới bảng: "Bấm vào một dòng để chọn bản đó. Các thanh điểm là máy so sánh âm thanh, chỉ để tham khảo; quyết định là tai bạn."
  - **Cột phải:**
    - `PublishPanel` "Đưa vào Style Preset": "Bản chọn: Epoch N", ô "Tên hiện ở Style Preset" (mặc định tên Style hiện có hoặc tên LoRA), thanh "Độ mạnh mặc định" 0–1 bước 0.05 mặc định 0.7 với nhãn "Nhẹ"/"Mạnh", nút "Đưa Epoch N vào Style Preset" (disabled khi chưa chọn hoặc `files_job` đang chạy, khi đó hiện "Đang cập nhật trên máy…"). Ghi chú "Các bản không chọn được giữ 30 ngày rồi tự xóa để đỡ tốn ổ." + nút "Xóa ngay" (confirm, gọi cleanup). `files_job` lỗi → hiện lỗi và cho bấm lại.
    - `QuickTryPanel` "Thử nhanh với câu lệnh của bạn": textarea (mặc định `test_prompt`), chip "Epoch N" và "Độ mạnh X", nút "Tạo thử" → tạo job, hiện link "Xem trong Hàng đợi".
    - "LoRA này được train từ" (P2).
  - **LossChart:** thêm prop `epochSteps?: number[]` vẽ vạch dọc và nhãn "Epoch N" tại `step` của từng checkpoint.
- [ ] **Step 1: Viết test lỗi** (mock fetch): chọn dòng epoch 2 rồi bấm "Đưa Epoch 2 vào Style Preset" gửi đúng body `{checkpoint_id, style_name, lora_scale: 0.7}`; ghi chú blur gửi PATCH; "Xóa ngay" khi chưa chọn hiện thông báo 409 từ API; điểm `null` hiện "—" và "Chưa cài máy chấm"; dòng học thuộc hiện "Có dấu hiệu học thuộc: gần giống late_bus_home.mp3"; run kiểu cũ không hiện bảng; "Train lại với tùy chọn khác" có href đúng; LossChart với `epochSteps` vẽ 2 vạch.
- [ ] **Step 2: Chạy để thấy lỗi** — `npx vitest run src/pages/TrainingRunPage.test.tsx src/features/lora` → FAIL.
- [ ] **Step 3: Viết** các component và ghép trang. Poll 2 s khi `eval` hoặc `files_job` đang chạy (như trang run hiện nay).
- [ ] **Step 4: Chạy lại** — `npm test`, `npm run build` → PASS.
- [ ] **Step 5: Commit** — `feat(web): after-train review screen (listen, note, choose, publish)`.

---

### Task 11: Web – Train lại; tài liệu và toàn bộ test

**Files:**
- Modify: `apps/web/src/pages/CatalogPage.tsx`, `features/lora/CreateLoraDialog.tsx`, `features/lora/createLoraForm.ts` (+ test)
- Modify: `docs/tasks/CURRENT.md` (T-017 🟡), `docs/DECISIONS.md` (D33: R1–R12), `docs/CHANGELOG.md`, `docs/TESTING.md` nếu cần

**Interfaces:**
- Consumes: `useDatasetSnapshot` (P2), `DatasetSnapshot`.
- Produces:
  - `stateFromSnapshot(s: DatasetSnapshot, genreName: string): CreateLoraState` trong `createLoraForm.ts`: cấu hình, từ khóa, câu lệnh nghe thử, `holdoutSeed`, số bài giữ lại lấy từ bản chụp; tên = `"<tên cũ> (2)"`; `kind` suy từ epochs (1/4/8 → quick/standard/thorough, khác → custom).
  - CatalogPage đọc `?retrain=<snapshot_id>`: chọn các bài của bản chụp (train + giữ lại) còn trong catalog, mở hộp thoại với `initial`. Bài đã bị xóa khỏi catalog → Notice "N bài trong lần train trước không còn trong catalog."
- [ ] **Step 1: Viết test lỗi:** `stateFromSnapshot` (epochs 4 → `standard`, tên "(2)", holdoutSeed giữ nguyên); CatalogPage với `?retrain=` mở hộp thoại có sẵn tên và chọn đúng bài, báo bài thiếu.
- [ ] **Step 2: Chạy để thấy lỗi** — `npx vitest run src/features/lora/createLoraForm.test.ts src/pages/CatalogPage.test.tsx` → FAIL.
- [ ] **Step 3: Viết**; rồi cập nhật tài liệu: T-017 🟡 (chỉ test tự động; còn cần duyệt: deploy migration 0008, tải MERT + Audiobox, một lượt eval thật trên rtx6000), D33, CHANGELOG.
- [ ] **Step 4: Chạy toàn bộ:** `cargo test` + `cargo clippy --all-targets` (apps/api), `.venv/Scripts/python -m pytest -q` (worker), `npm test` + `npm run build` (web). Ghi số test vào T-017.
- [ ] **Step 5: Commit** — `feat(web): train again from a snapshot; docs: LoRA Studio P3 status`.
