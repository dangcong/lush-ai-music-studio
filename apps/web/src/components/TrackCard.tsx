import { Link } from 'react-router-dom'
import type { Track } from '../api/types'
import { cx } from '../lib/cx'
import { formatDate, formatDuration } from '../lib/format'
import { toPlayerItem } from '../lib/track'
import { usePlayer } from '../player/PlayerContext'
import { ExportActions } from './ExportActions'
import { FavoriteButton, PlayButton } from './TrackControls'

export function TrackCard({ track, showOwner }: { track: Track; showOwner?: boolean }) {
  const player = usePlayer()
  const item = toPlayerItem(track)
  const active = item !== null && player.current?.artifactId === item.artifactId

  return (
    <article className={cx('track-card', active && 'is-active')} aria-label={track.title}>
      <PlayButton track={track} />
      <div className="track-main">
        <Link to={`/tracks/${track.id}`} className="track-title">
          {track.title}
        </Link>
        <p className="track-meta">
          {track.genre_name && <span className="tag">{track.genre_name}</span>}
          {track.model_name && <span>{track.model_name}</span>}
          <span>{formatDuration(track.duration)}</span>
          <span>{formatDate(track.created_at)}</span>
          {showOwner && track.owner_name && <span>{track.owner_name}</span>}
        </p>
      </div>
      <div className="track-actions">
        <ExportActions track={track} />
        <FavoriteButton track={track} />
      </div>
    </article>
  )
}
