import { Pause, Play, Volume2, VolumeX } from 'lucide-react'
import { Link } from 'react-router-dom'
import { formatDuration } from '../lib/format'
import { usePlayer } from './PlayerContext'

export function PlayerBar() {
  const player = usePlayer()
  const { current, isPlaying, currentTime, duration, volume, error } = player
  const hasDuration = duration > 0

  return (
    <section className="player-bar" aria-label="Trình phát nhạc">
      <div className="player-track">
        <button
          type="button"
          className="play-btn lg is-active"
          disabled={!current}
          aria-label={isPlaying ? 'Tạm dừng' : 'Phát'}
          onClick={() => player.toggle()}
        >
          {isPlaying ? <Pause size={20} aria-hidden /> : <Play size={20} aria-hidden />}
        </button>
        <div className="player-info">
          {current ? (
            <>
              {current.trackId ? (
                <Link to={`/tracks/${current.trackId}`} className="player-title">
                  {current.title}
                </Link>
              ) : (
                <span className="player-title">{current.title}</span>
              )}
              <span className={error ? 'player-sub is-error' : 'player-sub'} role={error ? 'alert' : undefined}>
                {error ?? current.subtitle}
              </span>
            </>
          ) : (
            <span className="player-sub">Chưa chọn track nào để phát</span>
          )}
        </div>
      </div>

      <div className="player-seek">
        <span className="player-time">{formatDuration(current ? currentTime : null)}</span>
        <input
          type="range"
          className="range"
          aria-label="Vị trí phát"
          min={0}
          max={hasDuration ? duration : 1}
          step={0.1}
          value={hasDuration ? Math.min(currentTime, duration) : 0}
          disabled={!current || !hasDuration}
          style={{ ['--fill' as string]: `${hasDuration ? (currentTime / duration) * 100 : 0}%` }}
          onChange={(e) => player.seek(Number(e.target.value))}
        />
        <span className="player-time">{formatDuration(current && hasDuration ? duration : null)}</span>
      </div>

      <div className="player-volume">
        <button
          type="button"
          className="icon-btn"
          aria-label={volume === 0 ? 'Bật tiếng' : 'Tắt tiếng'}
          onClick={() => player.setVolume(volume === 0 ? 1 : 0)}
        >
          {volume === 0 ? <VolumeX size={18} aria-hidden /> : <Volume2 size={18} aria-hidden />}
        </button>
        <input
          type="range"
          className="range"
          aria-label="Âm lượng"
          min={0}
          max={1}
          step={0.01}
          value={volume}
          style={{ ['--fill' as string]: `${volume * 100}%` }}
          onChange={(e) => player.setVolume(Number(e.target.value))}
        />
      </div>
    </section>
  )
}
