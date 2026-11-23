# Dataset Factory + Train Style Lo-fi (pilot) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn a small batch of the user's own Lo-fi MP3s into an approved, immutable dataset version that Studio's existing Train Style (`train_lora`) can train on rtx6000. Then train, A/B-listen and let the user pick a checkpoint.

**Architecture:** Extend the existing dataset/caption pipeline in `lush-training/` (scan → build → caption → lock). Add five pieces:
- per-value confidence for BPM/key
- a captioner adapter
- a local review page
- a carry-over of captions across versions
- an exporter to Studio's `data/datasets/<name>/<version>/{dataset.json,audio/}` layout

Training stays in the Studio worker, which is unchanged. A/B renders talk to the local `acestep-api` over HTTP.

**Tech Stack:** Python 3.12, existing `lush_training` package, librosa + pyloudnorm, Pydantic v2, stdlib `http.server`, transformers (captioner, gated), FFmpeg/SoundFile, ACE-Step 1.5 `ca1e85f` HTTP API.

**Spec:** `docs/superpowers/specs/2026-11-23-lofi-style-lora-design.md`. Decision log: `docs/DECISIONS.md` D30.

## Global Constraints

- **Instrumental only.** A track a human confirms has vocal chops, speech or humming is excluded as a whole track (U6).
- **Whole songs (U7).** Use `segment.seconds: 240`, `segment.min_seconds: 30`. The trainer reads only the first 240 s (`workers/music-worker/src/lush_worker/training.py:205`).
- **No code changes in `workers/`, `apps/` or `ACE-Step-1.5/`.** `lush-training/` must not import from `apps/` or `workers/` (D28).
- **Immutable outputs.** Original MP3s are read-only. An exported dataset version is never overwritten. Fix something → new version.
- **No guessing.** BPM/key below the confidence threshold → `null`, and the field is left out of `dataset.json`. Only a human sets caption `status: approved` (`captions.set_fields(..., human=True)`).
- **Captioner `ACE-Step/acestep-captioner`:** no Music Flamingo, no Essentia or madmom. Download only after the user approves size and license.
- **Approval required, each time:**
  - installing on rtx6000
  - downloading the model
  - any GPU job (captioning, training, A/B render)
  - restarting `lush-worker` or stopping `acestep-api`
  - `git push`
- **Training window.** Train and A/B render stop music generation on the only node. Run them only in a window the user picked.
- **Studio dataset key** must match `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}(/[A-Za-z0-9][A-Za-z0-9._-]{0,63})?$` (`training.py:31`).
- **Paths on rtx6000:**
  - code: `/media/ai-rtx6000/AI_DATA/AiStudio/factory`
  - factory data (`TRAINING_DATA_ROOT`): `/media/ai-rtx6000/AI_DATA/AiStudio/data/factory`
  - originals: `…/data/import/lofi/<batch>/`
  - Studio datasets: `…/data/datasets/`
  - eval: `…/data/eval/`
- **Verify the mount before writing.** `findmnt /media/ai-rtx6000/AI_DATA` must show `/dev/nvme1n1p1 ext4` before any write on rtx6000.

## Review Focus

1. **MP3 quirks** (VBR, ID3 cover art, 44.1 kHz, Vietnamese or space-containing filenames). Scan and export must work, and output names must be IDs, not source names. Test: `test_scan_unicode_mp3_name` in Task 1.
2. **A track longer than 240 s** becomes two samples. The user must be told, not surprised. Test: `test_export_reports_split_tracks` in Task 1.
3. **The captioner writes "vocal-like", "voice", "choir pad"** for an instrumental track. The draft is flagged `needs_listening` and never silently approved. Test: `test_draft_with_voice_words_needs_listening` in Task 3.
4. **The reviewer closes the browser mid-review, or two tabs post the same item.** Each save is atomic per item, and the last write wins with both edits in `human_edits`. Test: `test_review_post_is_atomic_and_logged` in Task 4.
5. **A rebuild after vocal exclusions changes nothing for the other tracks.** Their approved captions carry over unchanged, and nothing needs re-review. Test: `test_carry_over_keeps_approved` in Task 4.

