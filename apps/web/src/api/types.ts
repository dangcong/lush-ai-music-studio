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
  default_duration: number | null
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
  duration: number | null
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

export interface Model {
  id: string
  engine: string
  key: string
  name: string
  version: string | null
  description: string | null
  capabilities: string[]
  status: ModelStatus
  installed_nodes: InstalledNode[]
}

export interface ModelUpdateInput {
  name?: string
  description?: string
  status?: ModelStatus
}

export interface Lora {
  id: string
  key: string
  name: string
  version: string | null
  genre_id: string | null
  base_model_id: string
  status: string
  installed_nodes: InstalledNode[]
}

export interface GenerationParams {
  inference_steps: number | null
  guidance_scale: number | null
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
