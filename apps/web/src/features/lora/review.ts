import type { Checkpoint, EvalRender, TrainingRunDetail } from '../../api/types'

export type StepState = 'done' | 'active' | 'waiting' | 'failed'
export interface ReviewStep {
  title: string
  state: StepState
  detail: string
}

/** Memorize thresholds, same values as the worker. NOT calibrated ("chưa hiệu chỉnh"). */
export const MEMORIZE = { mert: 0.95, melody: 0.8 }

/** Share of the eval job progress spent rendering (worker RENDER_SHARE); the rest is scoring. */
const RENDER_SHARE = 0.9
const MAX_EVALUATED = 10
const LIVE = ['queued', 'preparing', 'running']

function step(title: string, state: StepState, detail: string): ReviewStep {
  return { title, state, detail }
}

/** Number of listening renders the eval job makes: checkpoints x (1 + held out) + 1. */
function expectedRenders(run: TrainingRunDetail): number {
  const kept = run.checkpoints.filter((c) => c.status !== 'deleted').length
  return Math.min(kept, MAX_EVALUATED) * (1 + run.held_out_tracks.length) + 1
}

export function reviewSteps(run: TrainingRunDetail): ReviewStep[] {
  const T = ['Chuẩn bị dữ liệu', 'Train', 'Tạo nhạc nghe thử', 'Bạn nghe và chọn bản']
  const trained = run.status === 'completed'
  const trainFailed = run.status === 'failed' || run.status === 'cancelled'

  let s1: ReviewStep
  let s2: ReviewStep
  if (run.status === 'queued') {
    s1 = step(T[0]!, 'waiting', 'Chờ tới lượt')
    s2 = step(T[1]!, 'waiting', '')
  } else if (run.stage === 'preparing' || run.stage === 'preprocessing') {
    s1 = step(T[0]!, 'active', 'Đang chuẩn bị')
    s2 = step(T[1]!, 'waiting', '')
  } else if (trainFailed) {
    s1 = step(T[0]!, 'done', '')
    s2 = step(T[1]!, 'failed', run.status === 'cancelled' ? 'Đã hủy' : run.error ?? 'Train lỗi')
  } else if (trained) {
    s1 = step(T[0]!, 'done', '')
    s2 = step(T[1]!, 'done', 'Xong')
  } else {
    s1 = step(T[0]!, 'done', '')
    s2 = step(T[1]!, 'active', `${Math.round((run.progress ?? 0) * 100)}%`)
  }

  const ev = run.eval
  let s3: ReviewStep
  if (!trained) s3 = step(T[2]!, 'waiting', '')
  else if (!ev) s3 = step(T[2]!, 'waiting', 'Chờ tới lượt')
  else if (ev.status === 'completed') s3 = step(T[2]!, 'done', `${expectedRenders(run)} bài`)
  else if (ev.status === 'failed' || ev.status === 'cancelled') {
    s3 = step(T[2]!, 'failed', ev.status === 'cancelled' ? 'Đã hủy' : `Lỗi: ${ev.error ?? 'không rõ'}`)
  } else if (ev.status === 'running' && ev.stage === 'scoring') s3 = step(T[2]!, 'active', 'Đang chấm điểm')
  else if (ev.status === 'running') {
    const total = expectedRenders(run)
    const i = Math.min(total, Math.max(0, Math.floor((ev.progress / RENDER_SHARE) * total + 1e-9)))
    s3 = step(T[2]!, 'active', `${i}/${total} bài`)
  } else s3 = step(T[2]!, 'active', 'Chờ tới lượt')

  const files = run.files_job?.op === 'publish' ? run.files_job : null
  const chosenId = run.lora?.chosen_checkpoint_id ?? null
  const chosen = run.checkpoints.find((c) => c.id === chosenId)
  let s4: ReviewStep
  if (files && LIVE.includes(files.status)) s4 = step(T[3]!, 'active', 'Đang cập nhật trên máy…')
  else if (files && files.status === 'failed') {
    s4 = step(T[3]!, 'failed', `Đưa lên máy lỗi: ${files.error ?? 'không rõ'}`)
  } else if (chosen && run.style_preset?.is_active) {
    s4 = step(T[3]!, 'done', `Đã đưa Epoch ${chosen.epoch} vào Style Preset`)
  } else if (ev?.status === 'completed') s4 = step(T[3]!, 'active', 'Chờ bạn')
  else s4 = step(T[3]!, 'waiting', '')

  return [s1, s2, s3, s4]
}

export function checkpointVerdict(
  c: Checkpoint,
  all: Checkpoint[],
): { text: string; tone: 'warn' | 'good' | 'muted' } {
  if (c.train_score !== null && c.train_score >= MEMORIZE.mert) {
    return { text: `Có dấu hiệu học thuộc: gần giống ${c.train_match ?? 'một bài train'}`, tone: 'warn' }
  }
  if (c.melody_score !== null && c.melody_score >= MEMORIZE.melody) {
    return { text: `Có dấu hiệu học thuộc: gần giống ${c.melody_match ?? 'một bài train'}`, tone: 'warn' }
  }
  if (c.style_score === null) return { text: 'Chưa cài máy chấm', tone: 'muted' }
  const scored = all.filter((o) => o.status !== 'deleted' && o.style_score !== null)
  const best = Math.max(...scored.map((o) => o.style_score as number))
  // "Closest" is only meaningful when there is something to compare with.
  if (scored.length > 1 && c.style_score === best) return { text: 'Giống style nhất', tone: 'good' }
  return { text: '', tone: 'muted' }
}

/** Renders of one checkpoint (null = the no-LoRA reference), listening prompt first, then held out. */
export function rendersFor(run: TrainingRunDetail, checkpointId: string | null): EvalRender[] {
  const order = { base: 0, prompt: 1, held_out: 2 }
  return run.renders
    .filter((r) => r.checkpoint_id === checkpointId)
    .sort((a, b) => order[a.kind] - order[b.kind])
}