---

### Task 1: Whole-song config + Studio exporter

**Files:**
- Create: `lush-training/src/lush_training/studio_export.py`
- Create: `lush-training/configs/factory.lofi.example.yaml` (dataset config: `segment.seconds: 240`, `min_seconds: 30`, `split.validation: 0.2`, `split.test: 0`, `rights` block with comments)
- Modify: `lush-training/src/lush_training/cli.py` (add `studio-export` subcommand), `lush-training/scripts/` (add `export_studio_dataset.py`, the 14-line pattern of the others)
- Test: `lush-training/tests/test_studio_export.py`

**Interfaces:**
- Consumes: `dataset.build`, `dataset.load_manifest`, `captions.load_lock`, `captions.engine_sample`
- Produces: `export_studio(vdir: Path, caption_version: str, studio_root: Path, key: str) -> dict`. Returns `{"key", "dir", "train_samples", "split_tracks": [track_id…], "excluded_validation": [segment_id…]}`.

- [ ] **Step 1: Write failing tests**
  - `test_export_layout`: build a 2-group dataset from `conftest.song`, init + approve captions with `human=True`, lock `c1`, then `export_studio(vdir, "c1", tmp/"studio", "lofi-pilot/v1")`. Assert:
    - `tmp/studio/lofi-pilot/v1/dataset.json` has `samples` equal to the train segments only, each with `lyrics == "[Instrumental]"` and `audio_path` existing under `audio/`
    - `manifest.json` lists source sha256, `caption_version`, and the validation segment IDs
  - `test_export_refuses_existing`: a second export to the same key raises `LushTrainingError`
  - `test_export_rejects_bad_key`: key `"../x"` raises
  - `test_export_reports_split_tracks`: a 300 s song with `seconds=240` gives `split_tracks == [track_id]` and 2 samples
  - `test_scan_unicode_mp3_name`: an MP3 named `"Lofi chill – đêm 01.mp3"` scans, builds, and exports a file named `<segment_id>.wav`
- [ ] **Step 2: Run** `.venv\Scripts\python -m pytest tests/test_studio_export.py -q`. Expected: FAIL (module missing).
- [ ] **Step 3: Implement `export_studio`.**
  - Validate the key with the regex in Global Constraints. Call `load_lock` (this verifies hashes).
  - Write into `<key>.partial`: copy train WAVs, write `dataset.json` from the lock's `engine.train.json` (`metadata.name = key.replace('/', '-')`), and write `manifest.json`.
  - `os.replace` to final, then `chmod -R a-w` (no-op on Windows).
  - `split_tracks` = tracks with more than one segment.
- [ ] **Step 4: Run** the same command. Expected: PASS. Then run the full suite: `.venv\Scripts\python -m pytest -q`. Expected: all pass.
- [ ] **Step 5: Commit** `feat(lush-training): Studio Train Style dataset exporter`

### Task 2: BPM/key with confidence

**Files:**
- Create: `lush-training/src/lush_training/analysis.py`
- Modify: `lush-training/src/lush_training/captions.py:64-96` (`analyze_segment` uses `analysis`; `init_work` adds `key` evidence next to `tempo_bpm`), `lush-training/pyproject.toml` (keep `analysis` extra; add `pydantic>=2` to base deps)
- Test: `lush-training/tests/test_analysis.py` (skip the module if librosa is missing)

**Interfaces:**
- Produces (Pydantic models):
  - `TempoEstimate(bpm: float|None, confidence: float, alternatives: list[float], method: str)`
  - `KeyEstimate(key: str|None, confidence: float, method: str)`. `key` is formatted like ACE-Step's `keyscale`, e.g. `"D minor"`.
  - `estimate_tempo(mono: np.ndarray, rate: int, min_conf: float) -> TempoEstimate`
  - `estimate_key(mono: np.ndarray, rate: int, min_conf: float) -> KeyEstimate`
- `analysis/<segment_id>.json` gains `estimates.tempo` and `estimates.key` (the model dumps). `estimates.tempo_bpm` is kept for compatibility.

