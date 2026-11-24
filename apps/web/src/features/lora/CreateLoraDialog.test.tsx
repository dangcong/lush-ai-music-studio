import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { Route, Routes } from 'react-router-dom'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { BuildPlan, BuildTrack, LoraBuildRequest } from '../../api/types'
import { jsonResponse, makeCatalogTrack, makeModel, mockFetch, renderWithProviders } from '../../test/utils'
import { CreateLoraDialog } from './CreateLoraDialog'

const ACE = makeModel({ id: 'ace-1', engine: 'acestep', key: 'ace', name: 'ACE-Step' })

function bt(id: string, over: Partial<BuildTrack> = {}): BuildTrack {
  return { id, filename: `${id}.mp3`, duration_s: 120, model_key: 'moss', vocals: 'Instrumental', caption: 'x', ...over }
}

function plan(over: Partial<BuildPlan> = {}): BuildPlan {
  return {
    ready: true,
    errors: [],
    warnings: [],
    train: [bt('t1'), bt('t2')],
    held_out: [bt('h1')],
    missing_caption: [],
    voice_unconfirmed: [],
    no_tempo_key: [],
    train_seconds: 240,
    unknown_duration: 0,
    models: [{ model_key: 'moss', name: 'MOSS', count: 3 }],
    steps_per_epoch: 1,
    total_steps: 4,
    checkpoint_epochs: [1, 2, 3, 4],
    eval_renders: 3,
    name_taken: false,
    ...over,
  }
}

function setup({
  previewPlan = plan(),
  previewHandler = undefined as undefined | ((body: LoraBuildRequest) => Response | Promise<Response>),
  create = (() => jsonResponse({ job_id: 'j1', snapshot_id: 's1', lora_key: 'k' }, 201)) as () => Response,
  onDeselect = vi.fn(),
  tracksHandler = (() => jsonResponse([])) as () => Response,
} = {}) {
  const previews: LoraBuildRequest[] = []
  const posts: LoraBuildRequest[] = []
  const caption: unknown[] = []
  mockFetch({
    'GET /models': [ACE],
    'GET /training-runs': [],
    'GET /catalogs/c1/tracks': () => tracksHandler(),
    'POST /catalogs/c1/lora-preview': (init: RequestInit | undefined) => {
      const b = JSON.parse(init?.body as string)
      previews.push(b)
      return previewHandler ? previewHandler(b) : jsonResponse(previewPlan)
    },
    'POST /catalogs/c1/loras': (init: RequestInit | undefined) => {
      posts.push(JSON.parse(init?.body as string))
      return create()
    },
    'POST /catalogs/c1/caption-jobs': (init: RequestInit | undefined) => {
      caption.push(JSON.parse(init?.body as string))
      return jsonResponse({ job_id: 'cj', queued: [], skipped: [] }, 201)
    },
  })
  const onClose = vi.fn()
  renderWithProviders(
    <Routes>
      <Route
        path="/"
        element={
          <CreateLoraDialog
            catalogId="c1"
            catalogName="Lo-fi chill"
            genreName="Lofi"
            trackIds={['t1', 't2', 'h1']}
            onClose={onClose}
            onDeselect={onDeselect}
            captionModelKey="moss"
          />
        }
      />
      <Route path="/training/:id" element={<p>run page</p>} />
    </Routes>,
  )
  return { previews, posts, caption, onDeselect, user: userEvent.setup() }
}

afterEach(() => vi.restoreAllMocks())

const last = <T,>(a: T[]) => a[a.length - 1] as T

