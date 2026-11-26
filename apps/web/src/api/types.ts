// Types mirror docs/API_CONTRACTS.md. Keep field names identical to the contract.

export type Role = 'admin' | 'trainer' | 'member'

export const ROLES: Role[] = ['admin', 'trainer', 'member']

export interface User {
  id: string
  username: string
  display_name: string | null
  role: Role
  is_active: boolean
  created_at: string
}

export interface UserCreateInput {
  username: string
  password: string
  display_name?: string
  role: Role
}

export interface UserUpdateInput {
  display_name?: string
  role?: Role
  is_active?: boolean
  password?: string
}

export interface Genre {
  id: string
  parent_id: string | null
  name: string
  slug: string
  description: string | null
  prompt_template: string | null
  instruments: string[]
  moods: string[]
  bpm_min: number | null
  bpm_max: number | null
  /** Suggested duration range in seconds; both null = none (a subgenre then follows its genre). */
  duration_min: number | null
  duration_max: number | null
  is_active: boolean
  sort_order: number
}

export type GenreInput = Partial<Omit<Genre, 'id'>>

export type PresetKind = 'genre' | 'trained'

export interface Preset {
  id: string
  genre_id: string | null
  name: string
  description: string | null
  kind: PresetKind
  prompt_template: string | null
  instruments: string[]
  moods: string[]
  bpm: number | null
  /** Suggested duration range in seconds; both null = follow the subgenre / genre. */
  duration_min: number | null
  duration_max: number | null
  model_id: string | null
  lora_id: string | null
  lora_scale: number | null
  params: Record<string, unknown>
  version: number
  is_active: boolean
}

export type PresetInput = Partial<Omit<Preset, 'id' | 'version'>>

export interface InstalledNode {
  node_id: string
  node_name: string
  online: boolean
}

export type ModelStatus = 'active' | 'disabled'
export type ModelKind = 'generator' | 'captioner' | 'scorer'

export interface Model {
  id: string
  engine: string
  key: string
  name: string
  version: string | null
  description: string | null
  capabilities: string[]
  status: ModelStatus
  kind: ModelKind
  installed_nodes: InstalledNode[]
}

export interface ModelUpdateInput {
  name?: string
  description?: string
  status?: ModelStatus
}

export type LoraStatus = 'training' | 'evaluation' | 'approved' | 'archived'

export interface Lora {
  id: string
  key: string
  name: string
  version: string | null
  genre_id: string | null
  base_model_id: string
  status: LoraStatus
  installed_nodes: InstalledNode[]
  created_at?: string | null
  /** Training run that produced this LoRA, when it was trained in the studio. */
  job_id?: string | null
  dataset_key?: string | null
  trainer?: string | null
  config?: Record<string, unknown> | null
  adapter_sha256?: string | null
  created_by?: string | null
}

export interface LoraUpdateInput {
  status?: Exclude<LoraStatus, 'training'>
  name?: string
}

/** What a render node reported about one dataset folder; every field may be missing. */
export interface DatasetInfo {
  key?: string
  name?: string
  samples?: number
  seconds?: number
}

export interface Dataset {
  key: string
  node_id: string
  node_name: string
  online: boolean
  can_train: boolean
  info: DatasetInfo | null
}

/** Body of POST /training-runs. */
export interface TrainingRunInput {
  name: string
  dataset_key: string
  base_model_id: string
  rank: number
  alpha: number
  learning_rate: number
  epochs: number
  gradient_accumulation: number
  save_every: number
  warmup_steps: number
  seed: number | null
  max_minutes: number
}

/** `job.result` of a finished training run; treat every field as optional. */
export interface TrainingResult {
  lora?: { key?: string; adapter_sha256?: string; bytes?: number } | null
  epochs_completed?: number | null
  optimizer_steps?: number | null
  stop_reason?: string | null
  checkpoints?: { epoch?: number; bytes?: number }[] | null
  preprocess_seconds?: number | null
  train_seconds?: number | null
  vram_peak_mb?: number | null
  inference_restored?: boolean | null
  adapter_loadable?: boolean | null
}

/** A job of type `train_lora`. */
export interface TrainingRun extends Omit<Job, 'request' | 'result' | 'log'> {
  request: Partial<TrainingRunInput> & { dataset?: { key?: string }; lora?: { key?: string; name?: string } }
  result: TrainingResult | null
}

/** One point reported by the trainer. Any field may be null. */
export interface TrainingMetric {
  sequence: number | null
  global_step: number | null
  epoch: number | null
  train_loss: number | null
  learning_rate: number | null
  elapsed_seconds: number | null
  gpu_memory_mb: number | null
  recorded_at: string | null
}

