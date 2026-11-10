import { Download, Heart, Pause, Play, Star } from 'lucide-react'
import { artifactUrl } from '../api/client'
import { useUpdateTrack } from '../api/mutations'
import type { Artifact, Track } from '../api/types'
import { cx } from '../lib/cx'
import { artifactFormat, toPlayerItem } from '../lib/track'
import { usePlayer } from '../player/PlayerContext'

export function PlayButton({ track, large }: { track: Track; large?: boolean }) {
  const player = usePlayer()
  const item = toPlayerItem(track)
  const active = item !== null && player.current?.artifactId === item.artifactId
  const playing = active && player.isPlaying
  const size = large ? 22 : 18

  return (
    <button
      type="button"
      className={cx('play-btn', large && 'lg', active && 'is-active')}
      disabled={!item}
      title={item ? undefined : 'Track này chưa có file audio'}
      aria-label={playing ? `Tạm dừng ${track.title}` : `Phát ${track.title}`}
      onClick={() => item && player.toggle(item)}
    >
      {playing ? <Pause size={size} aria-hidden /> : <Play size={size} aria-hidden />}
    </button>
  )
}

export function FavoriteButton({ track }: { track: Track }) {
  const update = useUpdateTrack()
  return (
    <button
      type="button"
      className={cx('icon-btn', track.favorite && 'is-fav')}
      aria-pressed={track.favorite}
      aria-label={track.favorite ? 'Bỏ yêu thích' : 'Yêu thích'}
      title={track.favorite ? 'Bỏ yêu thích' : 'Yêu thích'}
      disabled={update.isPending}
      onClick={() => update.mutate({ id: track.id, input: { favorite: !track.favorite } })}
    >
      <Heart size={17} aria-hidden fill={track.favorite ? 'currentColor' : 'none'} />
    </button>
  )
}

const STARS = [1, 2, 3, 4, 5]

export function StarRating({ track }: { track: Track }) {
  const update = useUpdateTrack()
  return (
    <div className="stars" role="group" aria-label={`Đánh giá: ${track.rating}/5`}>
      {STARS.map((n) => {
        const on = n <= track.rating
        return (
          <button
            key={n}
            type="button"
            className={cx('star', on && 'is-on')}
            aria-label={`${n} sao`}
            aria-pressed={on}
            disabled={update.isPending}
            // Clicking the current rating clears it.
            onClick={() =>
              update.mutate({ id: track.id, input: { rating: n === track.rating ? 0 : n } })
            }
          >
            <Star size={15} aria-hidden fill={on ? 'currentColor' : 'none'} />
          </button>
        )
      })}
    </div>
  )
}

export function DownloadLink({
  artifact,
  variant = 'icon',
}: {
  artifact: Artifact
  variant?: 'icon' | 'button'
}) {
  const format = artifactFormat(artifact)
  if (variant === 'button') {
    return (
      <a className="btn" href={artifactUrl(artifact.id, true)} download={artifact.filename}>
        <Download size={16} aria-hidden />
        Tải {format}
      </a>
    )
  }
  return (
    <a
      className="icon-btn"
      href={artifactUrl(artifact.id, true)}
      download={artifact.filename}
      aria-label={`Tải xuống ${format}`}
      title={`Tải xuống ${format}`}
    >
      <Download size={17} aria-hidden />
    </a>
  )
}