- [ ] **Step 1: Write failing tests**
  - `test_tempo_click_track`: 90 BPM clicks, 30 s → `bpm` within ±1.5 of 90, `alternatives` contains ≈45 and ≈180
  - `test_tempo_noise_unknown`: white noise → `bpm is None`
  - `test_key_c_major_progression`: synthetic I–IV–V–I triads in C → `"C major"`
  - `test_key_noise_unknown`: white noise → `key is None`
  - `test_init_work_records_key_evidence`: `evidence` contains a `field == "key"` entry with `source == "tool"`, and `metadata["key"] is None` (evidence only, never auto-filled)
- [ ] **Step 2: Run** `pytest tests/test_analysis.py -q`. Expected: FAIL.
- [ ] **Step 3: Implement.**
  - Tempo: `librosa.beat.beat_track` plus a beat-strength confidence (mean onset strength at beats / mean overall, mapped to 0–1).
  - Key: Krumhansl–Schmuckler correlation on `librosa.feature.chroma_cqt` mean. Confidence = best r minus second-best r.
  - Pick starting thresholds so the tests pass, and store them in the YAML (`analysis.min_tempo_conf`, `analysis.min_key_conf`). Task 8 recalibrates them on real audio.
- [ ] **Step 4: Run** the suite. Expected: PASS.
- [ ] **Step 5: Commit** `feat(lush-training): tempo/key estimates with confidence`

### Task 3: Captioner adapter (drafts only)

**Files:**
- Create: `lush-training/src/lush_training/captioner.py`
- Create: `lush-training/scripts/draft_captions.py`
- Modify: `lush-training/pyproject.toml` (extra `captioner = ["transformers", "accelerate"]`; torch comes from the cu128 index at install, Task 6)
- Test: `lush-training/tests/test_captioner.py`

**Interfaces:**
- `class Captioner(Protocol)`: `name: str`, `revision: str`, `describe(audio: Path) -> str`
- `class FolderLabelCaptioner`: built from a template string in YAML (`captioner.folder_template`); `describe` returns the template
- `class AceStepCaptioner`: loads `models/captioner/acestep-captioner` with transformers, exactly as its model card says. Not unit-tested; smoke-tested in Task 7.
- `CaptionDraft` (Pydantic): `caption_en: str` (≤150 words), `voice_terms: list[str]`, `source: str` (`"<name>@<revision>"`)
- `draft_all(vdir: Path, captioner: Captioner, *, by: str) -> dict`: returns `{"drafted", "skipped_approved", "needs_listening"}`. Calls `captions.set_fields(vdir, sid, by=by, human=False, caption_text=…, status=…)` and appends an `evidence` entry with `source: "tool"` and the captioner name/revision.

- [ ] **Step 1: Write failing tests.** All use a `FakeCaptioner` returning fixed text.
  - `test_draft_sets_text_not_approved`: status stays `draft`, `author.kind == "assistant"`
  - `test_draft_with_voice_words_needs_listening`: text `"warm keys with a soft vocal-like pad"` → status `needs_listening`, `voice_terms` non-empty. Reuse `captions.VOCAL_TERMS` and add `vocal-like`.
  - `test_draft_skips_approved`: a segment already approved by a human is untouched
  - `test_draft_truncates_over_150_words`: the long text is rejected into `needs_listening` with a reason, not silently cut
- [ ] **Step 2: Run** `pytest tests/test_captioner.py -q`. Expected: FAIL.
- [ ] **Step 3: Implement** the protocol, `FolderLabelCaptioner`, `CaptionDraft` and `draft_all`. In `AceStepCaptioner`, load lazily inside `describe`, and keep the import of transformers inside the class.
- [ ] **Step 4: Run** the suite. Expected: PASS.
- [ ] **Step 5: Commit** `feat(lush-training): captioner adapter producing reviewable drafts`

### Task 4: Review page + vocal exclusion + caption carry-over

