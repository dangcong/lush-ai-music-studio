import { useEffect, useRef, useState } from 'react'
import WaveSurfer from 'wavesurfer.js'
import { artifactUrl } from '../api/client'
import { usePlayer } from '../player/PlayerContext'
import type { PlayerItem } from '../player/PlayerContext'
import { Spinner } from './ui'

function cssVar(name: string, fallback: string): string {
  const value = getComputedStyle(document.documentElement).getPropertyValue(name).trim()
  return value || fallback
}

/**
 * Large, seekable waveform. WaveSurfer only draws here: sound always comes from the
 * global player's single <audio> element, and the cursor follows that player.
 */
export function Waveform({ item, height = 128 }: { item: PlayerItem; height?: number }) {
  const containerRef = useRef<HTMLDivElement>(null)
  const waveRef = useRef<WaveSurfer | null>(null)
  const [status, setStatus] = useState<'loading' | 'ready' | 'error'>('loading')
  const player = usePlayer()
  const playerRef = useRef(player)
  playerRef.current = player
  const itemRef = useRef(item)
  itemRef.current = item

  const artifactId = item.artifactId

  useEffect(() => {
    const container = containerRef.current
    if (!container) return
    setStatus('loading')
    const wave = WaveSurfer.create({
      container,
      height,
      url: artifactUrl(artifactId),
      fetchParams: { credentials: 'include' },
      waveColor: cssVar('--wave', '#3a4a44'),
      progressColor: cssVar('--accent', '#62d6a2'),
      cursorColor: cssVar('--text', '#e8eeea'),
      cursorWidth: 1,
      barWidth: 2,
      barGap: 2,
      barRadius: 2,
      normalize: true,
      interact: true,
      dragToSeek: true,
    })
    waveRef.current = wave
    wave.on('ready', () => setStatus('ready'))
    wave.on('error', () => setStatus('error'))
    wave.on('interaction', (time) => {
      const p = playerRef.current
      if (p.current?.artifactId !== artifactId) p.play(itemRef.current)
      p.seek(time)
    })
    return () => {
      waveRef.current = null
      wave.destroy()
    }
  }, [artifactId, height])

  const isCurrent = player.current?.artifactId === artifactId
  const time = isCurrent ? player.currentTime : 0

  useEffect(() => {
    if (status === 'ready') waveRef.current?.setTime(time)
  }, [time, status])

  return (
    <div className="waveform" style={{ minHeight: height }}>
      <div ref={containerRef} className="waveform-canvas" aria-hidden />
      {status === 'loading' && (
        <div className="waveform-overlay">
          <Spinner label="Đang tải waveform" /> Đang tải waveform…
        </div>
      )}
      {status === 'error' && (
        <div className="waveform-overlay" role="alert">
          Không dựng được waveform cho file này.
        </div>
      )}
    </div>
  )
}
