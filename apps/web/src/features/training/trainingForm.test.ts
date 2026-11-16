import { describe, expect, it } from 'vitest'
import type { Dataset } from '../../api/types'
import {
  STALE_AFTER_MS,
  buildTrainingRequest,
  datasetLabel,
  defaultTrainingFormState,
  isStale,
  stageLabel,
  stepPlan,
  trainableDatasets,
  validateTrainingForm,
} from './trainingForm'
import type { TrainingFormState } from './trainingForm'

const valid: TrainingFormState = {
  ...defaultTrainingFormState,
  name: 'Lofi Style',
  datasetKey: 'lofi-set',
  baseModelId: 'm-ace',
}

function dataset(overrides: Partial<Dataset> = {}): Dataset {
  return {
    key: 'lofi-set',
    node_id: 'n1',
    node_name: 'render-node-01',
    online: true,
    can_train: true,
    info: { key: 'lofi-set', samples: 10, seconds: 754 },
    ...overrides,
  }
}

describe('validateTrainingForm', () => {
  it('accepts the pre-filled configuration once name, dataset and model are set', () => {
    expect(validateTrainingForm(valid)).toBeNull()
  })

  it('requires name, dataset and base model', () => {
    expect(validateTrainingForm({ ...valid, name: '  ' })).toBe('Hãy nhập Tên Train Style.')
    expect(validateTrainingForm({ ...valid, datasetKey: '' })).toBe('Hãy chọn Dataset.')
    expect(validateTrainingForm({ ...valid, baseModelId: '' })).toBe('Hãy chọn Base model.')
  })

  it.each([
    ['rank', '0'],
    ['rank', '129'],
    ['rank', '1.5'],
    ['alpha', '257'],
    ['learningRate', '0'],
    ['learningRate', '0.02'],
    ['learningRate', ''],
    ['epochs', '0'],
    ['epochs', '1001'],
    ['gradientAccumulation', '33'],
    ['saveEvery', '0'],
    ['saveEvery', '5'], // more than epochs (4)
    ['warmupSteps', '-1'],
    ['warmupSteps', '10001'],
    ['maxMinutes', '0'],
    ['maxMinutes', '1441'],
    ['seed', '-3'],
    ['seed', '1.2'],
  ] as [keyof TrainingFormState, string][])('rejects %s = "%s"', (key, value) => {
    expect(validateTrainingForm({ ...valid, [key]: value })).not.toBeNull()
  })

  it('accepts the edges of every range', () => {
    expect(
      validateTrainingForm({
        ...valid,
        rank: '128',
        alpha: '256',
        learningRate: '0.01',
        epochs: '1000',
        gradientAccumulation: '32',
        saveEvery: '1000',
        warmupSteps: '0',
        maxMinutes: '1440',
        seed: '0',
      }),
    ).toBeNull()
  })
})

describe('buildTrainingRequest', () => {
  it('builds the POST /training-runs body with numbers and a null seed', () => {
    expect(buildTrainingRequest({ ...valid, name: '  Lofi Style ' })).toEqual({
      name: 'Lofi Style',
      dataset_key: 'lofi-set',
      base_model_id: 'm-ace',
      rank: 32,
      alpha: 64,
      learning_rate: 0.0001,
      epochs: 4,
      gradient_accumulation: 4,
      save_every: 2,
      warmup_steps: 2,
      seed: null,
      max_minutes: 30,
    })
    expect(buildTrainingRequest({ ...valid, seed: '42' }).seed).toBe(42)
  })
})

describe('stepPlan', () => {
  it('computes optimizer steps per epoch as ceil(samples / gradient accumulation)', () => {
    expect(stepPlan(10, 4, 4)).toEqual({ samples: 10, stepsPerEpoch: 3, totalSteps: 12 })
    expect(stepPlan(8, 4, 4)).toEqual({ samples: 8, stepsPerEpoch: 2, totalSteps: 8 })
    expect(stepPlan(1, 32, 10)).toEqual({ samples: 1, stepsPerEpoch: 1, totalSteps: 10 })
  })

  it('returns null instead of guessing when an input is unknown', () => {
    expect(stepPlan(null, 4, 4)).toBeNull()
    expect(stepPlan(undefined, 4, 4)).toBeNull()
    expect(stepPlan(10, null, 4)).toBeNull()
    expect(stepPlan(10, 0, 4)).toBeNull()
    expect(stepPlan(10, 4, null)).toBeNull()
  })
})

describe('datasets', () => {
  it('keeps only trainable datasets, one per key', () => {
    const list = [
      dataset({ key: 'a' }),
      dataset({ key: 'a', node_id: 'n2' }),
      dataset({ key: 'b', can_train: false }),
      dataset({ key: 'c' }),
    ]
    expect(trainableDatasets(list).map((d) => d.key)).toEqual(['a', 'c'])
  })

  it('labels a dataset with samples and total minutes, or says they are unknown', () => {
    expect(datasetLabel(dataset())).toBe('lofi-set — 10 mẫu · 12.6 phút')
    expect(datasetLabel(dataset({ info: null }))).toBe('lofi-set — chưa rõ số mẫu · chưa rõ thời lượng')
  })
})

describe('stageLabel / isStale', () => {
  it('translates known stages and passes unknown ones through', () => {
    expect(stageLabel('stopping_inference')).toBe('Đang dừng engine tạo nhạc')
    expect(stageLabel('training')).toBe('Đang train')
    expect(stageLabel('something_new')).toBe('something_new')
    expect(stageLabel(null)).toBe('—')
  })

  it('is stale only for preparing/running runs silent for more than 90 seconds', () => {
    const now = Date.parse('2026-01-01T00:10:00.000Z')
    const old = new Date(now - STALE_AFTER_MS - 1000).toISOString()
    const fresh = new Date(now - STALE_AFTER_MS + 1000).toISOString()
    expect(isStale('running', old, now)).toBe(true)
    expect(isStale('preparing', old, now)).toBe(true)
    expect(isStale('running', fresh, now)).toBe(false)
    expect(isStale('queued', old, now)).toBe(false)
    expect(isStale('completed', old, now)).toBe(false)
    expect(isStale('running', null, now)).toBe(false)
  })
})
