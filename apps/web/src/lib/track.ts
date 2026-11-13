import type { Artifact, ArtifactKind, GenerationMode, Track, TrackInput } from '../api/types'
import type { PlayerItem } from '../player/PlayerContext'

export function findArtifact(track: Track, kind: ArtifactKind): Artifact | undefined {
  return track.artifacts.find((a) => a.kind === kind)
}

/** Artifact used for streaming: the light preview when present, otherwise the master. */
export function playableArtifact(track: Track): Artifact | undefined {
  return findArtifact(track, 'audio_preview') ?? findArtifact(track, 'audio_master')
}

/** "WAV", "MP3", ... derived from the artifact's filename or content type. */
export function artifactFormat(artifact: Artifact): string {
  const ext = artifact.filename.includes('.') ? artifact.filename.split('.').pop() : undefined
  if (ext) return ext.toUpperCase()
  const subtype = artifact.content_type.split('/')[1] ?? 'audio'
  return subtype.replace(/^x-/, '').replace('mpeg', 'mp3').toUpperCase()
}

/** How the track was made; tracks from before the modes existed are text-to-music. */
export function trackMode(track: Pick<Track, 'params'>): GenerationMode {
  const mode = track.params?.mode
  return mode === 'reference' || mode === 'cover' ? mode : 'text'
}

/** The Reference Audio / Source Audio the track was generated from, if any. */
export function trackInput(track: Pick<Track, 'params'>, role: 'reference_audio' | 'source_audio'): TrackInput | null {
  const inputs = track.params?.inputs
  if (!inputs || typeof inputs !== 'object') return null
  const input = (inputs as Record<string, unknown>)[role] as Partial<TrackInput> | undefined
  return input && typeof input.artifact_id === 'string' ? (input as TrackInput) : null
}

export function toPlayerItem(track: Track): PlayerItem | null {
  const artifact = playableArtifact(track)
  if (!artifact) return null
  const subtitle = [track.genre_name, track.model_name].filter(Boolean).join(' · ')
  return { artifactId: artifact.id, title: track.title, subtitle, trackId: track.id }
}
