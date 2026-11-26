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
  applyAuto,
  recommendedEpochs,
  autoSaveEvery,
  autoWarmup,
  trainSongCount,
} from './createLoraForm'
import type { DatasetSnapshot } from '../../api/types'

describe('createLoraForm', () => {
  it('defaults: rank 64 / alpha 128, lr 1e-4, 720 min', () => {
    const s = initialCreateLoraState('m1', 'Lo-fi', () => 0.5)
    expect(s.holdoutCount).toBe('2')
    expect(s.kind).toBe('standard')
    expect(s.saveEveryAuto).toBe(true)
    expect(s.warmupAuto).toBe(true)
    expect(s.testPrompt).toBe('Lo-fi')
    expect(s.baseModelId).toBe('m1')
    expect(Number.isInteger(s.holdoutSeed)).toBe(true)
    expect(CREATE_LORA_DEFAULTS).toMatchObject({ rank: 64, alpha: 128, learning_rate: 0.0001, gradient_accumulation: 4, max_minutes: 720 })
  })

  it('recommended epochs follow the ACE-Step tutorial and Run A/B by number of songs', () => {
    expect(recommendedEpochs('standard', 8)).toBe(500)
    expect(recommendedEpochs('standard', 25)).toBe(500)
    expect(recommendedEpochs('thorough', 20)).toBe(800)
    expect(recommendedEpochs('standard', 26)).toBe(300)
    expect(recommendedEpochs('thorough', 60)).toBe(500)
    expect(recommendedEpochs('standard', 61)).toBe(150)
    expect(recommendedEpochs('thorough', 95)).toBe(250)
  })

  it('auto save gives about 10 checkpoints and auto warmup about 5% of the steps (max 100)', () => {
    expect(autoSaveEvery(150)).toBe(15)
    expect(autoSaveEvery(1)).toBe(1)
    expect(autoSaveEvery(225)).toBe(23)
    expect(autoWarmup(20, 4, 150)).toBe(38) // 5 steps/epoch x 150 = 750 -> 37.5
    expect(autoWarmup(200, 4, 80)).toBe(100)
    expect(autoWarmup(3, 4, 1)).toBe(0)
  })

  it('train song count is the ticked tracks minus excluded and held-out ones, at least 1', () => {
    const s = { ...initialCreateLoraState('m', 'G'), excluded: ['t2'] }
    expect(trainSongCount(s, ['t1', 't2', 't3', 't4', 't5'])).toBe(2)
    expect(trainSongCount(s, ['t1'])).toBe(1)
  })

  it('applyAuto fills epochs, save and warmup until the user edits them', () => {
    const ids = Array.from({ length: 22 }, (_, i) => `t${i}`) // 20 to train + 2 held out
    const a = applyAuto(initialCreateLoraState('m', 'G'), ids)
    expect([a.epochs, a.saveEvery, a.warmupSteps]).toEqual(['500', '50', '100'])
    const mine = applyAuto({ ...initialCreateLoraState('m', 'G'), kind: 'custom', epochs: '40', saveEvery: '7', saveEveryAuto: false }, ids)
    expect([mine.epochs, mine.saveEvery, mine.warmupSteps]).toEqual(['40', '7', '10'])
  })

  it('train kind switches back to the recommended epochs; custom keeps the typed value', () => {
    const custom = setKind({ ...initialCreateLoraState('m', 'G'), epochs: '5' }, 'custom')
    expect(custom.kind).toBe('custom')
    expect(custom.epochs).toBe('5')
    expect(applyAuto(setKind(custom, 'standard'), ['a', 'b', 'c']).epochs).toBe('500')
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
    expect(r.epochs).toBe(500)
    expect(r.seed).toBeNull()
  })

  it('localProblem flags bad numbers and buildLoraRequest returns null', () => {
    const base = initialCreateLoraState('m', 'G')
    expect(localProblem(base)).toBeNull()
    const bad = { ...base, saveEvery: '501' } // more than the 500 epochs
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
        limits: { max_seconds: 5400 },
        samples: [sample('a'), sample('b')],
        held_out: [sample('h')],
        models: [],
      },
    } as DatasetSnapshot
  }

  it('copies the config and names the new LoRA "(2)"', () => {
    const s = stateFromSnapshot(snap(4), 'Lofi')
    expect(s.name).toBe('Rain (2)')
    expect(s.kind).toBe('custom') // the old numbers are kept as they were, not recomputed
    expect(s.epochs).toBe('4')
    expect([s.saveEveryAuto, s.warmupAuto]).toEqual([false, false])
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
    expect(s.maxMinutes).toBe('90')
    expect(localProblem(s)).toBeNull()
  })

  it('any snapshot reopens as "Tự nhập", including an old 1-epoch run', () => {
    expect(stateFromSnapshot(snap(1), 'L').kind).toBe('custom')
    expect(stateFromSnapshot(snap(150), 'L').kind).toBe('custom')
  })
})
