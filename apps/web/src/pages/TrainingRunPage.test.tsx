import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { Route, Routes } from 'react-router-dom'
import { describe, expect, it } from 'vitest'
import type { TrainingMetric, TrainingRunDetail } from '../api/types'
import { jsonResponse, makeModel, makeTrainingRun, makeUser, mockFetch, renderWithProviders } from '../test/utils'
import { TrainingRunPage } from './TrainingRunPage'

function metric(overrides: Partial<TrainingMetric>): TrainingMetric {
  return {
    sequence: 1,
    global_step: 1,
    epoch: 1,
    train_loss: 1,
    learning_rate: null,
    elapsed_seconds: 10,
    gpu_memory_mb: 9000,
    recorded_at: '2026-01-01T00:01:00.000Z',
    ...overrides,
  }
}

const metrics = [
  metric({ sequence: 1, global_step: 1, epoch: 1, train_loss: 0.9123, elapsed_seconds: 30 }),
  metric({ sequence: 2, global_step: 2, epoch: 1, train_loss: 0.7001, elapsed_seconds: 60 }),
  metric({ sequence: 3, global_step: 3, epoch: 2, train_loss: 0.5512, elapsed_seconds: 95, gpu_memory_mb: 10240 }),
]

function setup(run: TrainingRunDetail, role: 'admin' | 'member' = 'admin') {
  const cancelled: string[] = []
  const fetchMock = mockFetch({
    [`GET /training-runs/${run.id}`]: run,
    'GET /models': [makeModel({ id: 'm-ace', engine: 'acestep', name: 'ACE-Step' })],
    [`POST /jobs/${run.id}/cancel`]: () => {
      cancelled.push(run.id)
      return jsonResponse({ ok: true })
    },
  })
  renderWithProviders(
    <Routes>
      <Route path="/training/:id" element={<TrainingRunPage />} />
    </Routes>,
    { user: makeUser({ role }), route: `/training/${run.id}` },
  )
  return { cancelled, fetchMock, user: userEvent.setup() }
}

/** Text of the <dd> next to the <dt> with this label. */
function value(label: string, within_: HTMLElement = document.body): string | null | undefined {
  return within(within_).getByText(label).nextElementSibling?.textContent
}

const fresh = () => new Date().toISOString()

