import { describe, expect, it } from 'vitest'
import { makeTrack } from '../test/utils'
import { pinTrackSettings } from './track'

describe('pinTrackSettings', () => {
  it('fills blank seed / BPM / Key / Time Signature with what the track was really made with', () => {
    const track = makeTrack({ seed: 1234, bpm: 40, key: 'C minor', time_signature: '2/4' })
    const pinned = pinTrackSettings({ prompt: 'Lo-fi', seed: null, bpm: null, key: null, time_signature: null }, track)
    expect(pinned).toMatchObject({ prompt: 'Lo-fi', seed: 1234, bpm: 40, key: 'C minor', time_signature: '2/4' })
  })

  it('keeps what the user typed in the original request', () => {
    const track = makeTrack({ seed: 1234, bpm: 40, key: 'C minor', time_signature: '2/4' })
    const pinned = pinTrackSettings({ prompt: 'Lo-fi', seed: 7, bpm: 80, key: 'A minor', time_signature: '4/4' }, track)
    expect(pinned).toMatchObject({ seed: 7, bpm: 80, key: 'A minor', time_signature: '4/4' })
  })
})
