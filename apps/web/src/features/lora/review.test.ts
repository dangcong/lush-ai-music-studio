import { describe, expect, it } from 'vitest'
import { makeTrainingRun } from '../../test/utils'
import type { Checkpoint, EvalInfo, EvalRender, TrainingRunDetail } from '../../api/types'
import { checkpointVerdict, MEMORIZE, rendersFor, reviewSteps } from './review'

function cp(over: Partial<Checkpoint> = {}): Checkpoint {
  return {
    id: 'c1', epoch: 2, step: 10, bytes: 1, status: 'kept', note: '', expires_at: null,
    delete_requested_at: null, style_score: null, train_score: null, train_match: null,
    melody_score: null, melody_match: null, quality_score: null, ...over,
  }
}
function ev(over: Partial<EvalInfo> = {}): EvalInfo {
  return { job_id: 'e1', status: 'running', progress: 0, stage: 'running', error: null,
    scorers: { mert: true, audiobox: true }, ...over }
}
const lora = { id: 'l1', key: 'k', base_key: 'k', name: 'L', status: 'evaluation' as const, chosen_checkpoint_id: null }
function done(over: Partial<TrainingRunDetail> = {}): TrainingRunDetail {
  return makeTrainingRun({ status: 'completed', progress: 1, lora,
    checkpoints: [cp({ id: 'a', epoch: 2 }), cp({ id: 'b', epoch: 4 })],
    held_out_tracks: [{ filename: 'h.mp3', artifact_id: 'x' }], ...over })
}

describe('reviewSteps', () => {
  it('has the four titled steps', () => {
    expect(reviewSteps(makeTrainingRun()).map((s) => s.title)).toEqual([
      'Chuẩn bị dữ liệu', 'Train', 'Tạo nhạc nghe thử', 'Bạn nghe và chọn bản'])
  })
  it('shows i/N while the eval renders', () => {
    // 2 checkpoints x (1 + 1 held out) + 1 = 5 renders; progress 0.54 of 0.9 render share -> 3 done
    const steps = reviewSteps(done({ eval: ev({ stage: 'rendering', progress: 0.54 }) }))
    expect(steps[1]!.state).toBe('done')
    expect(steps[2]).toMatchObject({ state: 'active', detail: '3/5 bài' })
    expect(steps[3]!.state).toBe('waiting')
  })
  it('shows scoring', () => {
    expect(reviewSteps(done({ eval: ev({ stage: 'scoring', progress: 0.95 }) }))[2]!.detail).toBe('Đang chấm điểm')
  })
  it('fails step 3 when eval failed', () => {
    const steps = reviewSteps(done({ eval: ev({ status: 'failed', error: 'hết bộ nhớ' }) }))
    expect(steps[2]).toMatchObject({ state: 'failed' })
    expect(steps[2]!.detail).toContain('hết bộ nhớ')
  })
  it('waits for the user after eval completes', () => {
    const steps = reviewSteps(done({ eval: ev({ status: 'completed', progress: 1 }) }))
    expect(steps[2]!.state).toBe('done')
    expect(steps[3]).toMatchObject({ state: 'active', detail: 'Chờ bạn' })
  })
  it('shows the update in progress while a publish job is live, even when an old epoch is chosen', () => {
    const steps = reviewSteps(done({
      eval: ev({ status: 'completed' }),
      lora: { ...lora, chosen_checkpoint_id: 'a' },
      style_preset: { id: 's', name: 'S', lora_scale: 0.7, is_active: true },
      files_job: { job_id: 'f', status: 'running', op: 'publish', error: null },
    }))
    expect(steps[3]).toMatchObject({ state: 'active', detail: 'Đang cập nhật trên máy…' })
  })
  it('shows a publish failure', () => {
    const steps = reviewSteps(done({
      eval: ev({ status: 'completed' }),
      files_job: { job_id: 'f', status: 'failed', op: 'publish', error: 'sha lệch' },
    }))
    expect(steps[3]).toMatchObject({ state: 'failed', detail: 'Đưa lên máy lỗi: sha lệch' })
  })
  it('is done once the chosen epoch is in an active Style', () => {
    const steps = reviewSteps(done({
      eval: ev({ status: 'completed' }),
      lora: { ...lora, chosen_checkpoint_id: 'b', status: 'approved' },
      style_preset: { id: 's', name: 'S', lora_scale: 0.7, is_active: true },
      files_job: { job_id: 'f', status: 'completed', op: 'publish', error: null },
    }))
    expect(steps[3]).toMatchObject({ state: 'done', detail: 'Đã đưa Epoch 4 vào Style Preset' })
  })
  it('marks train failed when the run failed', () => {
    const steps = reviewSteps(makeTrainingRun({ status: 'failed', error: 'oom' }))
    expect(steps[1]!.state).toBe('failed')
  })
})

describe('checkpointVerdict', () => {
  it('warns about memorizing at the boundary (mert)', () => {
    const c = cp({ train_score: MEMORIZE.mert, train_match: 'a.mp3' })
    expect(checkpointVerdict(c, [c])).toEqual({ text: 'Có dấu hiệu học thuộc: gần giống a.mp3', tone: 'warn' })
  })
  it('warns about memorizing by melody', () => {
    const c = cp({ style_score: 0.5, melody_score: 0.8, melody_match: 'b.mp3' })
    expect(checkpointVerdict(c, [c]).tone).toBe('warn')
    expect(checkpointVerdict(c, [c]).text).toContain('b.mp3')
  })
  it('does not warn just below the thresholds', () => {
    const c = cp({ style_score: 0.5, train_score: 0.949, melody_score: 0.799 })
    expect(checkpointVerdict(c, [c]).tone).not.toBe('warn')
  })
  it('names the closest style among scored checkpoints', () => {
    const a = cp({ id: 'a', style_score: 0.6 })
    const b = cp({ id: 'b', style_score: 0.8 })
    expect(checkpointVerdict(b, [a, b])).toEqual({ text: 'Giống style nhất', tone: 'good' })
    expect(checkpointVerdict(a, [a, b])).toEqual({ text: '', tone: 'muted' })
  })
  it('says the scorer is missing without a MERT score', () => {
    const c = cp()
    expect(checkpointVerdict(c, [c])).toEqual({ text: 'Chưa cài máy chấm', tone: 'muted' })
  })
})

describe('rendersFor', () => {
  const r = (id: string, kind: EvalRender['kind'], checkpoint_id: string | null): EvalRender => ({
    id, kind, checkpoint_id, sample_filename: null, prompt: 'p', artifact_id: 'a' + id, scores: {} })
  const run = makeTrainingRun({ renders: [r('1', 'base', null), r('2', 'held_out', 'a'), r('3', 'prompt', 'a'), r('4', 'prompt', 'b')] })
  it('filters by checkpoint, prompt before held out', () => {
    expect(rendersFor(run, 'a').map((x) => x.id)).toEqual(['3', '2'])
  })
  it('null gives the no-LoRA render', () => {
    expect(rendersFor(run, null).map((x) => x.id)).toEqual(['1'])
  })
})
