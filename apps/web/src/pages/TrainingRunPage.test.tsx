import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { Route, Routes } from 'react-router-dom'
import { describe, expect, it } from 'vitest'
import type { Checkpoint, EvalRender, TrainingMetric, TrainingRunDetail } from '../api/types'
import { jsonResponse, makeCheckpoint, makeModel, makeTrainingRun, makeUser, mockFetch, renderWithProviders } from '../test/utils'
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

function setup(run: TrainingRunDetail, role: 'admin' | 'member' | 'trainer' = 'admin', extra: Record<string, unknown> = {}) {
  const cancelled: string[] = []
  const fetchMock = mockFetch({
    [`GET /training-runs/${run.id}`]: run,
    'GET /models': [makeModel({ id: 'm-ace', engine: 'acestep', name: 'ACE-Step' })],
    ...extra,
    [`POST /jobs/${run.id}/cancel`]: () => {
      cancelled.push(run.id)
      return jsonResponse({ ok: true })
    },
  })
  renderWithProviders(
    <Routes>
      <Route path="/training/:id" element={<TrainingRunPage />} />
      <Route path="/training" element={<p>training list</p>} />
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
        lora: { id: 'lora-1', key: 'lofi-style-v1', name: 'Lofi Style', status: 'evaluation', base_key: 'lofi-style-v1', chosen_checkpoint_id: null },
      }),
    )
    const result = await screen.findByRole('region', { name: 'Kết quả' })
    expect(value('Lý do dừng', result)).toBe('Hết giới hạn thời gian')
    expect(value('Epoch đã chạy', result)).toBe('3')
    expect(value('Optimizer steps', result)).toBe('9')
    expect(value('VRAM đỉnh', result)).toBe('11000 MB')
    expect(value('Engine tạo nhạc', result)).toBe('Đã khôi phục')
    expect(value('Nạp thử adapter', result)).toBe('Nạp được')
    expect(within(result).queryByText('Checkpoints')).not.toBeInTheDocument()
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
    expect(dialog.getByText(/mất toàn bộ phần đã train\. Hủy tay chưa giữ bản đã lưu/)).toBeInTheDocument()
    expect(cancelled).toHaveLength(0)

    await user.click(dialog.getByRole('button', { name: 'Hủy run' }))
    await waitFor(() => expect(cancelled).toEqual(['run-1']))
  })

  it('keeps non-admins out without requesting the run', () => {
    const { fetchMock } = setup(makeTrainingRun(), 'member')
    expect(screen.getByText(/Chỉ admin và trainer mới xem được training run/)).toBeInTheDocument()
    expect(fetchMock).not.toHaveBeenCalled()
  })

  describe('LoRA Studio snapshot and checkpoints', () => {
    const snapshot = {
      id: 'snap-1',
      catalog_id: 'c1',
      catalog_name: 'Lofi Catalog',
      samples: 12,
      held_out: ['hold-a.mp3', 'hold-b.mp3'],
      keyword: 'lofi_rain',
      test_prompt: 'rainy lofi beat',
      models: [
        { model_key: 'qwen', name: 'Qwen Omni', count: 9 },
        { model_key: 'gemini', name: 'Gemini', count: 3 },
      ],
    }

    it('shows where the LoRA was trained from', async () => {
      setup(makeTrainingRun({ snapshot }), 'trainer')
      const box = within(await screen.findByRole('region', { name: 'LoRA này được train từ' }))
      expect(box.getByRole('heading', { name: 'LoRA này được train từ' })).toBeInTheDocument()
      expect(box.getByRole('link', { name: 'Lofi Catalog' })).toHaveAttribute('href', '/catalogs/c1')
      expect(box.getByText(/12 bài train/)).toBeInTheDocument()
      expect(box.getByText('Giữ lại')).toBeInTheDocument()
      expect(box.getByText(/hold-a\.mp3/)).toBeInTheDocument()
      expect(box.getByText(/hold-b\.mp3/)).toBeInTheDocument()
      expect(box.getByText('lofi_rain')).toBeInTheDocument()
      expect(box.getByText('rainy lofi beat')).toBeInTheDocument()
      expect(box.getByText('Qwen Omni · 9 bài')).toBeInTheDocument()
    })

    it('says "Không có" when the keyword is empty', async () => {
      setup(makeTrainingRun({ snapshot: { ...snapshot, keyword: '' } }))
      const box = within(await screen.findByRole('region', { name: 'LoRA này được train từ' }))
      expect(box.getByText('Không có')).toBeInTheDocument()
    })

    it('opens the song table with training captions and a held-out label', async () => {
      const sample = (o: Record<string, unknown>) => ({
        catalog_track_id: 't', artifact_id: 'a', filename: 'x.mp3', sha256: '', size_bytes: 1, duration_s: 10,
        model_key: 'qwen', caption_seq: 1, caption_source: 'x', mood: null, instruments: null, vocals: '',
        caption: 'c', train_caption: 'cap', bpm: 80, keyscale: 'C major', voice_confirmed: false, ...o,
      })
      const { user } = setup(makeTrainingRun({ snapshot }), 'admin', {
        'GET /dataset-snapshots/snap-1': {
          id: 'snap-1', catalog_id: 'c1', job_id: 'run-1', lora_key: 'k', created_at: '2026-01-01T00:00:00.000Z',
          content: {
            samples: [sample({ filename: 'train-1.mp3', train_caption: 'warm piano loop' })],
            held_out: [sample({ filename: 'hold-a.mp3', train_caption: null, bpm: null, keyscale: null, model_key: null })],
            models: snapshot.models,
          },
        },
      })
      await user.click(await screen.findByRole('button', { name: 'Xem danh sách bài và mô tả' }))
      const table = within(await screen.findByRole('table', { name: 'Danh sách bài và mô tả' }))
      for (const h of ['Bài', 'Model', 'Mô tả khi train', 'Tempo', 'Key']) {
        expect(table.getByRole('columnheader', { name: h })).toBeInTheDocument()
      }
      expect(table.getByText('warm piano loop')).toBeInTheDocument()
      const held = table.getByText('hold-a.mp3').closest('tr') as HTMLElement
      expect(within(held).getByText('Giữ lại')).toBeInTheDocument()
      expect(within(held).getAllByText('—').length).toBeGreaterThanOrEqual(3)
    })

    it('lists saved checkpoints without any path', async () => {
      setup(
        makeTrainingRun({
          checkpoints: [
            makeCheckpoint({ id: 'ck1', epoch: 1, step: 10, bytes: 5 * 1024 * 1024, status: 'kept' }),
            makeCheckpoint({ id: 'ck2', epoch: 2, step: null, bytes: 10 * 1024 * 1024, status: 'kept' }),
          ],
        }),
      )
      const box = within(await screen.findByRole('region', { name: 'Bản lưu' }))
      expect(box.getByText('Epoch 1 · bước 10 · 5 MB')).toBeInTheDocument()
      expect(box.getByText('Epoch 2 · bước — · 10 MB')).toBeInTheDocument()
      expect(box.getByText('Nghe thử và chọn bản sẽ có ở bước sau.')).toBeInTheDocument()
      expect(document.body.textContent).not.toMatch(/[\\/]\w+\.(safetensors|pt)|\/mnt|\/media|[A-Z]:\\/)
    })

    it('does not duplicate checkpoints inside the result summary', async () => {
      setup(
        makeTrainingRun({
          status: 'completed',
          result: { checkpoints: [{ epoch: 2, bytes: 1024 }] } as TrainingRunDetail['result'],
          checkpoints: [makeCheckpoint({ id: 'ck1', epoch: 2, step: 4, bytes: 1024 * 1024, status: 'kept' })],
        }),
      )
      await screen.findByRole('region', { name: 'Bản lưu' })
      expect(screen.queryByText('Checkpoints')).not.toBeInTheDocument()
    })

    it('falls back to result.checkpoints for a legacy run without checkpoint rows', async () => {
      setup(
        makeTrainingRun({
          status: 'completed',
          checkpoints: [],
          result: { checkpoints: [{ epoch: 2, bytes: 1048576 }] } as TrainingRunDetail['result'],
        }),
      )
      const box = within(await screen.findByRole('region', { name: 'Bản lưu' }))
      expect(box.getByText('Epoch 2 · bước — · 1 MB')).toBeInTheDocument()
    })

    it('shows the catalog name as Dataset when the run has no dataset_key', async () => {
      setup(makeTrainingRun({ snapshot, request: { name: 'Lofi Style', base_model_id: 'm-ace', epochs: 4 } }))
      const cfg = within(await screen.findByRole('region', { name: 'Cấu hình' }))
      expect(cfg.getByText('Dataset').nextElementSibling?.textContent).toBe('Lofi Catalog')
    })

    it('a legacy run has no snapshot box and no checkpoint box', async () => {
      setup(makeTrainingRun({ snapshot: null, checkpoints: [] }))
      await screen.findByRole('heading', { name: 'Lofi Style' })
      expect(screen.queryByText('LoRA này được train từ')).not.toBeInTheDocument()
      expect(screen.queryByRole('region', { name: 'Bản lưu' })).not.toBeInTheDocument()
    })

    it('labels the preparing_dataset stage', async () => {
      setup(makeTrainingRun({ status: 'running', stage: 'preparing_dataset', updated_at: new Date().toISOString() }))
      expect(await screen.findAllByText('Chuẩn bị dữ liệu')).not.toHaveLength(0)
    })
  })
})

