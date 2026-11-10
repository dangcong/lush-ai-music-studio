import type { Artifact, ArtifactKind, Track } from '../api/types'
import type { PlayerItem } from '../player/PlayerContext'

export function findArtifact(track: Track, kind: ArtifactKind): Artifact | undefined {
  return track.artifacts.find((a) => a.kind === kind)
}

/** Artifact used for streaming: the light preview when present, otherwise the master. */
export function playableArtifact(track: Track): Artifact | undefined {
  return findArtifact(track, 'audio_preview') ?? findArtifact(track, 'audio_master')
}

/** Artifact offered by a single "download" button: the master, otherwise the preview. */
export function downloadableArtifact(track: Track): Artifact | undefined {
  return findArtifact(track, 'audio_master') ?? findArtifact(track, 'audio_preview')
}

/** "WAV", "MP3", ... derived from the artifact's filename or content type. */
export function artifactFormat(artifact: Artifact): string {
  const ext = artifact.filename.includes('.') ? artifact.filename.split('.').pop() : undefined
  if (ext) return ext.toUpperCase()
  const subtype = artifact.content_type.split('/')[1] ?? 'audio'
  return subtype.replace(/^x-/, '').replace('mpeg', 'mp3').toUpperCase()
}

export function toPlayerItem(track: Track): PlayerItem | null {
  const artifact = playableArtifact(track)
  if (!artifact) return null
  const subtitle = [track.genre_name, track.model_name].filter(Boolean).join(' · ')
  return { artifactId: artifact.id, title: track.title, subtitle, trackId: track.id }
}
