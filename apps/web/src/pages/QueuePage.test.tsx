import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { jsonResponse, makeJob, makeUser, mockFetch, renderWithProviders } from '../test/utils'
import { QueuePage } from './QueuePage'

const SUMMARY = { queued: 1, preparing: 0, running: 0, training: null }

function setup(role: 'admin' | 'member' | 'trainer') {
  const cancelled: string[] = []
  const listUrls: string[] = []
  const jobs = [
    makeJob({ id: 'gen-1', title: 'Rainy night', status: 'queued' }),
    makeJob({
      id: 'cap-1',
      type: 'caption_tracks',
      title: 'Mô tả 3 bài',
      status: 'running',
      progress: 0.4,
      owner_name: 'Carol',
    }),
    makeJob({ id: 'ana-1', type: 'analyze_track', title: 'song.mp3', status: 'queued' }),
  ]
  mockFetch({
    'GET /jobs': (_init: RequestInit | undefined, url: URL) => {
      listUrls.push(url.search)
      return jsonResponse(jobs)
    },
    'GET /queue': SUMMARY,
    'POST /jobs/cap-1/cancel': () => {
      cancelled.push('cap-1')
      return jsonResponse(makeJob({ id: 'cap-1', type: 'caption_tracks', cancel_requested: true }))
    },
  })
  renderWithProviders(<QueuePage />, { user: makeUser({ role }) })
  return { cancelled, listUrls, user: userEvent.setup() }
}

describe('QueuePage catalog jobs', () => {
  it('lists caption and analyze jobs for a trainer and cancels one', async () => {
    const { cancelled, listUrls, user } = setup('trainer')
    expect(await screen.findByText('Mô tả 3 bài')).toBeInTheDocument()
    expect(screen.getByText('song.mp3')).toBeInTheDocument()
    expect(screen.getByText('Tạo mô tả')).toBeInTheDocument()
    expect(screen.getByText('Đo nhạc')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Xem log Mô tả 3 bài' })).toBeInTheDocument()
    expect(decodeURIComponent(listUrls[0] ?? '')).toContain('type=generate,analyze_track,caption_tracks')

    await user.click(screen.getByRole('button', { name: 'Hủy Mô tả 3 bài' }))
    await waitFor(() => expect(cancelled).toEqual(['cap-1']))
  })

  it('does not show catalog jobs to a member', async () => {
    const { listUrls } = setup('member')
    expect(await screen.findByText('Rainy night')).toBeInTheDocument()
    expect(screen.queryByText('Mô tả 3 bài')).not.toBeInTheDocument()
    expect(screen.queryByText('song.mp3')).not.toBeInTheDocument()
    expect(listUrls[0]).not.toContain('caption_tracks')
  })
})
