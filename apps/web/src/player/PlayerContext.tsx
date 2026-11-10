import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react'
import type { ReactNode } from 'react'
import { artifactUrl } from '../api/client'

export interface PlayerItem {
  /** Artifact whose content is streamed from /api/v1/artifacts/{id}/content. */
  artifactId: string
  title: string
  subtitle?: string
  trackId?: string
}

export interface PlayerContextValue {
  current: PlayerItem | null
  isPlaying: boolean
  currentTime: number
  duration: number
  volume: number
  error: string | null
  /** Starts the given item, or resumes it if it is already loaded. */
  play: (item: PlayerItem) => void
  pause: () => void
  /** Play/pause the given item; without an argument toggles the current one. */
  toggle: (item?: PlayerItem) => void
  seek: (seconds: number) => void
  setVolume: (volume: number) => void
  stop: () => void
}

const PlayerContext = createContext<PlayerContextValue | null>(null)

export function PlayerProvider({ children }: { children: ReactNode }) {
  const audioRef = useRef<HTMLAudioElement>(null)
  const [current, setCurrent] = useState<PlayerItem | null>(null)
  const [isPlaying, setIsPlaying] = useState(false)
  const [currentTime, setCurrentTime] = useState(0)
  const [duration, setDuration] = useState(0)
  const [volume, setVolumeState] = useState(1)
  const [error, setError] = useState<string | null>(null)

  const startPlayback = useCallback(() => {
    const audio = audioRef.current
    if (!audio) return
    // play() rejects when interrupted by a newer load; the <audio> events keep state right.
    void Promise.resolve(audio.play()).catch(() => {})
  }, [])

  const play = useCallback(
    (item: PlayerItem) => {
      const audio = audioRef.current
      if (!audio) return
      if (current?.artifactId !== item.artifactId) {
        audio.src = artifactUrl(item.artifactId)
        setCurrentTime(0)
        setDuration(0)
      }
      setError(null)
      setCurrent(item)
      startPlayback()
    },
    [current, startPlayback],
  )

  const pause = useCallback(() => {
    audioRef.current?.pause()
  }, [])

  const toggle = useCallback(
    (item?: PlayerItem) => {
      const target = item ?? current
      if (!target) return
      const isCurrent = current?.artifactId === target.artifactId
      if (isCurrent && isPlaying) pause()
      else play(target)
    },
    [current, isPlaying, pause, play],
  )

  const seek = useCallback((seconds: number) => {
    const audio = audioRef.current
    if (!audio) return
    audio.currentTime = seconds
    setCurrentTime(seconds)
  }, [])

  const setVolume = useCallback((value: number) => {
    const clamped = Math.min(1, Math.max(0, value))
    if (audioRef.current) audioRef.current.volume = clamped
    setVolumeState(clamped)
  }, [])

  const stop = useCallback(() => {
    const audio = audioRef.current
    if (audio) {
      audio.pause()
      audio.removeAttribute('src')
      audio.load()
    }
    setCurrent(null)
    setIsPlaying(false)
    setCurrentTime(0)
    setDuration(0)
    setError(null)
  }, [])

  // Space toggles playback when focus is not on an interactive control.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.code !== 'Space' || e.defaultPrevented || !current) return
      const el = e.target as HTMLElement | null
      if (el?.closest('input, textarea, select, button, a, [contenteditable], [role="dialog"]')) return
      e.preventDefault()
      toggle()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [current, toggle])

  const value = useMemo<PlayerContextValue>(
    () => ({
      current,
      isPlaying,
      currentTime,
      duration,
      volume,
      error,
      play,
      pause,
      toggle,
      seek,
      setVolume,
      stop,
    }),
    [current, isPlaying, currentTime, duration, volume, error, play, pause, toggle, seek, setVolume, stop],
  )

  return (
    <PlayerContext.Provider value={value}>
      {children}
      {/* The single <audio> element of the app; it lives above the router so playback survives navigation. */}
      <audio
        ref={audioRef}
        data-testid="player-audio"
        preload="metadata"
        onPlay={() => setIsPlaying(true)}
        onPause={() => setIsPlaying(false)}
        onEnded={() => setIsPlaying(false)}
        onTimeUpdate={(e) => setCurrentTime(e.currentTarget.currentTime)}
        onDurationChange={(e) => {
          const d = e.currentTarget.duration
          setDuration(Number.isFinite(d) ? d : 0)
        }}
        onError={() => {
          // Ignore the error fired by clearing the source in stop().
          if (!audioRef.current?.getAttribute('src')) return
          setIsPlaying(false)
          setError('Không phát được file audio này.')
        }}
      />
    </PlayerContext.Provider>
  )
}

export function usePlayer(): PlayerContextValue {
  const ctx = useContext(PlayerContext)
  if (!ctx) throw new Error('usePlayer must be used inside PlayerProvider')
  return ctx
}