describe('CreateLoraDialog', () => {
  it('blocks on an unconfirmed voice track until the user confirms', async () => {
    const { previews, user } = setup({
      previewPlan: plan({ ready: false, voice_unconfirmed: [bt('t2', { vocals: 'Female vocals' })] }),
    })
    const start = await screen.findByRole('button', { name: 'Bắt đầu train' })
    await screen.findByText('t2.mp3')
    expect(start).toBeDisabled()
    await user.click(screen.getByRole('button', { name: 'Xác nhận không có giọng' }))
    await waitFor(() => expect(last(previews).voice_confirmed).toEqual(['t2']))
  })

  it('drops a track without caption and tells the page to deselect it', async () => {
    const { previews, onDeselect, user } = setup({
      previewPlan: plan({ ready: false, missing_caption: [bt('t2', { caption: '' })] }),
    })
    await user.click(await screen.findByRole('button', { name: 'Bỏ bài này' }))
    expect(onDeselect).toHaveBeenCalledWith(['t2'])
    await waitFor(() => expect(last(previews).track_ids).toEqual(['t1', 'h1']))
  })

  it('creates captions for a missing-caption track with the picked model', async () => {
    const { caption, user } = setup({
      previewPlan: plan({ ready: false, missing_caption: [bt('t2', { caption: '' })] }),
    })
    await user.click(await screen.findByRole('button', { name: 'Tạo mô tả' }))
    await waitFor(() => expect(caption).toEqual([{ model_key: 'moss', track_ids: ['t2'] }]))
  })

  it('sets epochs from the quick preset', async () => {
    const { previews, user } = setup()
    await user.click(await screen.findByRole('button', { name: 'Thử nhanh · 1 epoch' }))
    await waitFor(() => expect(last(previews).epochs).toBe(1))
  })

  it('lets the user set the held-out count to 0', async () => {
    const { previews, user } = setup()
    const field = await screen.findByLabelText('Số bài giữ lại')
    expect(screen.getByText('Từ 0 đến 5. Đặt 0 được.')).toBeInTheDocument()
    await user.clear(field)
    await user.type(field, '0')
    await waitFor(() => expect(last(previews).holdout_count).toBe(0))
  })

  it('shows the real reason when the preview request fails', async () => {
    setup({
      previewHandler: () =>
        jsonResponse({ error: { code: 'validation', message: 'Catalog không còn bài nào' } }, 422),
    })
    expect(await screen.findByText(/Catalog không còn bài nào/)).toBeInTheDocument()
  })

  it('does not promise that a cancelled run keeps its checkpoints', async () => {
    setup()
    expect(
      await screen.findByText(
        /Có thể hủy giữa chừng\. Hết giới hạn thời gian thì vẫn giữ các bản đã lưu; hủy tay thì chưa giữ \(sẽ có ở bước sau\)\./,
      ),
    ).toBeInTheDocument()
  })

  it('creates the LoRA and opens the training run', async () => {
    const { posts, user } = setup()
    await user.type(await screen.findByLabelText(/Tên LoRA/), 'Lofi v1')
    const start = screen.getByRole('button', { name: 'Bắt đầu train' })
    await waitFor(() => expect(start).toBeEnabled())
    await user.click(start)
    expect(await screen.findByText('run page')).toBeInTheDocument()
    expect(posts).toHaveLength(1)
    expect(posts[0]?.name).toBe('Lofi v1')
  })

  it('shows a 409 from create inside the dialog', async () => {
    const { user } = setup({
      create: () =>
        jsonResponse({ error: { code: 'conflict', message: 'Đang có một training run khác chưa kết thúc' } }, 409),
    })
    await user.type(await screen.findByLabelText(/Tên LoRA/), 'Lofi v1')
    const start = screen.getByRole('button', { name: 'Bắt đầu train' })
    await waitFor(() => expect(start).toBeEnabled())
    await user.click(start)
    expect(await screen.findByText('Đang có một training run khác chưa kết thúc')).toBeInTheDocument()
    expect(screen.queryByText('run page')).not.toBeInTheDocument()
  })

  it('shows the advanced defaults', async () => {
    const { user } = setup()
    await user.click(await screen.findByText('Mặc định · Nâng cao'))
    expect(screen.getByLabelText('Rank')).toHaveValue(32)
    expect(screen.getByLabelText('Alpha')).toHaveValue(64)
    expect(screen.getByLabelText('Warmup steps')).toHaveValue(2)
  })

  it('renders the plan numbers', async () => {
    setup({ previewPlan: plan({ unknown_duration: 1 }) })
    expect(await screen.findByText(/train · 1 nghe thử/)).toBeInTheDocument()
    expect(screen.getByText(/1 bài chưa rõ thời lượng/)).toBeInTheDocument()
    expect(screen.getByText(/1 bước\/epoch × 4 epoch = 4 bước/)).toBeInTheDocument()
    expect(screen.getByText(/2 bài LoRA \+ 1 bài gốc/)).toBeInTheDocument()
    expect(screen.getByText('h1.mp3')).toBeInTheDocument()
    expect(within(screen.getByRole('dialog')).getByText('MOSS · 3 bài')).toBeInTheDocument()
  })

  it('shows the mockup title and hints', async () => {
    setup()
    expect(await screen.findByRole('heading', { name: 'Tạo LoRA · từ catalog Lo-fi chill' })).toBeInTheDocument()
    expect(screen.getByText('Quá thì dừng, giữ bản đã lưu.')).toBeInTheDocument()
    expect(screen.getByText(/website tạm không tạo nhạc được trên máy rtx6000/)).toBeInTheDocument()
  })

  it('renders plan errors and keeps Start disabled', async () => {
    setup({ previewPlan: plan({ ready: false, errors: ['Tên LoRA đã tồn tại'] }) })
    expect(await screen.findByText('Tên LoRA đã tồn tại')).toBeInTheDocument()
    expect(screen.getByText('Kiểm tra trước khi train')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Bắt đầu train' })).toBeDisabled()
  })

  it('disables Start while the preview for new input is still loading', async () => {
    let release: () => void = () => {}
    const gate = new Promise<void>((r) => (release = r))
    let calls = 0
    const { user } = setup({
      previewHandler: async () => {
        calls += 1
        if (calls > 1) await gate
        return jsonResponse(plan())
      },
    })
    await screen.findByText(/train · 1 nghe thử/)
    const start = screen.getByRole('button', { name: 'Bắt đầu train' })
    await user.type(screen.getByLabelText(/Tên LoRA/), 'A')
    await waitFor(() => expect(calls).toBeGreaterThan(1))
    expect(start).toBeDisabled()
    release()
    await waitFor(() => expect(start).toBeEnabled())
  })

  it('counts selected tracks without the excluded one', async () => {
    const { user } = setup({ previewPlan: plan({ ready: false, missing_caption: [bt('t2', { caption: '' })] }) })
    const count = async () => (await screen.findByText('bài đã chọn')).previousElementSibling?.textContent
    expect(await count()).toBe('3')
    await user.click(await screen.findByRole('button', { name: 'Bỏ bài này' }))
    await waitFor(async () => expect(await count()).toBe('2'))
  })

  it('shows the caption job as running, then re-enables the button when it ends without a caption', async () => {
    let requested = false
    let fetches = 0
    const { user } = setup({
      previewPlan: plan({ ready: false, missing_caption: [bt('t2', { caption: '' })] }),
      tracksHandler: () => {
        fetches += 1
        return jsonResponse([
          makeCatalogTrack({ id: 't2', captioning_model_key: requested && fetches < 3 ? 'moss' : null }),
        ])
      },
    })
    const button = await screen.findByRole('button', { name: 'Tạo mô tả' })
    requested = true
    await user.click(button)
    expect(await screen.findByText('Đang tạo mô tả…')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Tạo mô tả' })).toBeDisabled()
    // The job ends without a caption: the tracks list stops reporting it as running.
    fetches = 10
    await waitFor(() => expect(screen.queryByText('Đang tạo mô tả…')).not.toBeInTheDocument(), { timeout: 7000 })
    expect(screen.getByRole('button', { name: 'Tạo mô tả' })).toBeEnabled()
  }, 10000)
})
