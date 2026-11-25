# ACE-Step XL SFT only + LM 4B Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `acestep-v15-xl-sft` the only music model (generation and LoRA training), remove Turbo everywhere, and add LM `acestep-5Hz-lm-4B` behind two switches (Thinking when generating, `acestep-lm` captioner when labelling).

**Architecture:** Website (API + web) stays model-agnostic; the worker learns LM availability from ACE-Step's `/v1/models` inventory (`llm_initialized`) and advertises it as capability `thinking` on the XL model plus a built-in captioner `acestep-lm`. LM labelling reuses the existing `caption_tracks` / `describe_input` jobs and calls ACE-Step `/release_task` with `full_analysis_only`. Ops steps on rtx6000 and the VPS run in a fixed order, each behind the user's go-ahead.

**Tech Stack:** Rust/Axum/SQLx (apps/api), React 19 + TS + Vitest (apps/web), Python + httpx + pytest (workers/music-worker, lush-training), ACE-Step 1.5 `ca1e85f` on rtx6000.

**Spec:** `docs/superpowers/specs/2026-11-25-acestep-xl-only-design.md`

## Global Constraints

- Model key: `acestep-v15-xl-sft`; trainer variant `xl_sft`; LM `acestep-5Hz-lm-4B`; captioner key `acestep-lm`, display name `ACE-Step LM 4B`.
- Default `inference_steps` 50 (no Turbo branch, no lower bound); default `guidance_scale` 7.
- `params.thinking` bool, default false; false ⇒ the ACE-Step request is byte-for-byte today's (`thinking: false`, CoT off). True ⇒ `thinking: true`, `use_cot_caption`/`use_cot_language` stay false.
- API error for thinking without LM: 422 "LM chưa bật trên máy render". Worker error when LM missing at run time: "LM chưa nạp trên máy render".
- `acestep-captioner` stays the default captioner and stays preferred by `describe_input` (D35).
- Caption from LM: `model_key = 'acestep-lm'`, `source = 'model'`, `source_ref` = loaded LM checkpoint name. No migration.
- AGENTS.md: no deploy, push, model download, GPU job, config/*.env edit or data deletion without the user's go-ahead for that step. Report mock vs real honestly. Behaviour changes update `docs/API_CONTRACTS.md` and `docs/WORKER_PROTOCOL.md` in the same change. Commits end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- rtx6000: verify `/media/ai-rtx6000/AI_DATA` is mounted from `/dev/nvme1n1p1` before any write; never format/remount. lushvietnix2: no model download, audio processing or training there.

## Review Focus

1. A Style Preset saved with `params.thinking: true`, then LM switched off on rtx6000 → generating from that preset must give the 422, not a GPU job that fails later. (Task 4 test.)
2. `params.thinking` sent as a non-bool (`"true"`, `1`) → 422 validation, not silently treated as on/off. (Task 4 test.)
3. ACE-Step inventory without `llm_initialized` key (older server) → worker treats LM as absent, no crash at register. (Task 2 test.)
4. LM analysis returns an empty `prompt` for a track → that track is counted as failed like other captioners, the job continues. (Task 3 test.)
5. Worker restarted with LM off after a Thinking job was queued → job fails non-retryable with "LM chưa nạp trên máy render". (Task 2 test.)

---

### Task 1: Ops spike on rtx6000 — XL SFT + LM 4B load, LoRA on XL (GATED)

Each numbered step needs the user's explicit go-ahead in chat before it runs. Real GPU work; record results in `docs/TESTING.md` as real.

- [ ] **Step 1:** `ssh rtx6000 'findmnt /media/ai-rtx6000/AI_DATA; df -h /media/ai-rtx6000/AI_DATA'` — Expected: source `/dev/nvme1n1p1`, ≥ 40 GB free.
- [ ] **Step 2 (gated: download ~28 GB):** download `ACE-Step/acestep-v15-xl-sft` and `ACE-Step/acestep-5Hz-lm-4B` into `ACE-Step-1.5/checkpoints/<same name>` with `huggingface-cli download` from the ACE-Step venv. Expected: 4 DiT safetensors shards + `config.json` + `silence_latent.pt`; LM dir has its safetensors and tokenizer files; sizes ≈ 19.95 GB / 8.43 GB.
- [ ] **Step 3 (gated: config edit):** back up `config/acestep.env` to `config/acestep.env.bak-2026-11-25`, set `ACESTEP_CONFIG_PATH=acestep-v15-xl-sft`, `ACESTEP_INIT_LLM=true`, `ACESTEP_LM_MODEL_PATH=acestep-5Hz-lm-4B`; restart `acestep-api` (user unit) and `lush-worker`. Expected: `curl /v1/models` shows `acestep-v15-xl-sft` `is_loaded: true`, `llm_initialized: true`, `loaded_lm_model: acestep-5Hz-lm-4B`; `nvidia-smi` VRAM noted. If start fails or VRAM leaves < 20 GB free: set `ACESTEP_INIT_LLM=false`, report, ask the user (spec "Ngưỡng bỏ LM").
- [ ] **Step 4:** one 30 s text2music `/release_task` with `inference_steps: 50`, no `shift`; then the same with `shift: 1.0` if the request model accepts it (grep `shift` in `acestep/api` request model). Record time, VRAM, whether results differ. Decision recorded in the ledger: send `shift: 1.0` explicitly (Task 2) only if the server default for XL SFT is not 1.0.
- [ ] **Step 5:** one `/release_task` with `full_analysis_only: true` and a Lo-fi MP3 as `src_audio`. Expected: result has non-empty `prompt`, `bpm`, `keyscale`. Save the raw result JSON to the scratchpad for Task 3's fixture.
- [ ] **Step 6 (gated: GPU training):** tiny LoRA spike via the trainer CLI (3 tracks, 1 epoch, `--model-variant xl_sft`, output under `data/training/spike-xl/`), stopping `acestep-api` during training as the worker does; restart it, `/v1/lora/load` the adapter, generate the same seed with LoRA on and off. Expected: load OK, `/v1/lora/status` shows the adapter, the two WAVs differ. **If load fails: stop the plan and ask the user.** Delete `spike-xl` afterwards (it is test output, not user data).
- [ ] **Step 7:** append the measurements to `docs/TESTING.md` (real, date, numbers) and commit.

### Task 2: Worker — XL defaults, Thinking, LM capability

**Files:**
- Modify: `workers/music-worker/src/lush_worker/engines/acestep.py` (lines 18, 48, 224, 287; `build_release_body`, `list_models`, new `lm_status`)
- Modify: `workers/music-worker/src/lush_worker/engines/base.py` (add `CAP_THINKING = "thinking"`)
- Test: `workers/music-worker/tests/test_acestep_engine.py`, `tests/test_training.py`, `tests/test_evaluation.py`, `tests/test_runner.py`

**Interfaces:**
- Produces: `DEFAULT_STEPS = 50`; `CAP_THINKING`; `AceStepEngine.lm_status() -> Optional[str]` (loaded LM model name or None; None on missing key or error); `list_models()` adds `"thinking"` to capabilities when `lm_status()` is not None; `build_release_body` honours `params.thinking`.

- [ ] **Step 1: failing tests** in `test_acestep_engine.py` (replace every Turbo fixture key with `acestep-v15-xl-sft`):
  - `test_default_steps_is_50_for_xl_sft`: `build_release_body(payload(params={}))["inference_steps"] == 50`.
  - `test_thinking_off_by_default`: body `thinking is False`, `use_cot_caption is False`.
  - `test_thinking_on_sets_thinking_only`: `params={"thinking": True}` ⇒ `thinking is True`, `use_cot_caption is False`, `use_cot_language is False`.
  - `test_list_models_adds_thinking_when_lm_loaded`: fake inventory `{"models":[{"name":"acestep-v15-xl-sft","is_loaded":True}],"llm_initialized":True,"loaded_lm_model":"acestep-5Hz-lm-4B"}` ⇒ capabilities contain `"thinking"`.
  - `test_list_models_without_llm_key`: inventory lacking `llm_initialized` ⇒ no `"thinking"`, no exception.
  - `test_generate_thinking_without_lm_fails_non_retryable` (runner or engine level, whichever owns the check): payload thinking true, `lm_status()` None ⇒ EngineError "LM chưa nạp trên máy render", not retryable.
  - Grep `turbo` in the three other worker test files and switch fixtures to the XL key.
- [ ] **Step 2:** `.venv\Scripts\python -m pytest -q tests/test_acestep_engine.py` — Expected: the new tests FAIL.
- [ ] **Step 3:** implement. `thinking` parsed strictly: bool only, else `AceStepError("invalid generate params: thinking must be true/false")`. Add `shift: 1.0` to the body only if Task 1 Step 4 decided so.
- [ ] **Step 4:** full worker suite `.venv\Scripts\python -m pytest -q` — Expected: all pass, and `grep -ri turbo src tests` in `workers/music-worker` finds nothing.
- [ ] **Step 5:** update `docs/WORKER_PROTOCOL.md` (base_model example key, capability `thinking`, `params.thinking`), commit `feat(worker): XL SFT defaults and LM thinking capability`.

### Task 3: Worker — `acestep-lm` captioner

**Files:**
- Create: `workers/music-worker/src/lush_worker/lm_caption.py`
- Modify: `engines/acestep.py` (`AceStepClient.analyze_audio`), `runner.py` (registration, `_execute_captioning`, `_execute_describe`)
- Test: `workers/music-worker/tests/test_lm_caption.py`, `tests/test_runner.py`

**Interfaces:**
- Consumes: `AceStepEngine.lm_status()` (Task 2), `captioning.extract_fields`, `captioning._post`-style per-track posting used by `run_caption_job`.
- Produces: `AceStepClient.analyze_audio(audio: Path, timeout: float) -> dict` (release `full_analysis_only`, file field `src_audio`, wait via `query_result`, return first result dict); `LM_CAPTIONER_KEY = "acestep-lm"`; `lm_captioner_model(lm_name: str) -> dict` (`{"engine":"captioner","key":"acestep-lm","name":"ACE-Step LM 4B","version":lm_name,"capabilities":["caption"]}`); `run_lm_caption_job(ctx, api, engine, payload) -> dict` (same result shape as `run_caption_job`); `run_lm_describe_job(ctx, engine, payload) -> dict` (same shape as `run_describe_job`: caption, mood, instruments, vocals, source_ref, tempo_bpm, key).

- [ ] **Step 1: failing tests** in `test_lm_caption.py` using the Task 1 Step 5 JSON as fixture (fake client):
  - `test_describe_maps_prompt_bpm_key`: caption == fixture `prompt`, `tempo_bpm` == fixture bpm, `key` == fixture keyscale, `source_ref == "acestep-lm@acestep-5Hz-lm-4B"`.
  - `test_caption_job_empty_prompt_counts_failed`: 2 tracks, second returns `prompt: ""` ⇒ `done == 1`, `failed` lists track 2, job not raised.
  - `test_describe_lm_missing_fails`: `lm_status()` None ⇒ EngineError "LM chưa nạp trên máy render".
  - In `test_runner.py`: `test_registration_lists_acestep_lm_when_lm_loaded` (model present, `caption_tracks` and `describe_input` in job_types even with no script captioners) and `test_registration_without_lm_has_no_acestep_lm`.
- [ ] **Step 2:** run them — Expected FAIL (module missing).
- [ ] **Step 3:** implement; runner dispatches `model.key == "acestep-lm"` to the LM functions before the script-captioner lookup.
- [ ] **Step 4:** full worker suite — Expected: all pass.
- [ ] **Step 5:** `docs/WORKER_PROTOCOL.md` section "Captioner": built-in `acestep-lm`, when it is registered, result mapping. Commit `feat(worker): acestep-lm captioner via ACE-Step analysis`.

### Task 4: API — `params.thinking` validation, fixtures

**Files:**
- Modify: `apps/api/src/routes/jobs.rs` (after the params merge at ~line 424-433), preset create/update validation in whichever route writes `style_presets.params` (`routes/registry.rs` — grep `params`)
- Test: `apps/api/tests/api.rs`

**Interfaces:**
- Produces: `fn check_thinking(params: &Map<String, Value>, capabilities: &[String]) -> AppResult<()>` in `jobs.rs`.

- [ ] **Step 1: failing tests** (`cargo test --test api thinking`), replacing all 16 Turbo fixture keys with `acestep-v15-xl-sft`:
  - `generate_thinking_without_capability_is_422`: message "LM chưa bật trên máy render".
  - `generate_thinking_with_capability_queues_job_with_param`: job payload `params.thinking == true`.
  - `generate_thinking_non_bool_is_422`.
  - `generate_from_preset_with_thinking_after_lm_removed_is_422`: preset params `{"thinking": true}`, model capabilities without `thinking`.
  - `preset_params_thinking_non_bool_is_422`.
- [ ] **Step 2:** run — Expected FAIL.
- [ ] **Step 3:** implement `check_thinking` on the merged params (preset + request).
- [ ] **Step 4:** `cargo test` and `cargo clippy --all-targets` — Expected: all pass, no warnings.
- [ ] **Step 5:** `docs/API_CONTRACTS.md`: `params.thinking` in GenerateRequest and Preset. Commit `feat(api): params.thinking needs a model with the thinking capability`.

### Task 5: Web — Thinking switch

**Files:**
- Modify: `apps/web/src/api/types.ts` (~line 454: `thinking?: boolean | null`), `src/features/create/formState.ts` (state `thinking: boolean`, preset/request load, request build), `src/features/create/GenerateForm.tsx` (Nâng cao, near steps/guidance ~line 395), `src/pages/admin/StylesPage.tsx`
- Test: `src/features/create/GenerateForm.test.tsx`, `src/features/create/formState.test.ts` (or the existing formState tests)

- [ ] **Step 1: failing tests:**
  - `shows Thinking switch only when the model has the thinking capability`.
  - `sends params.thinking true when switched on; omits/false when off`.
  - `loads thinking from a preset`.
- [ ] **Step 2:** `npx vitest run src/features/create` — Expected FAIL.
- [ ] **Step 3:** implement. Label "Thinking (LM)", hint "LM lên dàn ý trước khi tạo nhạc; chậm hơn." Counts as an advanced field (the `GenerateForm.tsx:41` open-if-set check).
- [ ] **Step 4:** `npm test` and `npm run build` — Expected pass.
- [ ] **Step 5:** commit `feat(web): Thinking (LM) switch`.

### Task 6: lush-training defaults

**Files:** `lush-training/src/lush_training/cli.py:602`, `src/lush_training/render_ab.py:25`, `lush-training/src/lush_training/acestep_backend.py:324` (docstring), tests `tests/test_render_ab.py`, `tests/test_model.py`, docs `lush-training/docs/MODEL_COMPATIBILITY.md`, `STATUS.md`.

- [ ] **Step 1:** change test fixtures/assertions to `acestep-v15-xl-sft` and 50 steps; run `pytest -q` in `lush-training` — Expected FAIL.
- [ ] **Step 2:** default `--model` `acestep-v15-xl-sft`; replace `TURBO_STEPS = 8` with `DEFAULT_STEPS = 50`. Keep the `is_turbo` warning in `model.py`.
- [ ] **Step 3:** `pytest -q` — Expected pass. Commit `chore(lush-training): XL SFT defaults`.

### Task 7: Docs and decision

- [ ] Update `docs/MUSIC_GENERATION.md`, `RENDER_NODE_DEPLOYMENT.md` (checkpoints, env vars incl. `ACESTEP_INIT_LLM`, `ACESTEP_LM_MODEL_PATH`), `STYLE_TRAINING.md`, `PROJECT_BRIEF.md`, `ROADMAP.md`, `docs/superpowers/mockups/create-lora-defaults.html:98`. Leave history in `TESTING.md`, `CHANGELOG.md`, old specs/plans.
- [ ] Add **D36** to `docs/DECISIONS.md` (decision, reason, cost: ~20 GB + 8.4 GB, slower 50-step generation, history wiped).
- [ ] `CHANGELOG.md` line; `docs/tasks/CURRENT.md` next step = Task 8.
- [ ] Verify: `grep -rin turbo apps workers lush-training/src docs --include=*.{rs,py,ts,tsx,md}` lists only history/is_turbo lines. Commit `docs: ACE-Step XL SFT only (D36)`.

### Task 8: Wipe history, remove Turbo files, deploy, end-to-end (GATED)

Each step needs the user's go-ahead. Order matters: the worker must already run XL (Task 1 Step 3) so Turbo is not re-registered.

- [ ] **Step 1:** `ssh lushvietnix2`: confirm the worker no longer reports Turbo (`model_installations` for the Turbo row gone or worker models list); `sqlite3 /var/lib/lush/lush.db ".backup /var/lib/lush/lush.db.bak-2026-11-25"`.
- [ ] **Step 2 (gated: deletion):** write `scripts/ops/2026-11-25-wipe-turbo-history.sql` (one transaction, FK-safe order; read the schema for every table that references jobs/tracks/artifacts first): delete all `tracks`, `artifacts`, `jobs`, `audit_logs`, the `acestep-v15-turbo` row in `model_installations` and `models`. Keep users, sessions, genres, nodes, captioner/scorer models. Dry-run on a copy of the backup; check counts (0 jobs/tracks/artifacts/audit_logs; 1 user; 23 genres). Then run on prod with `lush-api` stopped, start it. Delete the artifacts' files under the API storage dir (find the path from `.env`/config, delete only files whose IDs were in the deleted rows).
- [ ] **Step 3 (gated: deletion):** on rtx6000 remove `checkpoints/acestep-v15-turbo` and `checkpoints/acestep-5Hz-lm-1.7B`.
- [ ] **Step 4 (gated: push + deploy):** run the `lush-ai-music-studio:deploy` skill.
- [ ] **Step 5:** end-to-end on the website, real (spec "Kiểm thử từ đầu đến cuối" 1–7). Record results in `docs/TESTING.md` and `docs/tasks/COMPLETED.md`; delete the DB backup after 7 days (add to `CURRENT.md`).
