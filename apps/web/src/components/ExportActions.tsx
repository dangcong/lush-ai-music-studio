import {
  AudioLines,
  AudioWaveform,
  FileArchive,
  FileCode,
  FileMusic,
  KeyboardMusic,
  type LucideIcon,
} from 'lucide-react'
import { artifactUrl, trackArchiveUrl } from '../api/client'
import type { Artifact, Track } from '../api/types'
import { formatBytes } from '../lib/format'
import { artifactFormat } from '../lib/track'

interface ExportFormat {
  id: string
  /** Short text on the button. */
  label: string
  /** Full name used in the tooltip. */
  name: string
  ext: string
  /** Artifact kind that carries this file. */
  kind: string
  icon: LucideIcon
  /** Take the label from the file (the master may be WAV or FLAC). */
  labelFromFile?: boolean
}

/**
 * Add a format here; its button enables itself once tracks carry an artifact of that kind.
 * `midi`, `musicxml` and `score_pdf` are planned for Phase 4 and are not in docs/API_CONTRACTS.md
 * yet: until the backend produces them those buttons stay unavailable.
 */
export const EXPORT_FORMATS: ExportFormat[] = [
  { id: 'mp3', label: 'MP3', name: 'MP3', ext: '.mp3', kind: 'audio_preview', icon: AudioLines },
  {
    id: 'wav',
    label: 'WAV',
    name: 'WAV',
    ext: '.wav',
    kind: 'audio_master',
    icon: AudioWaveform,
    labelFromFile: true,
  },
  { id: 'midi', label: 'MIDI', name: 'MIDI', ext: '.mid', kind: 'midi', icon: KeyboardMusic },
  { id: 'musicxml', label: 'XML', name: 'MusicXML', ext: '.musicxml', kind: 'musicxml', icon: FileCode },
  { id: 'pdf', label: 'PDF', name: 'Sheet Music PDF', ext: '.pdf', kind: 'score_pdf', icon: FileMusic },
]

function fileExt(artifact: Artifact, fallback: string): string {
  const dot = artifact.filename.lastIndexOf('.')
  return dot > 0 ? artifact.filename.slice(dot) : fallback
}

export function ExportActions({ track }: { track: Track }) {
  const available: string[] = []

  const buttons = EXPORT_FORMATS.map((f) => {
    const Icon = f.icon
    const artifact = track.artifacts.find((a) => a.kind === f.kind)
    if (artifact) {
      const label = f.labelFromFile ? artifactFormat(artifact) : f.label
      const name = f.labelFromFile ? label : f.name
      available.push(name)
      return (
        <a
          key={f.id}
          className="export-btn"
          href={artifactUrl(artifact.id, true)}
          download={artifact.filename}
          aria-label={`Tải xuống ${name}`}
          title={`Tải xuống ${name} (${fileExt(artifact, f.ext)}) · ${formatBytes(artifact.size_bytes)}`}
        >
          <Icon size={13} aria-hidden />
          {label}
        </a>
      )
    }
    // aria-disabled (not disabled) so the tooltip still shows on hover and focus.
    const title = `${f.name} (${f.ext}) — chưa khả dụng: hệ thống chưa có chức năng tạo file này`
    return (
      <button key={f.id} type="button" className="export-btn" aria-disabled="true" aria-label={title} title={title}>
        <Icon size={13} aria-hidden />
        {f.label}
      </button>
    )
  })

  const zip = available.length ? (
    <a
      className="export-btn"
      href={trackArchiveUrl(track.id)}
      download
      aria-label="Tải tất cả (.zip)"
      title={`Tải tất cả file hiện có (.zip): ${available.join(', ')}`}
    >
      <FileArchive size={13} aria-hidden />
      ZIP
    </a>
  ) : (
    <button
      type="button"
      className="export-btn"
      aria-disabled="true"
      aria-label="Tải tất cả (.zip) — chưa khả dụng: track chưa có file nào"
      title="Tải tất cả (.zip) — chưa khả dụng: track chưa có file nào"
    >
      <FileArchive size={13} aria-hidden />
      ZIP
    </button>
  )

  return (
    <div className="export-actions" role="group" aria-label="Tải xuống">
      {buttons}
      {zip}
    </div>
  )
}