**Files:**
- Create: `lush-training/src/lush_training/review.py` (logic), `lush-training/src/lush_training/review_static/index.html` (one page, vanilla JS), `lush-training/scripts/review_server.py`
- Modify: `lush-training/src/lush_training/captions.py` (add `carry_over`)
- Test: `lush-training/tests/test_review.py`

**Interfaces:**
- `ReviewDecision` (Pydantic):
  - `segment_id: str`
  - `verdict: Literal["correct","edited","wrong","exclude"]`
  - `caption_text: str|None`
  - `metadata: dict` (subset of `captions.METADATA_FIELDS`)
  - `hallucinations: list[Literal["instrument","mood","genre","tempo","key","other"]]`
  - `has_voice: bool`
  - `seconds_spent: float`
  - `reviewer: str`
- `apply_decision(project: Project, dataset: str, vdir: Path, d: ReviewDecision) -> dict`:
  - `correct` / `edited` → `set_fields(human=True, status="approved")`
  - `wrong` → `status="draft"` with the edit kept
  - `exclude` or `has_voice` → `dataset.record_review(project, dataset, track_sha, "vocal" if has_voice else …, by=reviewer)`, and the caption becomes `needs_listening`
  - Every call appends one line to `<vdir>/review/log.jsonl`.
- `serve(project, dataset, vdir, host="127.0.0.1", port=8765)`. Routes:
  - `GET /` (page)
  - `GET /api/items` (segment, analysis estimates, draft, status)
  - `GET /audio/<segment_id>` (WAV from the version; ID validated against the manifest)
  - `POST /api/items/<segment_id>`
  - Bind to localhost only.
- `carry_over(from_vdir: Path, to_vdir: Path) -> dict`: copies `captions/work/<sid>.json` for every segment ID present in both versions with an identical `segment_sha256`, rewriting `dataset.manifest_sha256`. Returns `{"carried", "new"}`.
- Non-voice exclusion uses a new `record_review` decision `"excluded"`. Extend `VOCAL_DECISIONS` and have `build` treat it like `"vocal"`, with the reason "người duyệt loại".

- [ ] **Step 1: Write failing tests**
  - `test_apply_correct_approves`
  - `test_apply_voice_excludes_track_on_next_build`: after `has_voice=True`, `build(..., "v2")` lists the track in `excluded`
  - `test_review_post_is_atomic_and_logged`: two decisions for one item → final state is the last, `human_edits` has both, `log.jsonl` has 2 lines
  - `test_audio_route_rejects_unknown_id`: `../manifest` → 404
  - `test_carry_over_keeps_approved`: approve all in v1, exclude one track, build v2, `carry_over` → the remaining segments are approved in v2 and `validate_work(v2)["ready_to_lock"]` is true
- [ ] **Step 2: Run** `pytest tests/test_review.py -q`. Expected: FAIL.
- [ ] **Step 3: Implement.** Use `http.server.ThreadingHTTPServer`, JSON in and out, and Pydantic validation (422 on bad input). The page shows: player, BPM/key estimate with confidence and alternatives, draft caption (editable), metadata fields, the four verdict buttons, hallucination checkboxes, a voice checkbox, and a timer from item open to save.
- [ ] **Step 4: Run** the suite. Expected: PASS. Then start the server locally on the test dataset and click through one item in the browser.
- [ ] **Step 5: Commit** `feat(lush-training): caption review page, vocal exclusion, carry-over`

### Task 5: Pilot report

**Files:**
- Create: `lush-training/src/lush_training/pilot_report.py`, `lush-training/scripts/pilot_report.py`
- Test: `lush-training/tests/test_pilot_report.py`

**Interfaces:**
- `pilot_report(vdir: Path, scan: dict) -> Path`: writes `<vdir>/review/pilot-report.md` with:
  - counts per verdict and % correct without edit
  - hallucinations per type
  - median and mean `seconds_spent`
  - tracks excluded for voice / other
  - BPM/key: share where the reviewer kept, changed or cleared the tool value
  - tracks split (>240 s)
  - near-duplicates found

