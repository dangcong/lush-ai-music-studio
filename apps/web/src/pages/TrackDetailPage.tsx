import { useState } from 'react'
import type { FormEvent, ReactNode } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { ArrowLeft, Check, FileMusic, Pencil, RefreshCw, Shuffle, Trash2, WandSparkles, X } from 'lucide-react'
import { Link, useNavigate, useParams } from 'react-router-dom'
import { api } from '../api/api'
import { queryKeys, useLoras, useTrack } from '../api/hooks'
import { useUpdateTrack } from '../api/mutations'
import type { GenerationRequest, Track } from '../api/types'
import { useCurrentUser } from '../auth/AuthContext'
import { ConfirmDialog } from '../components/Modal'
import { useToast } from '../components/Toast'
import { DownloadLink, FavoriteButton, PlayButton, StarRating } from '../components/TrackControls'
import { Waveform } from '../components/Waveform'
import { ErrorState, LoadingState, Spinner } from '../components/ui'
import { formatBytes, formatDateTime, formatDuration } from '../lib/format'
import { findArtifact, toPlayerItem } from '../lib/track'
import { usePlayer } from '../player/PlayerContext'
import type { CreateLocationState } from './CreatePage'

function numberParam(params: Record<string, unknown>, key: string): number | null {
  const value = params[key]
  return typeof value === 'number' ? value : null
}

/** Best-effort settings when the original job request cannot be loaded. */
function requestFromTrack(track: Track): Partial<GenerationRequest> {
  return {
    prompt: track.prompt,
    title: track.title,
    genre_id: track.genre_id,
    subgenre_id: track.subgenre_id,
    preset_id: track.preset_id,
    model_id: track.model_id,
    lora_id: track.lora_id,
    lora_scale: track.lora_scale,
    duration: track.duration ?? undefined,
    bpm: track.bpm,
    key: track.key,
    time_signature: track.time_signature,
    seed: track.seed,
    params: {
      inference_steps: numberParam(track.params, 'inference_steps'),
      guidance_scale: numberParam(track.params, 'guidance_scale'),
    },
  }
}

function MetaItem({ label, children, wide }: { label: string; children: ReactNode; wide?: boolean }) {
  return (
    <div className={wide ? 'meta-item wide' : 'meta-item'}>
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  )
}

function TitleEditor({ track }: { track: Track }) {
  const update = useUpdateTrack()
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState(track.title)

  if (!editing) {
    return (
      <div className="title-row">
        <h1>{track.title}</h1>
        <button
          type="button"
          className="icon-btn"
          aria-label="Đổi tiêu đề"
          title="Đổi tiêu đề"
          onClick={() => {
            setDraft(track.title)
            setEditing(true)
          }}
        >
          <Pencil size={16} aria-hidden />
        </button>
      </div>
    )
  }

  const save = (e: FormEvent) => {
    e.preventDefault()
    const title = draft.trim()
    if (!title || title === track.title) {
      setEditing(false)
      return
    }
    update.mutate({ id: track.id, input: { title } }, { onSuccess: () => setEditing(false) })
  }

  return (
    <form className="title-row" onSubmit={save}>
      <input
        type="text"
        className="title-input"
        aria-label="Tiêu đề"
        autoFocus
        maxLength={200}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Escape') setEditing(false)
        }}
      />
      <button type="submit" className="icon-btn" aria-label="Lưu tiêu đề" disabled={update.isPending}>
        <Check size={16} aria-hidden />
      </button>
      <button type="button" className="icon-btn" aria-label="Hủy đổi tiêu đề" onClick={() => setEditing(false)}>
        <X size={16} aria-hidden />
      </button>
    </form>
  )
}