/** GET /training-runs/{id}. */
export interface TrainingRunDetail extends TrainingRun {
  log?: string | null
  /** Last time the worker reported anything for this run. */
  updated_at?: string | null
  metrics?: TrainingMetric[] | null
  lora: ReviewLora | null
  /** Newest eval_lora job of the LoRA; null before one exists. */
  eval: EvalInfo | null
  renders: EvalRender[]
  held_out_tracks: HeldOutTrack[]
  style_preset: StylePresetRef | null
  files_job: FilesJob | null
  /** Summary of the dataset snapshot the run trained from; null for runs on a node dataset. */
  snapshot: SnapshotSummary | null
  /** Kept epochs; the node path never leaves the server. */
  checkpoints: Checkpoint[]
}

/** `snapshot` on GET /training-runs/{id}. */
export interface SnapshotSummary {
  id: string
  catalog_id: string
  catalog_name: string
  /** Number of training samples. */
  samples: number
  /** Filenames of the held-out songs. */
  held_out: string[]
  keyword: string
  test_prompt: string
  models: ModelCount[]
}

export interface Checkpoint {
  id: string
  epoch: number
  step: number | null
  bytes: number | null
  status: string
  note: string
  expires_at: string | null
  delete_requested_at: string | null
  /** Reference scores only; null while the scorer is not installed. */
  style_score: number | null
  train_score: number | null
  train_match: string | null
  melody_score: number | null
  melody_match: string | null
  quality_score: number | null
}

/** `lora` on GET /training-runs/{id}. */
export interface ReviewLora {
  id: string
  key: string
  base_key: string
  name: string
  status: LoraStatus
  chosen_checkpoint_id: string | null
}

/** `eval` on GET /training-runs/{id}. `stage` is `rendering` or `scoring` while running. */
export interface EvalInfo {
  job_id: string
  status: string
  progress: number
  stage: string | null
  error: string | null
  scorers: { mert: boolean; audiobox: boolean }
}

export interface RenderScores {
  mert_style?: number | null
  mert_train?: number | null
  train_match?: string | null
  melody?: number | null
  melody_match?: string | null
  quality?: number | null
}

export interface EvalRender {
  id: string
  kind: 'base' | 'prompt' | 'held_out'
  checkpoint_id: string | null
  sample_filename: string | null
  prompt: string
  /** Play through /artifacts/{id}/content. */
  artifact_id: string
  scores: RenderScores
}

export interface HeldOutTrack {
  filename: string
  artifact_id: string
}

export interface StylePresetRef {
  id: string
  name: string
  lora_scale: number | null
  is_active: boolean
}

/** Newest `lora_files` job of the LoRA. */
export interface FilesJob {
  job_id: string
  status: string
  op: string
  error: string | null
}

export interface PublishInput {
  checkpoint_id: string
  style_name: string
  lora_scale: number
}

/** 202 with a job, or 200 with `job_id: null` when the epoch is already installed. */
export interface PublishResult {
  job_id: string | null
  style_preset_id: string
}

export interface CleanupResult {
  job_id: string | null
  count: number
}

export interface QuickTryInput {
  checkpoint_id: string
  prompt: string
  lora_scale: number
}

/** Body of POST /catalogs/{id}/lora-preview and POST /catalogs/{id}/loras. */
export interface LoraBuildRequest {
  name: string
  keyword: string
  test_prompt: string
  track_ids: string[]
  holdout_count: number
  holdout_seed: number
  voice_confirmed: string[]
  base_model_id: string
  rank: number
  alpha: number
  learning_rate: number
  epochs: number
  gradient_accumulation: number
  save_every: number
  warmup_steps: number
  seed: number | null
  max_minutes: number
}

export interface BuildTrack {
  id: string
  filename: string
  duration_s: number | null
  model_key: string | null
  vocals: string
  caption: string
  /** `model`: the machine's caption, never saved by a person. */
  caption_source: string | null
  /** Words of the genre's watch list found in the caption (D37). */
  suspect_words: string[]
}

export interface StyleOutlier {
  id: string
  filename: string
  /** Cosine of the song's MERT embedding to the group's mean. */
  score: number
}

export interface ModelCount {
  model_key: string
  name: string
  count: number
}

export interface BuildPlan {
  ready: boolean
  errors: string[]
  warnings: string[]
  train: BuildTrack[]
  held_out: BuildTrack[]
  missing_caption: BuildTrack[]
  voice_unconfirmed: BuildTrack[]
  no_tempo_key: BuildTrack[]
  /** Train and held-out songs with a machine caption nobody reviewed (warning only). */
  unreviewed_caption: BuildTrack[]
  /** Songs far from the group's style (MERT), warning only (D37). */
  style_outliers: StyleOutlier[]
  /** Songs analysed without a MERT embedding. */
  style_unmeasured: number
  train_seconds: number
  unknown_duration: number
  models: ModelCount[]
  steps_per_epoch: number
  total_steps: number
  checkpoint_epochs: number[]
  eval_renders: number
  name_taken: boolean
}