- [ ] **Step 1: Write the failing test** `test_report_counts` from a hand-made `log.jsonl` (3 correct, 1 edited with `["instrument"]`, 1 voice). Assert the lines `Đúng: 3/5`, `Bịa nhạc cụ: 1`, `Loại vì giọng: 1`.
- [ ] **Step 2: Run** it. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the suite. Expected: PASS.
- [ ] **Step 5: Commit** `feat(lush-training): pilot caption report`

### Task 6: Install factory on rtx6000 + upload pilot batch (ops, needs approval)

**Files:**
- Create: `lush-training/scripts/install_rtx6000.sh` (idempotent): rsync target, `uv venv` Python 3.12, `pip install -e ".[analysis,captioner]"` with torch from `https://download.pytorch.org/whl/cu128`, and write `configs/project.yaml` with `data_root` = factory data path
- Modify: `docs/RENDER_NODE_DEPLOYMENT.md` (a section "Dataset Factory")

- [ ] **Step 1:** Ask the user for approval to install, and for the folder of 10–20 pilot MP3s on the dev PC.
- [ ] **Step 2:** `ssh rtx6000 findmnt -no SOURCE,FSTYPE /media/ai-rtx6000/AI_DATA`. Expected: `/dev/nvme1n1p1 ext4`.
- [ ] **Step 3:** Rsync `lush-training/` (excluding `.venv`, `configs/project.yaml`) to `AiStudio/factory/`, then run the install script. Verify: `factory/.venv/bin/python scripts/doctor.py` shows the GPU, and `pytest -q` passes on rtx6000.
- [ ] **Step 4:** `scp` the MP3s to `data/import/lofi/<batch-YYYYMMDD>/`, then `chmod -R a-w`. Verify: `sha256sum` on rtx6000 equals `Get-FileHash` on the dev PC for every file.
- [ ] **Step 5:** Write `configs/dataset.lofi.yaml` on rtx6000 from the example, pointing `source_dirs` at the batch. Set `rights.confirmed_by` to the user's name, as the user states it (ask).
- [ ] **Step 6: Commit** the script and docs.

### Task 7: Captioner check, download and smoke test (needs approval)

- [ ] **Step 1:** Read the official `ACE-Step/acestep-captioner` model card and file list (sizes, license, base model, transformers version, inference snippet). Report to the user and ask approval to download into `AiStudio/models/captioner/acestep-captioner` (pinned revision hash).
- [ ] **Step 2:** If declined, use `FolderLabelCaptioner` plus manual captions and skip to Task 8. Record the decision in D30.
- [ ] **Step 3:** If approved, download, then run `draft_captions.py` on 2 segments while `acestep-api` keeps running. Measure: VRAM peak (`nvidia-smi` every 1 s), seconds per track. Expected: text output, and website generation still works (`/health` 200).
- [ ] **Step 4:** Fill in the real loading code in `AceStepCaptioner` if it differed from Task 3. Run pytest. Commit.

### Task 8: Pilot dataset run (user in the loop)

- [ ] **Step 1:** On rtx6000: `prepare_dataset.py scan`, then show the scan summary (excluded files, near-duplicates, warnings) to the user.
- [ ] **Step 2:** `prepare_dataset.py build --version v1`, then `validate_captions.py init` (analysis), then `draft_captions.py`.
- [ ] **Step 3:** Start `review_server.py` and give the user `ssh -L 8765:127.0.0.1:8765 rtx6000` and the URL. The user reviews every item.
- [ ] **Step 4:** If any track was excluded: build `v2`, run `carry_over(v1, v2)`, and review only the new or changed items. Recalibrate the BPM/key thresholds from the review data and record the new values in YAML.
- [ ] **Step 5:** `validate_captions.py lock --caption-version c1`, `pilot_report.py`, then `export_studio_dataset.py --key lofi-pilot/v1` to `AiStudio/data/datasets/`. Verify: `dataset.json` sample count equals the train segments, and the directory is read-only.
- [ ] **Step 6:** Show the pilot report to the user.

### Task 9: T1 smoke + T2 train through Studio (in the user's window)

