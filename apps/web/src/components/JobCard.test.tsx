import { screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import type { Job } from '../api/types'
import { makeJob, makeTrack, renderWithProviders } from '../test/utils'
import { JobCard } from './JobCard'

function renderCard(job: Job, extra: Partial<Parameters<typeof JobCard>[0]> = {}) {
  const onCancel = vi.fn()
  const onRetry = vi.fn()
  renderWithProviders(<JobCard job={job} onCancel={onCancel} onRetry={onRetry} {...extra} />)
  return { onCancel, onRetry, user: userEvent.setup() }
}

describe('JobCard', () => {
  it('queued: shows the waiting state and can be cancelled', async () => {
    const job = makeJob({ status: 'queued' })
    const { onCancel, user } = renderCard(job)

    expect(screen.getByText('Đang chờ')).toBeInTheDocument()
    expect(screen.queryByRole('progressbar')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Thử lại' })).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Hủy' }))
    expect(onCancel).toHaveBeenCalledWith(job)
  })

  it('preparing: shows the stage reported by the API', () => {
    renderCard(makeJob({ status: 'preparing', progress: 0, stage: 'preparing', message: 'loading model' }))
    expect(screen.getByText('Chuẩn bị')).toBeInTheDocument()
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '0')
    expect(screen.getByText('loading model')).toBeInTheDocument()
  })

  it('running: renders the real progress, stage and message', () => {
    renderCard(
      makeJob({
        status: 'running',
        progress: 0.42,
        stage: 'running',
        message: 'diffusion 12/27',
        node_name: 'render-node-01',
      }),
    )
    expect(screen.getByText('Đang chạy')).toBeInTheDocument()
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '42')
    expect(screen.getByText('42%')).toBeInTheDocument()
    expect(screen.getByText('diffusion 12/27')).toBeInTheDocument()
    expect(screen.getByText(/render-node-01/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Hủy' })).toBeEnabled()
  })

  it('running with cancel_requested: shows "Đang hủy…" instead of the cancel button', () => {
    renderCard(makeJob({ status: 'running', progress: 0.5, cancel_requested: true }))
    expect(screen.getByText('Đang hủy…')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Hủy' })).not.toBeInTheDocument()
  })

  it('completed: renders a playable music card for the resulting track', async () => {
    const track = makeTrack({ title: 'Rainy night' })
    const { user } = renderCard(makeJob({ status: 'completed', track_id: track.id }), { track })

    expect(screen.getByRole('link', { name: 'Rainy night' })).toHaveAttribute('href', '/tracks/track-1')
    await user.click(screen.getByRole('button', { name: 'Phát Rainy night' }))
    // The preview artifact is preferred for streaming.
    expect(screen.getByTestId('player-audio')).toHaveAttribute('src', '/api/v1/artifacts/art-preview/content')
    expect(screen.getByRole('link', { name: 'Tải xuống WAV' })).toHaveAttribute(
      'href',
      '/api/v1/artifacts/art-master/content?download=true',
    )
    expect(screen.queryByRole('button', { name: 'Hủy' })).not.toBeInTheDocument()
  })

  it('completed while the track is still loading: shows a loading note', () => {
    renderCard(makeJob({ status: 'completed', track_id: 'track-1' }), { trackLoading: true })
    expect(screen.getByText(/Đang tải kết quả/)).toBeInTheDocument()
  })

  it('failed: shows the error and offers retry', async () => {
    const job = makeJob({ status: 'failed', error: 'CUDA out of memory' })
    const { onRetry, user } = renderCard(job)

    expect(screen.getByRole('alert')).toHaveTextContent('CUDA out of memory')
    expect(screen.queryByRole('button', { name: 'Hủy' })).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Thử lại' }))
    expect(onRetry).toHaveBeenCalledWith(job)
  })

  it('cancelled: shows the cancelled state and offers retry', () => {
    renderCard(makeJob({ status: 'cancelled' }))
    expect(screen.getByText('Đã hủy')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Thử lại' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Hủy' })).not.toBeInTheDocument()
  })

  it('names the mode of a reference / cover job, and tags the resulting track', () => {
    const { unmount } = renderWithProviders(
      <JobCard
        job={makeJob({ status: 'queued', request: { prompt: 'Strings', mode: 'cover' } })}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />,
    )
    expect(screen.getByText(/Cover \/ Remix/)).toBeInTheDocument()
    unmount()

    const track = makeTrack({ params: { mode: 'reference' } })
    renderCard(makeJob({ status: 'completed', track_id: track.id }), { track })
    expect(screen.getByText('Reference')).toBeInTheDocument()
  })
})