describe('TrainingRunPage – after-train review', () => {
  const snapshot = {
    id: 'snap-1',
    catalog_id: 'c1',
    catalog_name: 'Lofi Catalog',
    samples: 12,
    held_out: ['rainy_window.mp3', 'coffee_steam.mp3'],
    keyword: 'lushlofi',
    test_prompt: 'lushlofi, rainy night lo-fi',
    models: [],
  }
  const lora = {
    id: 'lora-1', key: 'lofi-v1', base_key: 'lofi-v1', name: 'Lofi Chill v1', status: 'evaluation' as const,
    chosen_checkpoint_id: null as string | null,
  }
  const ck = (o: Partial<Checkpoint>) => makeCheckpoint(o)
  const render = (o: Partial<EvalRender>): EvalRender => ({
    id: 'r', kind: 'prompt', checkpoint_id: null, sample_filename: null, prompt: 'p', artifact_id: 'a', scores: {}, ...o,
  })
  const checkpoints = [
    ck({ id: 'ck1', epoch: 1, step: 4 }),
    ck({ id: 'ck2', epoch: 2, step: 8, style_score: 0.72, train_score: 0.5, train_match: 'a.mp3', quality_score: 7.1 }),
    ck({ id: 'ck3', epoch: 3, step: 12, style_score: 0.6, train_score: 0.97, train_match: 'late_bus_home.mp3', quality_score: 6 }),
  ]
  const renders: EvalRender[] = [
    render({ id: 'rb', kind: 'base', artifact_id: 'art-base' }),
    ...['ck1', 'ck2', 'ck3'].flatMap((id) => [
      render({ id: `${id}-p`, kind: 'prompt', checkpoint_id: id, artifact_id: `art-${id}-p` }),
      render({ id: `${id}-h1`, kind: 'held_out', checkpoint_id: id, sample_filename: 'rainy_window.mp3', artifact_id: `art-${id}-h1` }),
      render({ id: `${id}-h2`, kind: 'held_out', checkpoint_id: id, sample_filename: 'coffee_steam.mp3', artifact_id: `art-${id}-h2` }),
    ]),
  ]
  function reviewRun(o: Partial<TrainingRunDetail> = {}): TrainingRunDetail {
    return makeTrainingRun({
      status: 'completed', progress: 1, finished_at: '2026-01-01T00:20:00.000Z', snapshot, lora, checkpoints, renders,
      eval: { job_id: 'ev1', status: 'completed', progress: 1, stage: null, error: null, scorers: { mert: true, audiobox: true } },
      held_out_tracks: [
        { filename: 'rainy_window.mp3', artifact_id: 'art-real-1' },
        { filename: 'coffee_steam.mp3', artifact_id: 'art-real-2' },
      ],
      metrics: [
        metric({ sequence: 1, global_step: 1, train_loss: 0.9 }),
        metric({ sequence: 2, global_step: 12, train_loss: 0.4 }),
      ],
      ...o,
    })
  }
  const bodyOf = (init: RequestInit | undefined) => JSON.parse(String(init?.body))
  const conflict = (message: string) => jsonResponse({ error: { code: 'conflict', message } }, 409)
  const row = (name: string) => screen.getByText(name).closest('tr') as HTMLElement

  it('shows the LoRA name, the four steps and the retrain link', async () => {
    setup(reviewRun())
    expect(await screen.findByRole('heading', { name: 'Lofi Chill v1' })).toBeInTheDocument()
    const steps = within(screen.getByRole('list', { name: 'Các bước' }))
    expect(steps.getByText(/Tạo nhạc nghe thử/)).toBeInTheDocument()
    expect(steps.getByText('10 bài')).toBeInTheDocument()
    expect(steps.getByText('Chờ bạn')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Train lại với tùy chọn khác' })).toHaveAttribute(
      'href',
      '/catalogs/c1?retrain=snap-1',
    )
    expect(screen.getByRole('region', { name: 'LoRA này được train từ' })).toBeInTheDocument()
  })

  it('selecting epoch 2 and publishing sends checkpoint, style name and the default 0.7 scale', async () => {
    const bodies: unknown[] = []
    const { user } = setup(reviewRun(), 'trainer', {
      'POST /loras/lora-1/publish': (init: RequestInit | undefined) => {
        bodies.push(bodyOf(init))
        return jsonResponse({ job_id: 'f1', style_preset_id: 's1' }, 202)
      },
    })
    await screen.findByText('Epoch 2 · bước 8')
    expect(screen.getByRole('button', { name: /vào Style Preset/ })).toBeDisabled()
    await user.click(screen.getByText('Epoch 2 · bước 8'))
    const panel = within(screen.getByRole('region', { name: 'Đưa vào Style Preset' }))
    expect(panel.getByText('Epoch 2')).toBeInTheDocument()
    expect(panel.getByLabelText('Tên hiện ở Style Preset')).toHaveValue('Lofi Chill v1')
    await user.click(panel.getByRole('button', { name: 'Đưa Epoch 2 vào Style Preset' }))
    await waitFor(() =>
      expect(bodies).toEqual([{ checkpoint_id: 'ck2', style_name: 'Lofi Chill v1', lora_scale: 0.7 }]),
    )
  })

  it('saves a note with PATCH when the field loses focus', async () => {
    const bodies: unknown[] = []
    const { user } = setup(reviewRun(), 'admin', {
      'PATCH /lora-checkpoints/ck1': (init: RequestInit | undefined) => {
        bodies.push(bodyOf(init))
        return jsonResponse(makeCheckpoint({ id: 'ck1', note: 'ấm' }))
      },
    })
    const note = await screen.findByRole('textbox', { name: 'Ghi chú Epoch 1' })
    expect(note).toHaveAttribute('placeholder', 'Ghi chú…')
    await user.type(note, 'ấm')
    expect(bodies).toEqual([])
    await user.tab()
    await waitFor(() => expect(bodies).toEqual([{ note: 'ấm' }]))
  })

  it('"Xóa ngay" with nothing chosen shows the 409 message from the API', async () => {
    const { user } = setup(reviewRun(), 'admin', {
      'POST /loras/lora-1/cleanup': () => conflict('Chọn một bản trước khi xóa các bản còn lại'),
    })
    await user.click(await screen.findByRole('button', { name: 'Xóa ngay' }))
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Xóa ngay' }))
    expect(await screen.findByText('Chọn một bản trước khi xóa các bản còn lại')).toBeInTheDocument()
  })

  it('null scores show "—" and "Chưa cài máy chấm"', async () => {
    setup(reviewRun())
    await screen.findByText('Epoch 1 · bước 4')
    const r = within(row('Epoch 1 · bước 4'))
    expect(r.getAllByText('—')).toHaveLength(3)
    expect(r.getByText('Chưa cài máy chấm')).toBeInTheDocument()
    for (const label of ['Giống style', 'Giống bài train', 'Chất lượng âm thanh']) {
      expect(r.getByText(label)).toBeInTheDocument()
    }
  })

  it('a memorizing epoch shows the warning with the closest training song', async () => {
    setup(reviewRun())
    await screen.findByText('Epoch 3 · bước 12')
    expect(
      within(row('Epoch 3 · bước 12')).getByText('Có dấu hiệu học thuộc: gần giống late_bus_home.mp3'),
    ).toBeInTheDocument()
  })

  it('has a reference row with the no-LoRA render and the real held-out songs, and expands held-out renders', async () => {
    const { user } = setup(reviewRun())
    await screen.findByText('Bài mẫu')
    const ref = within(row('Bài mẫu'))
    expect(ref.getByRole('button', { name: 'Phát Bài mẫu · Không LoRA' })).toBeInTheDocument()
    expect(ref.getByRole('button', { name: 'Phát rainy_window.mp3 (thật)' })).toBeInTheDocument()
    expect(ref.getByRole('button', { name: 'Phát coffee_steam.mp3 (thật)' })).toBeInTheDocument()

    const r = within(row('Epoch 1 · bước 4'))
    expect(r.getByRole('button', { name: 'Phát Epoch 1 · LoRA' })).toBeInTheDocument()
    expect(r.getByRole('button', { name: 'Phát Epoch 1 · LoRA tạo · rainy_window.mp3' })).toBeInTheDocument()
    expect(r.queryByRole('button', { name: 'Phát Epoch 1 · LoRA tạo · coffee_steam.mp3' })).not.toBeInTheDocument()
    await user.click(r.getByRole('button', { name: '+ 1 bài nữa' }))
    expect(r.getByRole('button', { name: 'Phát Epoch 1 · LoRA tạo · coffee_steam.mp3' })).toBeInTheDocument()
  })

  it('marks the chosen epoch and selects it by default', async () => {
    setup(reviewRun({ lora: { ...lora, chosen_checkpoint_id: 'ck2' } }))
    await screen.findByText('Epoch 2 · bước 8')
    expect(within(row('Epoch 2 · bước 8')).getByText('Đang chọn')).toBeInTheDocument()
    expect(within(row('Epoch 1 · bước 4')).queryByText('Đang chọn')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Đưa Epoch 2 vào Style Preset' })).toBeEnabled()
  })

  it('a deleted checkpoint is dimmed and cannot be selected', async () => {
    const { user } = setup(
      reviewRun({ checkpoints: [ck({ id: 'ck1', epoch: 1, step: 4, status: 'deleted' }), checkpoints[1]!] }),
    )
    await screen.findByText('Epoch 1 · bước 4')
    const r = row('Epoch 1 · bước 4')
    expect(within(r).getByText('Đã xóa khỏi máy')).toBeInTheDocument()
    expect(within(r).getByRole('radio')).toBeDisabled()
    await user.click(screen.getByText('Epoch 1 · bước 4'))
    expect(screen.getByRole('button', { name: /vào Style Preset/ })).toBeDisabled()
  })

  it('while the publish job runs: "Đang cập nhật trên máy…" and the button is disabled', async () => {
    setup(
      reviewRun({
        lora: { ...lora, chosen_checkpoint_id: 'ck2' },
        files_job: { job_id: 'f1', status: 'running', op: 'publish', error: null },
      }),
    )
    const panel = within(await screen.findByRole('region', { name: 'Đưa vào Style Preset' }))
    expect(panel.getByText('Đang cập nhật trên máy…')).toBeInTheDocument()
    expect(panel.getByRole('button', { name: 'Đưa Epoch 2 vào Style Preset' })).toBeDisabled()
  })

  it('a failed publish job shows its error and the button can be pressed again', async () => {
    setup(
      reviewRun({
        lora: { ...lora, chosen_checkpoint_id: 'ck2' },
        files_job: { job_id: 'f1', status: 'failed', op: 'publish', error: 'sha không khớp' },
      }),
    )
    const panel = within(await screen.findByRole('region', { name: 'Đưa vào Style Preset' }))
    expect(panel.getByText(/sha không khớp/)).toBeInTheDocument()
    expect(panel.getByRole('button', { name: 'Đưa Epoch 2 vào Style Preset' })).toBeEnabled()
  })

  it('quick try sends the prompt, epoch and scale and links to the queue', async () => {
    const bodies: unknown[] = []
    const { user } = setup(reviewRun(), 'admin', {
      'POST /loras/lora-1/try': (init: RequestInit | undefined) => {
        bodies.push(bodyOf(init))
        return jsonResponse({ id: 'job-9' }, 201)
      },
    })
    await user.click(await screen.findByText('Epoch 2 · bước 8'))
    const panel = within(screen.getByRole('region', { name: 'Thử nhanh với câu lệnh của bạn' }))
    const prompt = panel.getByRole('textbox')
    expect(prompt).toHaveValue('lushlofi, rainy night lo-fi')
    expect(panel.getByText('Epoch 2')).toBeInTheDocument()
    expect(panel.getByText('Độ mạnh 0.7')).toBeInTheDocument()
    await user.clear(prompt)
    await user.type(prompt, 'piano mưa')
    await user.click(panel.getByRole('button', { name: 'Tạo thử' }))
    await waitFor(() => expect(bodies).toEqual([{ checkpoint_id: 'ck2', prompt: 'piano mưa', lora_scale: 0.7 }]))
    expect(await panel.findByRole('link', { name: 'Xem trong Hàng đợi' })).toHaveAttribute('href', '/queue')
  })

  it('"Xóa LoRA" confirms, deletes and goes back to /training', async () => {
    const deleted: string[] = []
    const { user } = setup(reviewRun(), 'admin', {
      'DELETE /loras/lora-1': () => {
        deleted.push('lora-1')
        return new Response(null, { status: 204 })
      },
    })
    await user.click(await screen.findByRole('button', { name: 'Xóa LoRA' }))
    const dialog = within(screen.getByRole('dialog'))
    expect(
      dialog.getByText('Xóa LoRA Lofi Chill v1? Hồ sơ được lưu trữ, các bản lưu trên máy sẽ bị xóa.'),
    ).toBeInTheDocument()
    await user.click(dialog.getByRole('button', { name: 'Xóa LoRA' }))
    await waitFor(() => expect(deleted).toEqual(['lora-1']))
    expect(await screen.findByText('training list')).toBeInTheDocument()
  })

  it('"Xóa LoRA" shows a 409 from the API as returned', async () => {
    const { user } = setup(reviewRun(), 'admin', {
      'DELETE /loras/lora-1': () => conflict('Còn Style dùng LoRA này: Lofi'),
    })
    await user.click(await screen.findByRole('button', { name: 'Xóa LoRA' }))
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Xóa LoRA' }))
    expect(await screen.findByText('Còn Style dùng LoRA này: Lofi')).toBeInTheDocument()
  })

  it('draws an epoch line on the loss chart for each checkpoint', async () => {
    setup(reviewRun({ checkpoints: [checkpoints[0]!, checkpoints[2]!] }))
    await screen.findByText('Epoch 1 · bước 4')
    expect(screen.getAllByTestId('epoch-line')).toHaveLength(2)
  })

  it('a legacy run (no snapshot) keeps the old page: no steps, no review table', async () => {
    setup(reviewRun({ snapshot: null }))
    await screen.findByRole('heading', { name: 'Lofi Style' })
    expect(screen.queryByRole('list', { name: 'Các bước' })).not.toBeInTheDocument()
    expect(screen.queryByText('Bài mẫu')).not.toBeInTheDocument()
    expect(screen.queryByRole('region', { name: 'Đưa vào Style Preset' })).not.toBeInTheDocument()
  })

  it('a run without a LoRA yet keeps the old page', async () => {
    setup(reviewRun({ status: 'running', lora: null, updated_at: fresh() }))
    await screen.findByRole('heading', { name: 'Lofi Style' })
    expect(screen.queryByRole('list', { name: 'Các bước' })).not.toBeInTheDocument()
  })
})
