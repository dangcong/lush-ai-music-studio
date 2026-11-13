import { describe, expect, it } from 'vitest'
import { buildGenerationRequest, emptyFormState, stateFromRequest, validate } from './formState'
import type { GenerateFormState } from './formState'

const base: GenerateFormState = {
  ...emptyFormState,
  prompt: 'Warm jazz piano',
  modelId: 'm-1',
}

describe('buildGenerationRequest', () => {
  it('without a Reference Audio it is a text job', () => {
    const body = buildGenerationRequest(base, 'key')
    expect(body).toMatchObject({ mode: 'text', reference_audio_id: null, source_audio_id: null })
    expect(body.params).toEqual({ inference_steps: null, guidance_scale: null })
  })

  it('with a Reference Audio it is a reference job', () => {
    const body = buildGenerationRequest({ ...base, referenceAudioId: 'in-ref' }, 'key')
    expect(body).toMatchObject({ mode: 'reference', reference_audio_id: 'in-ref', source_audio_id: null })
    expect(body.params).not.toHaveProperty('audio_cover_strength')
  })
})

describe('validate', () => {
  it('never requires an audio file', () => {
    expect(validate(base)).toBeNull()
    expect(validate({ ...base, referenceAudioId: 'in-ref' })).toBeNull()
  })
})

describe('stateFromRequest', () => {
  it('restores the Reference Audio of a reference request', () => {
    const state = stateFromRequest({ prompt: 'Drone', mode: 'reference', reference_audio_id: 'in-ref' })
    expect(state.referenceAudioId).toBe('in-ref')
  })

  it('refills an old cover request as a text one', () => {
    const state = stateFromRequest({
      prompt: 'String quartet',
      mode: 'cover',
      source_audio_id: 'in-src',
      reference_audio_id: null,
      params: { inference_steps: null, guidance_scale: null, audio_cover_strength: 0.8 },
    })
    expect(state.referenceAudioId).toBe('')
    expect(buildGenerationRequest(state, 'k')).toMatchObject({ mode: 'text', source_audio_id: null })
  })

  it('treats a request from before the modes existed as text', () => {
    expect(stateFromRequest({ prompt: 'Old job' }).referenceAudioId).toBe('')
  })

  it('round-trips through buildGenerationRequest', () => {
    const body = buildGenerationRequest({ ...base, referenceAudioId: 'in-ref' }, 'key')
    expect(buildGenerationRequest(stateFromRequest(body), 'key')).toEqual(body)
  })
})