describe('TrainingRunPage', () => {
  it('running with metrics: stage label, progress, latest numbers and one chart point per reported metric', async () => {
    setup(
      makeTrainingRun({
        status: 'running',
        stage: 'training',
        progress: 0.42,
        node_name: 'render-node-01',
        started_at: '2026-01-01T00:00:30.000Z',
        updated_at: fresh(),
        metrics,
        log: 'Epoch 1/4\nStep 3 loss 0.5512',
      }),
    )

    expect(await screen.findByRole('heading', { name: 'Lofi Style' })).toBeInTheDocument()
    expect(screen.getByText('Đang chạy')).toBeInTheDocument()
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '42')
    expect(value('Giai đoạn')).toBe('Đang train')
    expect(value('Node')).toBe('render-node-01')

    expect(value('Epoch mới nhất')).toBe('2')
    expect(value('Optimizer step mới nhất')).toBe('3')
    expect(value('Train loss mới nhất')).toBe('0.5512')
    expect(value('Thời gian đã train')).toBe('1:35')
    expect(value('GPU memory')).toBe('10240 MB')
    // The trainer does not report these.
    expect(value('Learning rate', screen.getByRole('region', { name: 'Số liệu training' }))).toBe('Chưa có dữ liệu')
    expect(value('Validation loss')).toBe('Không hỗ trợ')

    expect(screen.getByRole('img', { name: /Train loss theo optimizer step: 3 điểm, step 1 đến 3/ })).toBeInTheDocument()
    expect(screen.getAllByTestId('loss-point')).toHaveLength(3)
    expect(screen.getByText('Step 2: loss 0.7001')).toBeInTheDocument()

    expect(screen.getByText(/Step 3 loss 0\.5512/)).toHaveClass('code-block')
    expect(screen.queryByText(/Mất cập nhật/)).not.toBeInTheDocument()
    // Training runs cannot be retried.
    expect(screen.queryByRole('button', { name: /Thử lại/ })).not.toBeInTheDocument()
  })

  it('skips metrics without a step or loss instead of inventing points', async () => {
    setup(
      makeTrainingRun({
        status: 'running',
        stage: 'training',
        updated_at: fresh(),
        metrics: [
          metric({ sequence: 1, global_step: 1, train_loss: 0.9 }),
          metric({ sequence: 2, global_step: 2, train_loss: null }),
          metric({ sequence: 3, global_step: null, train_loss: 0.5 }),
          metric({ sequence: 4, global_step: 4, train_loss: 0.4 }),
        ],
      }),
    )
    await screen.findByRole('heading', { name: 'Lofi Style' })
    expect(screen.getAllByTestId('loss-point')).toHaveLength(2)
  })

  it('without metrics: says there is no data yet and draws no chart', async () => {
    setup(makeTrainingRun({ status: 'running', stage: 'preprocessing', updated_at: fresh(), metrics: [] }))

    await screen.findByRole('heading', { name: 'Lofi Style' })
    expect(value('Giai đoạn')).toBe('Tiền xử lý dataset')
    expect(value('Epoch mới nhất')).toBe('Chưa có dữ liệu')
    expect(value('Train loss mới nhất')).toBe('Chưa có dữ liệu')
    expect(value('GPU memory')).toBe('Chưa có dữ liệu')
    expect(screen.queryByRole('img')).not.toBeInTheDocument()
    expect(screen.queryByTestId('loss-point')).not.toBeInTheDocument()
    expect(screen.getByText('Chưa có dữ liệu', { selector: 'p' })).toBeInTheDocument()
    expect(screen.getByText('Run này chưa có log.')).toBeInTheDocument()
  })

  it('warns "Mất cập nhật từ HH:MM" when a running run has been silent for over 90 seconds', async () => {
    const updatedAt = new Date(Date.now() - 5 * 60_000)
    setup(
      makeTrainingRun({
        status: 'running',
        stage: 'training',
        progress: 0.3,
        message: 'Epoch 2/4',
        updated_at: updatedAt.toISOString(),
        metrics,
      }),
    )
    const hhmm = `${String(updatedAt.getHours()).padStart(2, '0')}:${String(updatedAt.getMinutes()).padStart(2, '0')}`
    expect(await screen.findByText(`Mất cập nhật từ ${hhmm}`)).toBeInTheDocument()
    // The last report is shown as such, not as live progress.
    expect(screen.getByRole('region', { name: 'Tiến độ' })).toHaveClass('is-stale')
    expect(screen.getByText(/lần báo cuối — không còn cập nhật/)).toBeInTheDocument()
    expect(screen.queryByText('Epoch 2/4')).not.toBeInTheDocument()
    expect(document.querySelector('.spin')).toBeNull()
  })

  it('does not warn about lost updates for a finished run', async () => {
    setup(makeTrainingRun({ status: 'failed', error: 'CUDA out of memory', updated_at: '2026-01-01T00:05:00.000Z' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('CUDA out of memory')
    expect(screen.queryByText(/Mất cập nhật/)).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Hủy run' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /Thử lại/ })).not.toBeInTheDocument()
  })

  it('shows the configuration and limits of the run', async () => {
    setup(makeTrainingRun({ status: 'queued' }))
    const config = within(await screen.findByRole('region', { name: 'Cấu hình' }))
    const cell = (label: string) => config.getByRole('rowheader', { name: label }).nextElementSibling?.textContent
    expect(cell('Dataset')).toBe('lofi-set')
    await waitFor(() => expect(cell('Base model')).toBe('ACE-Step'))
    expect(cell('Rank')).toBe('32')
    expect(cell('Alpha')).toBe('64')
    expect(cell('Learning rate')).toBe('0.0001')
    expect(cell('Gradient accumulation')).toBe('4')
    expect(cell('Seed')).toBe('Ngẫu nhiên')
    expect(value('Giới hạn thời gian')).toBe('30 phút')
    expect(value('Lưu checkpoint')).toBe('Mỗi 2 epoch (chỉ theo epoch)')
    expect(value('Dừng sớm theo validation')).toBe('Không hỗ trợ')
  })

  it('completed: summarises the result and links to the Train Style', async () => {
    setup(
      makeTrainingRun({
        status: 'completed',
        progress: 1,
        finished_at: '2026-01-01T00:20:00.000Z',
        updated_at: '2026-01-01T00:20:00.000Z',
        metrics,
        result: {
          lora: { key: 'lofi-style-v1', adapter_sha256: 'abc', bytes: 5 * 1024 * 1024 },
          epochs_completed: 3,
          optimizer_steps: 9,
          stop_reason: 'time_limit',
          checkpoints: [{ epoch: 2, bytes: 1024 * 1024 }],
          preprocess_seconds: 65,
          train_seconds: 600,
          vram_peak_mb: 11000,
          inference_restored: true,
          adapter_loadable: true,
        },
        lora: { id: 'lora-1', key: 'lofi-style-v1', name: 'Lofi Style', status: 'evaluation' },
      }),
    )
    const result = await screen.findByRole('region', { name: 'Kết quả' })
    expect(value('Lý do dừng', result)).toBe('Hết giới hạn thời gian')
    expect(value('Epoch đã chạy', result)).toBe('3')
    expect(value('Optimizer steps', result)).toBe('9')
    expect(value('VRAM đỉnh', result)).toBe('11000 MB')
    expect(value('Engine tạo nhạc', result)).toBe('Đã khôi phục')
    expect(value('Nạp thử adapter', result)).toBe('Nạp được')
    expect(value('Checkpoints', result)).toBe('epoch 2 (1.0 MB)')
    expect(screen.getByRole('link', { name: /Mở “Lofi Style” trong Train Style Library/ })).toHaveAttribute(
      'href',
      '/lora-library',
    )
    expect(screen.queryByRole('button', { name: 'Hủy run' })).not.toBeInTheDocument()
  })

  it('completed with an empty result: every field falls back to "Chưa có dữ liệu"', async () => {
    setup(makeTrainingRun({ status: 'completed', result: { inference_restored: false } }))
    const result = await screen.findByRole('region', { name: 'Kết quả' })
    expect(value('Lý do dừng', result)).toBe('Chưa có dữ liệu')
    expect(value('Nạp thử adapter', result)).toBe('Chưa có dữ liệu')
    expect(value('Engine tạo nhạc', result)).toBe('CHƯA khôi phục được')
    expect(within(result).getByRole('alert')).toHaveTextContent('Engine tạo nhạc chưa được khôi phục')
  })

  it('cancel: explains what is lost and calls POST /jobs/{id}/cancel after confirmation', async () => {
    const { user, cancelled } = setup(
      makeTrainingRun({ status: 'running', stage: 'training', updated_at: fresh(), metrics }),
    )
    await user.click(await screen.findByRole('button', { name: 'Hủy run' }))
    const dialog = within(screen.getByRole('dialog'))
    expect(dialog.getByText(/mất toàn bộ phần đã train sau checkpoint cuối cùng đã lưu/)).toBeInTheDocument()
    expect(cancelled).toHaveLength(0)

    await user.click(dialog.getByRole('button', { name: 'Hủy run' }))
    await waitFor(() => expect(cancelled).toEqual(['run-1']))
  })

  it('keeps non-admins out without requesting the run', () => {
    const { fetchMock } = setup(makeTrainingRun(), 'member')
    expect(screen.getByText(/Chỉ admin mới tạo và xem được training run/)).toBeInTheDocument()
    expect(fetchMock).not.toHaveBeenCalled()
  })
})