/** 201 body of POST /catalogs/{id}/loras. */
export interface CreateLoraResult {
  job_id: string
  snapshot_id: string
  lora_key: string
}

export interface SnapshotSample {
  catalog_track_id: string
  artifact_id: string
  filename: string
  sha256: string
  size_bytes: number
  duration_s: number | null
  model_key: string | null
  caption_seq: number | null
  caption_source: string | null
  mood: string | null
  instruments: string | null
  vocals: string
  caption: string
  train_caption: string
  bpm: number | null
  keyscale: string | null
  voice_confirmed: boolean
}

/** GET /dataset-snapshots/{id}; `content` is the immutable record of the selection (schema 1). */
export interface DatasetSnapshot {
  id: string
  catalog_id: string
  job_id: string
  lora_key: string
  created_at: string
  content: {
    schema: number
    catalog: { id: string; name: string }
    genre: { id: string; name: string }
    lora: { key: string; name: string }
    keyword: string
    test_prompt: string
    holdout_seed: number
    base_model: { id: string; key: string | null }
    config: {
      rank: number
      alpha: number
      learning_rate: number
      epochs: number
      gradient_accumulation: number
      save_every: number
      warmup_steps: number
      seed: number
      max_duration_s: number
    }
    limits: { max_seconds: number }
    samples: SnapshotSample[]
    held_out: SnapshotSample[]
    models: ModelCount[]
  }
}

/** The training run that currently occupies a render node. */
export interface QueueTraining {
  job_id: string
  title: string | null
  stage: string | null
  started_at: string | null
  node_name: string | null
}

export interface GenerationParams {
  inference_steps: number | null
  guidance_scale: number | null
  /** Cover mode only, 0..1: higher keeps closer to the Source Audio. */
  audio_cover_strength?: number | null
  /** The LM plans the song first (model capability `thinking`, D36). */
  thinking?: boolean | null
}

export type GenerationMode = 'text' | 'reference' | 'cover'

/** Model capability each mode needs (`text` needs none beyond `generate`). */
export const MODE_CAPABILITY: Record<GenerationMode, string | null> = {
  text: null,
  reference: 'reference_audio',
  cover: 'cover',
}

/** An uploaded audio file; its role (reference / source) is decided by the job's mode. */
export interface AudioInput {
  id: string
  filename: string
  content_type: string
  size_bytes: number
  sha256: string
  created_at: string
}

/** Entry of `track.params.inputs`. */
export interface TrackInput {
  artifact_id: string
  filename: string
  size_bytes: number
  sha256: string
}

export interface GenerationRequest {
  prompt: string
  title: string | null
  project_id: string | null
  genre_id: string | null
  subgenre_id: string | null
  preset_id: string | null
  model_id: string | null
  lora_id: string | null
  lora_scale: number | null
  duration: number
  bpm: number | null
  key: string | null
  time_signature: string | null
  seed: number | null
  instruments: string[]
  moods: string[]
  instrumental: boolean
  params: GenerationParams
  mode: GenerationMode
  reference_audio_id: string | null
  source_audio_id: string | null
  idempotency_key: string
}

export type JobStatus = 'queued' | 'preparing' | 'running' | 'completed' | 'failed' | 'cancelled'

export const JOB_STATUSES: JobStatus[] = [
  'queued',
  'preparing',
  'running',
  'completed',
  'failed',
  'cancelled',
]

export interface Job {
  id: string
  type: string
  status: JobStatus
  priority: number
  /** Fraction 0..1. */
  progress: number | null
  stage: string | null
  message: string | null
  error: string | null
  attempt: number
  max_attempts: number
  cancel_requested: boolean
  owner_id: string
  owner_name: string | null
  node_id: string | null
  node_name: string | null
  track_id: string | null
  title: string | null
  /** The body the user originally sent to POST /generations. */
  request: Partial<GenerationRequest>
  result: Record<string, unknown> | null
  created_at: string
  started_at: string | null
  finished_at: string | null
  /** Only present on GET /jobs/{id}. */
  log?: string | string[] | null
}

export interface JobListParams {
  status?: JobStatus
  type?: string
  mine?: boolean
  limit?: number
}

export interface QueueSummary {
  queued: number
  preparing: number
  running: number
  nodes_online: number
  /** Set while a render node is training a LoRA (it cannot generate music meanwhile). */
  training?: QueueTraining | null
}

