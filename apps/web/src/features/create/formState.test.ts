import { describe, expect, it } from 'vitest'
import { makeGenre, makePreset } from '../../test/utils'
import {
  applyGenre,
  applyPreset,
  applySubgenre,
  buildGenerationRequest,
  durationRangeFor,
  emptyFormState,
  stateFromRequest,
  validate,
} from './formState'
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

describe('suggested duration range', () => {
  const jazz = makeGenre({ id: 'g-jazz', name: 'Jazz', duration_min: 60, duration_max: 120 })
  const bebop = makeGenre({ id: 'g-bebop', name: 'Bebop', parent_id: 'g-jazz', duration_min: 90, duration_max: 150 })
  const swing = makeGenre({ id: 'g-swing', name: 'Swing', parent_id: 'g-jazz' })
  const lofi = makeGenre({ id: 'g-lofi', name: 'Lo-fi', duration_min: 30, duration_max: 45 })
  const plain = makeGenre({ id: 'g-plain', name: 'Plain' })
  const genres = [jazz, bebop, swing, lofi, plain]
  const club = makePreset({ id: 'p-club', name: 'Smoky Club', genre_id: 'g-jazz', duration_min: 180, duration_max: 300 })
  const open = makePreset({ id: 'p-open', name: 'Open Style', genre_id: 'g-jazz' })
  const presets = [club, open]

  it('the most specific range wins: Style, then subgenre, then genre', () => {
    const pick = (genreId: string, subgenreId: string, presetId: string) =>
      durationRangeFor(genres, presets, { genreId, subgenreId, presetId })
    expect(pick('g-jazz', '', '')).toEqual({ min: 60, max: 120, source: 'Jazz' })
    expect(pick('g-jazz', 'g-bebop', '')).toEqual({ min: 90, max: 150, source: 'Bebop' })
    expect(pick('g-jazz', 'g-bebop', 'p-club')).toEqual({ min: 180, max: 300, source: 'Smoky Club' })
  })

  it('an empty range falls back to the next level; nothing set means no suggestion', () => {
    const pick = (genreId: string, subgenreId: string, presetId: string) =>
      durationRangeFor(genres, presets, { genreId, subgenreId, presetId })
    expect(pick('g-jazz', 'g-swing', '')?.source).toBe('Jazz')
    expect(pick('g-jazz', 'g-bebop', 'p-open')?.source).toBe('Bebop')
    expect(pick('g-plain', '', '')).toBeNull()
    expect(pick('', '', '')).toBeNull()
  })

  it('selecting a genre, subgenre or Style jumps to the upper end, even over a dragged value', () => {
    const dragged = { ...base, duration: 75 }
    const onJazz = applyGenre(dragged, 'g-jazz', genres, presets)
    expect(onJazz.duration).toBe(120)
    const onBebop = applySubgenre({ ...onJazz, duration: 100 }, 'g-bebop', genres, presets)
    expect(onBebop.duration).toBe(150)
    const onClub = applyPreset({ ...onBebop, duration: 100 }, club, genres, [])
    expect(onClub.duration).toBe(300)
  })

  it('changing again re-applies: dropping the Style falls back, another genre drops the Style', () => {
    const onClub = applyPreset(applyGenre(base, 'g-jazz', genres, presets), club, genres, [])
    expect(applyPreset({ ...onClub, duration: 200 }, undefined, genres, []).duration).toBe(120)
    const onLofi = applyGenre({ ...onClub, duration: 200 }, 'g-lofi', genres, presets)
    expect(onLofi.presetId).toBe('')
    expect(onLofi.duration).toBe(45)
  })

  it('keeps the duration when the new selection has no range at all', () => {
    expect(applyGenre({ ...base, duration: 75 }, 'g-plain', genres, presets).duration).toBe(75)
  })
})