function TrackDetail({ track }: { track: Track }) {
  const user = useCurrentUser()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const toast = useToast()
  const player = usePlayer()
  const lorasQuery = useLoras()
  const [confirmDelete, setConfirmDelete] = useState(false)
  const [preparing, setPreparing] = useState(false)

  const item = toPlayerItem(track)
  const master = findArtifact(track, 'audio_master')
  const preview = findArtifact(track, 'audio_preview')
  const canDelete = user.role === 'admin' || user.id === track.owner_id
  const loraName = track.lora_id
    ? (lorasQuery.data?.find((l) => l.id === track.lora_id)?.name ?? track.lora_id)
    : null
  const hasParams = Object.keys(track.params ?? {}).length > 0

  const remove = useMutation({
    mutationFn: () => api.tracks.remove(track.id),
    onSuccess: () => {
      if (player.current?.trackId === track.id) player.stop()
      queryClient.removeQueries({ queryKey: queryKeys.track(track.id) })
      void queryClient.invalidateQueries({ queryKey: ['tracks'] })
      toast.success('Đã xóa track.')
      navigate('/library', { replace: true })
    },
    onError: (error) => {
      setConfirmDelete(false)
      toast.error(error)
    },
  })

  const recreate = async () => {
    setPreparing(true)
    let prefill = requestFromTrack(track)
    if (track.job_id) {
      try {
        // The job keeps the exact body that was sent, including instruments and moods.
        const job = await queryClient.fetchQuery({
          queryKey: queryKeys.job(track.job_id),
          queryFn: () => api.jobs.get(track.job_id as string),
        })
        if (job.request?.prompt) prefill = job.request
      } catch {
        // Job not visible to this user (or gone): fall back to the track's own metadata.
      }
    }
    const state: CreateLocationState = { prefill }
    navigate('/create', { state })
  }

  return (
    <div className="page track-detail">
      <Link to="/library" className="back-link">
        <ArrowLeft size={16} aria-hidden />
        Thư viện
      </Link>

      <header className="detail-head">
        <PlayButton track={track} large />
        <div className="detail-head-main">
          <TitleEditor track={track} />
          <p className="track-meta">
            {track.genre_name && <span className="tag">{track.genre_name}</span>}
            {track.model_name && <span>{track.model_name}</span>}
            <span>{formatDuration(track.duration)}</span>
            {track.owner_name && <span>{track.owner_name}</span>}
            <span>{formatDateTime(track.created_at)}</span>
          </p>
        </div>
        <div className="detail-head-actions">
          <StarRating track={track} />
          <FavoriteButton track={track} />
        </div>
      </header>

      <section className="panel" aria-label="Waveform">
        {item ? (
          <Waveform item={item} />
        ) : (
          <p className="muted">Track này chưa có file audio.</p>
        )}
      </section>

      <div className="action-bar">
        {master && <DownloadLink artifact={master} variant="button" />}
        {preview && <DownloadLink artifact={preview} variant="button" />}
        <button type="button" className="btn" disabled={preparing} onClick={() => void recreate()}>
          {preparing ? <Spinner size={16} /> : <RefreshCw size={16} aria-hidden />}
          Tạo lại với thiết lập này
        </button>
        <span className="action-sep" aria-hidden />
        {/* No backend yet: shown disabled on purpose, never simulated. */}
        <button type="button" className="btn" disabled>
          <Shuffle size={16} aria-hidden />
          Remix <span className="soon">Sắp có</span>
        </button>
        <button type="button" className="btn" disabled>
          <WandSparkles size={16} aria-hidden />
          Extend <span className="soon">Sắp có</span>
        </button>
        <button type="button" className="btn" disabled>
          <FileMusic size={16} aria-hidden />
          Export MIDI <span className="soon">Sắp có</span>
        </button>
        {canDelete && (
          <button type="button" className="btn btn-danger-ghost push-right" onClick={() => setConfirmDelete(true)}>
            <Trash2 size={16} aria-hidden />
            Xóa
          </button>
        )}
      </div>

      <section className="panel" aria-labelledby="meta-heading">
        <h2 id="meta-heading" className="section-title">
          Thông tin
        </h2>
        <dl className="meta-grid">
          <MetaItem label="Prompt gốc" wide>
            <p className="prompt-text">{track.prompt || '—'}</p>
          </MetaItem>
          <MetaItem label="Prompt cuối (gửi cho model)" wide>
            <p className="prompt-text">{track.prompt_final || '—'}</p>
          </MetaItem>
          <MetaItem label="Genre">{track.genre_name ?? '—'}</MetaItem>
          <MetaItem label="Model">{track.model_name ?? '—'}</MetaItem>
          <MetaItem label="LoRA">
            {loraName ? `${loraName}${track.lora_scale !== null ? ` · scale ${track.lora_scale}` : ''}` : '—'}
          </MetaItem>
          <MetaItem label="Seed">
            <span className="mono">{track.seed ?? '—'}</span>
          </MetaItem>
          <MetaItem label="BPM">{track.bpm ?? '—'}</MetaItem>
          <MetaItem label="Key">{track.key ?? '—'}</MetaItem>
          <MetaItem label="Time Signature">{track.time_signature ?? '—'}</MetaItem>
          <MetaItem label="Duration">
            {track.duration !== null ? `${formatDuration(track.duration)} (${track.duration}s)` : '—'}
          </MetaItem>
          <MetaItem label="Phiên bản">{track.version}</MetaItem>
          <MetaItem label="Người tạo">{track.owner_name ?? '—'}</MetaItem>
          <MetaItem label="File" wide>
            {track.artifacts.length === 0 ? (
              '—'
            ) : (
              <ul className="file-list">
                {track.artifacts.map((a) => (
                  <li key={a.id}>
                    <span className="mono">{a.filename}</span>
                    <span className="muted">
                      {a.kind} · {a.content_type} · {formatBytes(a.size_bytes)}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </MetaItem>
          <MetaItem label="Params" wide>
            {hasParams ? <pre className="code-block">{JSON.stringify(track.params, null, 2)}</pre> : '—'}
          </MetaItem>
        </dl>
        <p className="field-hint">
          BPM, Key và Time Signature là hướng dẫn đã gửi cho model, không phải giá trị đo từ file audio.
        </p>
      </section>

      {confirmDelete && (
        <ConfirmDialog
          title="Xóa track?"
          message={
            <>
              Track <strong>{track.title}</strong> sẽ bị gỡ khỏi thư viện.
            </>
          }
          confirmLabel="Xóa track"
          danger
          busy={remove.isPending}
          onConfirm={() => remove.mutate()}
          onClose={() => setConfirmDelete(false)}
        />
      )}
    </div>
  )
}

export function TrackDetailPage() {
  const { id } = useParams()
  const trackQuery = useTrack(id)

  if (trackQuery.isPending) return <LoadingState />
  if (trackQuery.isError) {
    return (
      <div className="page">
        <ErrorState error={trackQuery.error} onRetry={() => void trackQuery.refetch()} />
      </div>
    )
  }
  return <TrackDetail key={trackQuery.data.id} track={trackQuery.data} />
}
