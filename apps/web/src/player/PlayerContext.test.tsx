import { act, fireEvent, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { PlayerProvider, usePlayer } from './PlayerContext'
import type { PlayerItem } from './PlayerContext'

const first: PlayerItem = { artifactId: 'art-1', title: 'First', trackId: 't1' }
const second: PlayerItem = { artifactId: 'art-2', title: 'Second', trackId: 't2' }

function Probe() {
  const player = usePlayer()
  return (
    <div>
      <output data-testid="state">
        {JSON.stringify({
          title: player.current?.title ?? null,
          playing: player.isPlaying,
          time: player.currentTime,
          volume: player.volume,
          error: player.error,
        })}
      </output>
      <button onClick={() => player.toggle(first)}>toggle first</button>
      <button onClick={() => player.toggle(second)}>toggle second</button>
      <button onClick={() => player.toggle()}>toggle current</button>
      <button onClick={() => player.seek(30)}>seek</button>
      <button onClick={() => player.setVolume(0.25)}>volume</button>
      <button onClick={() => player.stop()}>stop</button>
    </div>
  )
}

function setup() {
  render(
    <PlayerProvider>
      <Probe />
    </PlayerProvider>,
  )
  const audio = screen.getByTestId('player-audio') as HTMLAudioElement
  const state = () => JSON.parse(screen.getByTestId('state').textContent ?? '{}') as Record<string, unknown>
  return { audio, state, user: userEvent.setup() }
}

describe('PlayerContext', () => {
  it('starts empty and uses a single <audio> element', () => {
    const { state } = setup()
    expect(state()).toMatchObject({ title: null, playing: false })
    expect(document.querySelectorAll('audio')).toHaveLength(1)
  })

  it('plays an artifact from the content endpoint', async () => {
    const { audio, state, user } = setup()
    await user.click(screen.getByText('toggle first'))

    expect(audio).toHaveAttribute('src', '/api/v1/artifacts/art-1/content')
    expect(audio.play).toHaveBeenCalled()
    expect(state()).toMatchObject({ title: 'First', playing: true })
  })

  it('toggles pause / resume without reloading the source', async () => {
    const { audio, state, user } = setup()
    await user.click(screen.getByText('toggle first'))
    await user.click(screen.getByText('toggle first'))
    expect(state()).toMatchObject({ title: 'First', playing: false })

    await user.click(screen.getByText('toggle current'))
    expect(state()).toMatchObject({ title: 'First', playing: true })
    expect(audio).toHaveAttribute('src', '/api/v1/artifacts/art-1/content')
  })

  it('switches to another track in the same element', async () => {
    const { audio, state, user } = setup()
    await user.click(screen.getByText('toggle first'))
    await user.click(screen.getByText('toggle second'))

    expect(audio).toHaveAttribute('src', '/api/v1/artifacts/art-2/content')
    expect(state()).toMatchObject({ title: 'Second', playing: true })
    expect(document.querySelectorAll('audio')).toHaveLength(1)
  })

  it('seeks, changes volume and follows the element time', async () => {
    const { audio, state, user } = setup()
    await user.click(screen.getByText('toggle first'))
    await user.click(screen.getByText('seek'))
    expect(audio.currentTime).toBe(30)
    expect(state()).toMatchObject({ time: 30 })

    await user.click(screen.getByText('volume'))
    expect(audio.volume).toBe(0.25)
    expect(state()).toMatchObject({ volume: 0.25 })

    act(() => {
      audio.currentTime = 45
      fireEvent.timeUpdate(audio)
    })
    expect(state()).toMatchObject({ time: 45 })
  })

  it('stops playing when the track ends or fails to load', async () => {
    const { audio, state, user } = setup()
    await user.click(screen.getByText('toggle first'))
    fireEvent.ended(audio)
    expect(state()).toMatchObject({ playing: false, error: null })

    await user.click(screen.getByText('toggle first'))
    fireEvent.error(audio)
    expect(state()).toMatchObject({ playing: false, error: 'Không phát được file audio này.' })
  })

  it('stop() clears the current track', async () => {
    const { audio, state, user } = setup()
    await user.click(screen.getByText('toggle first'))
    await user.click(screen.getByText('stop'))

    expect(state()).toMatchObject({ title: null, playing: false, time: 0 })
    expect(audio).not.toHaveAttribute('src')
  })
})
