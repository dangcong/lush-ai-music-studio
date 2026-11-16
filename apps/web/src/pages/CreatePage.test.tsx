import { screen, waitFor } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { makeJob, makeModel, mockFetch, renderWithProviders } from '../test/utils'
import { CreatePage } from './CreatePage'

const baseSummary = { queued: 1, preparing: 0, running: 1, nodes_online: 1 }
const training = {
  job_id: 'run-1',
  title: 'Lofi Style',
  stage: 'training',
  started_at: '2026-01-01T00:00:00.000Z',
  node_name: 'render-node-01',
}

function setup(summary: unknown, jobs: unknown[] = []) {
  const fetchMock = mockFetch({
    'GET /queue/summary': summary,
    'GET /genres': [],
    'GET /presets': [],
    'GET /models': [makeModel()],
    'GET /loras': [],
    'GET /jobs': jobs,
    'GET /tracks': [],
  })
  renderWithProviders(<CreatePage />)
  return fetchMock
}

describe('CreatePage', () => {
  it('shows no training notice when the node is not training', async () => {
    setup({ ...baseSummary, training: null })
    expect(await screen.findByLabelText('Prompt')).toBeInTheDocument()
    expect(screen.queryByText(/đang train Train Style/)).not.toBeInTheDocument()
  })

  it('also works against a summary without the training field', async () => {
    setup(baseSummary, [makeJob({ status: 'queued' })])
    expect(await screen.findByText('Đang chờ máy render nhận job.')).toBeInTheDocument()
    expect(screen.queryByText(/đang train Train Style/)).not.toBeInTheDocument()
  })

  it('says the render node is training and that new jobs wait, without promising a time', async () => {
    setup({ ...baseSummary, training })
    const notice = await screen.findByText(/Render Node đang chạy Train Style “Lofi Style”/)
    expect(notice).toHaveTextContent('Job tạo nhạc mới sẽ nằm chờ trong hàng đợi và chạy khi training kết thúc')
    expect(notice).toHaveTextContent('chưa xác định được thời điểm')
    expect(notice.textContent).not.toMatch(/\d+\s*(phút|giờ|giây)/)
    // The form stays usable: jobs can still be queued.
    expect(await screen.findByRole('button', { name: 'Generate' })).toBeEnabled()
  })

  it('tells a queued job that the node is training', async () => {
    setup({ ...baseSummary, training }, [makeJob({ status: 'queued', message: 'waiting' })])
    expect(
      await screen.findByText('Máy render đang chạy Train Style “Lofi Style”. Job sẽ chạy khi training kết thúc.'),
    ).toBeInTheDocument()
  })

  it('asks for generate jobs only and never renders a training run as a music job', async () => {
    const fetchMock = setup({ ...baseSummary, training: null }, [
      makeJob({ id: 'job-g', title: 'Rainy night', status: 'queued' }),
      makeJob({ id: 'job-t', type: 'train_lora', title: 'Lofi Style run', status: 'running' }),
    ])
    expect(await screen.findByRole('article', { name: 'Rainy night' })).toBeInTheDocument()
    expect(screen.queryByText('Lofi Style run')).not.toBeInTheDocument()
    await waitFor(() => {
      const jobsCall = fetchMock.mock.calls.map((c) => String(c[0])).find((u) => u.includes('/jobs'))
      expect(jobsCall).toContain('type=generate')
    })
  })
})
