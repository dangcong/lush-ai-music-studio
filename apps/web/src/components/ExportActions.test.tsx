import { screen, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { artifactUrl, trackArchiveUrl } from '../api/client'
import type { Artifact } from '../api/types'
import { makeTrack, renderWithProviders } from '../test/utils'
import { ExportActions } from './ExportActions'
import { TrackCard } from './TrackCard'

describe('ExportActions', () => {
  it('offers MP3, WAV and ZIP downloads; MIDI, MusicXML and PDF are unavailable', () => {
    renderWithProviders(<ExportActions track={makeTrack()} />)
    const group = screen.getByRole('group', { name: 'Tải xuống' })

    expect(within(group).getByRole('link', { name: 'Tải xuống MP3' })).toHaveAttribute(
      'href',
      artifactUrl('art-preview', true),
    )
    const wav = within(group).getByRole('link', { name: 'Tải xuống WAV' })
    expect(wav).toHaveAttribute('href', artifactUrl('art-master', true))
    expect(wav).toHaveAttribute('title', expect.stringContaining('(.wav)'))

    const zip = within(group).getByRole('link', { name: 'Tải tất cả (.zip)' })
    expect(zip).toHaveAttribute('href', trackArchiveUrl('track-1'))
    expect(zip).toHaveAttribute('title', expect.stringContaining('MP3, WAV'))

    for (const name of [/^MIDI \(\.mid\)/, /^MusicXML \(\.musicxml\)/, /^Sheet Music PDF \(\.pdf\)/]) {
      const btn = within(group).getByRole('button', { name })
      expect(btn).toHaveAttribute('aria-disabled', 'true')
      expect(btn).toHaveAttribute('title', expect.stringContaining('chưa khả dụng'))
    }
    expect(within(group).getAllByRole('link')).toHaveLength(3)
  })

  it('turns a format into a download link once the track has that artifact', () => {
    const pdf = {
      id: 'art-pdf',
      kind: 'score_pdf',
      filename: 'rainy-night.pdf',
      content_type: 'application/pdf',
      size_bytes: 10,
      sha256: 'cc',
    } as unknown as Artifact // kind planned for Phase 4, not in ArtifactKind yet
    const track = makeTrack()
    renderWithProviders(<ExportActions track={{ ...track, artifacts: [...track.artifacts, pdf] }} />)

    const link = screen.getByRole('link', { name: 'Tải xuống Sheet Music PDF' })
    expect(link).toHaveAttribute('href', artifactUrl('art-pdf', true))
    expect(link).toHaveAttribute('download', 'rainy-night.pdf')
    expect(screen.getByRole('link', { name: 'Tải tất cả (.zip)' })).toHaveAttribute(
      'title',
      expect.stringContaining('Sheet Music PDF'),
    )
    expect(screen.getByRole('button', { name: /^MIDI/ })).toHaveAttribute('aria-disabled', 'true')
  })

  it('a track without files cannot be downloaded at all', () => {
    renderWithProviders(<ExportActions track={makeTrack({ artifacts: [] })} />)
    expect(screen.queryByRole('link')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: /^Tải tất cả \(\.zip\) — chưa khả dụng/ })).toHaveAttribute(
      'aria-disabled',
      'true',
    )
  })
})

describe('TrackCard actions', () => {
  it('has no star rating but keeps favorite and the download group', () => {
    renderWithProviders(<TrackCard track={makeTrack()} />)
    expect(screen.queryByRole('group', { name: /Đánh giá/ })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /sao$/ })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Yêu thích' })).toBeInTheDocument()
    expect(screen.getByRole('group', { name: 'Tải xuống' })).toBeInTheDocument()
  })
})