- [ ] **Step 1:** Ask the user for a window. Confirm no job is running (`/api/v1/jobs` queue empty). Restart `lush-worker`. Verify: the Training page lists `lofi-pilot/v1`.
- [ ] **Step 2: T1.** Create a run on the website: rank 32, alpha 64, lr 1e-4, 1 epoch, save_every 1. Record from the run page and the worker log: preprocess seconds, train seconds, VRAM peak, s/step, adapter size. After it finishes, generate one normal track to confirm inference is back. Archive the T1 LoRA.
- [ ] **Step 3: T2.** Same dataset and settings with 5 epochs (adjust only if T1 measurements demand it, and tell the user why). Verify: the log shows the preprocess step was skipped (tensor cache reuse), and 5 checkpoints were saved. The LoRA stays in `evaluation`.
- [ ] **Step 4: Cache-rebuild check (acceptance 4).** Later, edit one caption → lock `c2` → export `lofi-pilot/v2` → run a 1-epoch run. Verify: the log shows preprocess ran again. Archive that LoRA.
- [ ] **Step 5:** Write the measurements into `docs/STYLE_TRAINING.md` (a new section "Lo-fi pilot trên rtx6000") and `lush-training/docs/STATUS.md`.

### Task 10: A/B render + blind sheet (code, then run in a window)

**Files:**
- Create: `lush-training/src/lush_training/render_ab.py`, `lush-training/scripts/render_ab.py`, `lush-training/configs/eval_prompts.lofi.yaml` (prompts written from the **validation** tracks' approved captions plus 2 out-of-style prompts; 3 seeds each; 90 s)
- Test: `lush-training/tests/test_render_ab.py`

**Interfaces:**
- Consumes: `evaluate.make_plan`, `evaluate.make_ab`, `evaluate.memorization_screen`, `evaluate.write_report`
- `class AceStepHttp` (own small client; do not import from `workers/`). Uses `POST /release_task`, `POST /query_result`, `/v1/lora/{load,toggle,status}` and a download of the result file. `/v1/lora/unload` is never used. The behaviour mirrors `workers/music-worker/src/lush_worker/engines/acestep.py:105-160`.
- `render_side(client, plan: dict, out_dir: Path, *, side: str, lora_dir: Path|None) -> list[Path]`. Fixed `seed`, `inference_steps` and `lyrics="[Instrumental]"` from the plan. `lora_dir=None` → `toggle(False)`.

- [ ] **Step 1: Write failing tests** against a fake HTTP server (stdlib):
  - `test_render_base_toggles_off`
  - `test_render_lora_loads_once_per_checkpoint`
  - `test_same_seed_and_steps_both_sides`
  - `test_failed_task_raises`
- [ ] **Step 2: Run.** Expected: FAIL. **Step 3: Implement.** **Step 4: Run the suite.** Expected: PASS. **Step 5: Commit** `feat(lush-training): A/B render against acestep-api`.
- [ ] **Step 6 (window, approval):** `systemctl --user stop lush-worker`. Render base plus every T2 checkpoint into `AiStudio/data/eval/lofi-pilot-v1/<run>/`. Then run `make_ab`, `memorization_screen` against the train segments, and `write_report`. Then `systemctl --user restart acestep-api` and wait for `/health`, start `lush-worker`, and generate one normal track to check.
- [ ] **Step 7:** Give the user the blind sheet. The user listens and picks the checkpoint. Record the choice. The LoRA stays in `evaluation`; no Style is created.

### Task 11: Handover docs

- [ ] **Step 1:** Update the following:
  - `docs/tasks/CURRENT.md` (T-014 status)
  - `docs/CHANGELOG.md`
  - `lush-training/docs/STATUS.md` (each new item's verification level)
  - `lush-training/AGENTS.md` (a short paragraph: factory = data prep for Studio Train Style, not the decoder trainer)
  - D30, if anything changed
- [ ] **Step 2:** Write the handover for Codex review into `CURRENT.md` T-014: commits, commands run on rtx6000, test results, measurements, and what is still unverified.
- [ ] **Step 3:** Commit. Do not push without approval.
