import { describe, expect, it } from 'vitest'
import { makeJob } from '../../test/utils'
import {
  buildLoraRequest,
  CREATE_LORA_DEFAULTS,
  defaultTestPrompt,
  estimateMinutes,
  initialCreateLoraState,
  localProblem,
  setKeyword,
  setKind,
  stateFromSnapshot,
} from './createLoraForm'
import type { DatasetSnapshot } from '../../api/types'

describe('createLoraForm', () => {
  it('defaults match the approved dialog', () => {
    const s = initialCreateLoraState('m1', 'Lo-fi', () => 0.5)
    expect(s.epochs).toBe('4')
    expect(s.saveEvery).toBe('1')
    expect(s.holdoutCount).toBe('2')
    expect(s.kind).toBe('standard')
    expect(s.testPrompt).toBe('Lo-fi')
    expect(s.baseModelId).toBe('m1')
    expect(Number.isInteger(s.holdoutSeed)).toBe(true)
    expect(CREATE_LORA_DEFAULTS.rank).toBe(32)
  })

  it('train kind sets epochs', () => {
    expect(setKind(initialCreateLoraState('m', 'G'), 'quick').epochs).toBe('1')
    expect(setKind(initialCreateLoraState('m', 'G'), 'thorough').epochs).toBe('8')
    const custom = setKind({ ...initialCreateLoraState('m', 'G'), epochs: '5' }, 'custom')
    expect(custom.kind).toBe('custom')
    expect(custom.epochs).toBe('5')
  })

  it('keyword refills the test prompt until the user edits it', () => {
    let s = setKeyword(initialCreateLoraState('m', 'Lo-fi'), 'lushlofi', 'Lo-fi')
    expect(s.testPrompt).toBe('lushlofi, Lo-fi')
    expect(defaultTestPrompt('', 'Lo-fi')).toBe('Lo-fi')
    s = { ...s, testPrompt: 'mine', testPromptEdited: true }
    expect(setKeyword(s, 'x', 'Lo-fi').testPrompt).toBe('mine')
  })

  it('builds the request without excluded tracks and with numbers', () => {
    const s = { ...initialCreateLoraState('m', 'G'), name: ' A ', excluded: ['t2'] }
    const r = buildLoraRequest(s, ['t1', 't2', 't3'])!
    expect(r.track_ids).toEqual(['t1', 't3'])
    expect(r.name).toBe('A')
    expect(r.epochs).toBe(4)
    expect(r.seed).toBeNull()
  })

  it('localProblem flags bad numbers and buildLoraRequest returns null', () => {
    const base = initialCreateLoraState('m', 'G')
    expect(localProblem(base)).toBeNull()
    const bad = { ...base, saveEvery: '9' }
    expect(localProblem(bad)).toMatch(/Save every/)
    expect(buildLoraRequest(bad, ['t1'])).toBeNull()
    expect(localProblem({ ...base, seed: 'abc' })).toMatch(/Seed/)
    expect(localProblem({ ...base, seed: '2147483647' })).toBeNull()
    expect(localProblem({ ...base, seed: '2147483648' })).toMatch(/2147483647/)
    expect(localProblem({ ...base, baseModelId: '' })).toMatch(/Base model/)
  })

  it('estimateMinutes uses the latest completed run with steps, else null', () => {
    expect(estimateMinutes([], 16)).toBeNull()
    const run = (over: object) =>
      makeJob({ type: 'train_lora', status: 'completed', ...over })
    expect(estimateMinutes([run({ result: { train_seconds: 120, optimizer_steps: 8 } })], 16)).toBe(4)
    expect(estimateMinutes([run({ result: { train_seconds: 100, optimizer_steps: 8 } })], 16)).toBe(4)
    expect(estimateMinutes([makeJob({ status: 'failed', result: { train_seconds: 1, optimizer_steps: 1 } })], 16)).toBeNull()
    expect(estimateMinutes([run({ result: null })], 16)).toBeNull()
  })
})

describe('stateFromSnapshot', () => {
  const sample = (id: string) => ({ catalog_track_id: id }) as DatasetSnapshot['content']['samples'][number]
  function snap(epochs: number): DatasetSnapshot {
    return {
      id: 's1',
      catalog_id: 'c1',
      job_id: 'j1',
      lora_key: 'k',
      created_at: '',
      content: {
        schema: 1,
        catalog: { id: 'c1', name: 'Cat' },
        genre: { id: 'g', name: 'Lofi' },
        lora: { key: 'k', name: 'Rain' },
        keyword: 'lushrain',
        test_prompt: 'lushrain, soft',
        holdout_seed: 777,
        base_model: { id: 'bm', key: 'ace' },
        config: {
          rank: 16,
          alpha: 32,
          learning_rate: 0.0002,
          epochs,
          gradient_accumulation: 2,
          save_every: 2,
          warmup_steps: 5,
          seed: 42,
          max_duration_s: 240,
        },
        limits: { max_seconds: 240 },
        samples: [sample('a'), sample('b')],
        held_out: [sample('h')],
        models: [],
      },
    } as DatasetSnapshot
  }

  it('copies the config and names the new LoRA "(2)"', () => {
    const s = stateFromSnapshot(snap(4), 'Lofi')
    expect(s.name).toBe('Rain (2)')
    expect(s.kind).toBe('standard')
    expect(s.epochs).toBe('4')
    expect(s.holdoutSeed).toBe(777)
    expect(s.holdoutCount).toBe('1')
    expect(s.keyword).toBe('lushrain')
    expect(s.testPrompt).toBe('lushrain, soft')
    expect(s.testPromptEdited).toBe(true)
    expect(s.baseModelId).toBe('bm')
    expect(s.rank).toBe('16')
    expect(s.learningRate).toBe('0.0002')
    expect(s.saveEvery).toBe('2')
    expect(s.seed).toBe('42')
    expect(localProblem(s)).toBeNull()
  })

  it('derives the kind from epochs', () => {
    expect(stateFromSnapshot(snap(1), 'L').kind).toBe('quick')
    expect(stateFromSnapshot(snap(8), 'L').kind).toBe('thorough')
    expect(stateFromSnapshot(snap(5), 'L').kind).toBe('custom')
  })
})