export type ArtifactKind = 'audio_master' | 'audio_preview'

export interface Artifact {
  id: string
  kind: ArtifactKind
  filename: string
  content_type: string
  size_bytes: number
  sha256: string
}

export interface Track {
  id: string
  title: string
  owner_id: string
  owner_name: string | null
  project_id: string | null
  job_id: string | null
  parent_track_id: string | null
  version: number
  prompt: string
  prompt_final: string | null
  genre_id: string | null
  genre_name: string | null
  subgenre_id: string | null
  preset_id: string | null
  model_id: string | null
  model_name: string | null
  lora_id: string | null
  lora_scale: number | null
  seed: number | null
  bpm: number | null
  key: string | null
  time_signature: string | null
  duration: number | null
  favorite: boolean
  rating: number
  params: Record<string, unknown>
  artifacts: Artifact[]
  created_at: string
}

export interface TrackListParams {
  q?: string
  genre_id?: string
  project_id?: string
  favorite?: boolean
  mine?: boolean
  limit?: number
}

export interface TrackUpdateInput {
  title?: string
  favorite?: boolean
  rating?: number
  project_id?: string | null
}

export type NodeStatus = 'active' | 'disabled'

/** Free-form object reported by the worker; every field may be missing. */
export interface NodeGpu {
  name?: string
  utilization?: number
  vram_used_mb?: number
  vram_total_mb?: number
}

export interface NodeEngine {
  name?: string
  version?: string
  capabilities?: string[]
}

export interface RenderNode {
  id: string
  name: string
  status: NodeStatus
  maintenance: boolean
  online: boolean
  worker_version: string | null
  protocol_version: number | null
  job_types: string[]
  engines: NodeEngine[]
  gpu: NodeGpu | null
  models: { id: string; key: string; engine: string }[]
  loras: { id: string; key: string }[]
  current_job_id: string | null
  last_seen: string | null
  registered_at: string | null
  created_at: string
}

export interface NodeUpdateInput {
  name?: string
  status?: NodeStatus
  maintenance?: boolean
}

export interface NodeCreated {
  node: RenderNode
  token: string
}

export interface AuditLog {
  id: string
  actor_id: string | null
  actor_name: string | null
  action: string
  target_type: string | null
  target_id: string | null
  detail: Record<string, unknown> | null
  created_at: string
}

// ---- Catalogs (docs/API_CONTRACTS.md "Catalogs") ----

export interface Catalog {
  id: string
  name: string
  genre_id: string
  genre_name: string
  track_count: number
  total_seconds: number
  created_at: string
}

export interface CatalogCreateInput {
  name: string
  genre_id: string
}

export interface CatalogUpdateInput {
  name?: string
}

export type CatalogAnalysisStatus = 'pending' | 'done' | 'failed'
export type CatalogValueSource = 'auto' | 'human' | null

export interface CatalogTempoEstimate {
  /** null when the machine cannot pick between half/double time: see alternatives. */
  bpm: number | null
  confidence: number
  alternatives: number[]
  /** Half or double of `bpm` inside the genre's BPM range, stored as the track tempo (D37). */
  genre_bpm?: number | null
}

export interface CatalogKeyEstimate {
  /** null when the machine has no basis for a key. */
  key: string | null
  confidence: number
}

export interface CaptionView {
  model_key: string
  seq: number
  mood: string
  instruments: string
  vocals: string
  caption: string
  source: 'model' | 'human'
  source_ref: string | null
  created_at: string
  can_undo: boolean
  can_redo: boolean
}

export interface CaptionInput {
  mood: string
  instruments: string
  vocals: string
  caption: string
}

export interface CatalogTrack {
  id: string
  catalog_id: string
  artifact_id: string
  filename: string
  /** null until the analysis has run. */
  duration_s: number | null
  tempo_bpm: number | null
  tempo_source: CatalogValueSource
  key_name: string | null
  key_source: CatalogValueSource
  analysis_status: CatalogAnalysisStatus
  estimates: { tempo: CatalogTempoEstimate | null; key: CatalogKeyEstimate | null }
  current_model_key: string | null
  models_done: string[]
  caption: CaptionView | null
  captioning_model_key: string | null
  voice_warning: boolean
  created_at: string
}

export interface CatalogTrackUpdateInput {
  tempo_bpm?: number | null
  key_name?: string | null
  current_model_key?: string
}

export interface CaptionJobInput {
  model_key: string
  track_ids: string[]
  force?: boolean
}

export interface CaptionJobResult {
  job_id: string | null
  queued: string[]
  skipped: string[]
}
