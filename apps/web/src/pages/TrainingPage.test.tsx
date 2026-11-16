import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import type { TrainingRunInput } from '../api/types'
import { jsonResponse, makeModel, makeTrainingRun, makeUser, mockFetch, renderWithProviders } from '../test/utils'
import { TrainingPage } from './TrainingPage'

const models = [
  makeModel({ id: 'm-mock', name: 'Mock Engine' }),
  makeModel({ id: 'm-ace', engine: 'acestep', key: 'ace-step-v1', name: 'ACE-Step', version: '1.5' }),
]

const datasets = [
  {
    key: 'lofi-set',
    node_id: 'n1',
    node_name: 'render-node-01',
    online: true,
    can_train: true,
    info: { key: 'lofi-set', name: 'Lofi set', samples: 10, seconds: 754 },
  },
  { key: 'broken', node_id: 'n1', node_name: 'render-node-01', online: true, can_train: false, info: { key: 'broken' } },
]

function setup({
  runs = [] as unknown[],
  datasetList = datasets as unknown[],
  createResponse,
  role = 'admin' as const,
}: {
  runs?: unknown[]
  datasetList?: unknown[]
  createResponse?: () => Response
  role?: 'admin' | 'member' | 'trainer'
} = {}) {
  const posted: TrainingRunInput[] = []
  const fetchMock = mockFetch({
    'GET /training-runs': runs,
    'GET /datasets': datasetList,
    'GET /models': models,
    'POST /training-runs': (init: RequestInit | undefined) => {
      posted.push(JSON.parse(init?.body as string) as TrainingRunInput)
      return createResponse ? createResponse() : jsonResponse(makeTrainingRun({ id: 'run-new' }), 201)
    },
  })
  renderWithProviders(<TrainingPage />, { user: makeUser({ role }) })
  return { posted, fetchMock, user: userEvent.setup() }
}

async function openForm(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole('button', { name: 'Tạo training run' }))
  return within(await screen.findByRole('form', { name: 'Tạo training run' }))
}

describe('TrainingPage', () => {
  it('is titled "Train Style" and never calls it model training or fine-tuning', async () => {
    setup()
    expect(await screen.findByRole('heading', { name: 'Train Style' })).toBeInTheDocument()
    expect(document.body.textContent).not.toMatch(/fine-?tun|train model|huấn luyện model/i)
  })

  it('shows non-admins a notice and requests nothing', () => {
    const { fetchMock } = setup({ role: 'member' })
    expect(screen.getByText(/Chỉ admin mới tạo và xem được training run/)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Tạo training run' })).not.toBeInTheDocument()
    expect(fetchMock).not.toHaveBeenCalled()
  })

  it('lists runs with dataset, status, translated stage and progress', async () => {
    setup({
      runs: [
        makeTrainingRun({ id: 'run-2', title: 'Jazz Style', status: 'running', stage: 'training', progress: 0.5 }),
        makeTrainingRun({ id: 'run-1', title: 'Lofi Style', status: 'completed', progress: 1 }),
      ],
    })
    const link = await screen.findByRole('link', { name: 'Jazz Style' })
    expect(link).toHaveAttribute('href', '/training/run-2')
    const row = link.closest('tr') as HTMLElement
    expect(within(row).getByText('lofi-set')).toBeInTheDocument()
    expect(within(row).getByText('Đang chạy')).toBeInTheDocument()
    expect(within(row).getByText('Đang train')).toBeInTheDocument()
    expect(within(row).getByRole('progressbar')).toHaveAttribute('aria-valuenow', '50')
    expect(screen.getByRole('link', { name: 'Lofi Style' })).toHaveAttribute('href', '/training/run-1')
    // Training runs cannot be retried.
    expect(screen.queryByRole('button', { name: /Thử lại/ })).not.toBeInTheDocument()
  })

  it('pre-fills the technically tested configuration and labels it as not a quality recommendation', async () => {
    const { user } = setup()
    const form = await openForm(user)

    expect(form.getByText(/cấu hình đã chạy thử kỹ thuật, không phải khuyến nghị chất lượng/)).toBeInTheDocument()
    expect(form.getByLabelText('Rank')).toHaveValue(32)
    expect(form.getByLabelText('Alpha')).toHaveValue(64)
    expect(form.getByLabelText('Learning rate')).toHaveValue(0.0001)
    expect(form.getByLabelText('Epochs')).toHaveValue(4)
    expect(form.getByLabelText('Gradient accumulation')).toHaveValue(4)
    expect(form.getByLabelText('Save every N epochs')).toHaveValue(2)
    expect(form.getByLabelText('Warmup steps')).toHaveValue(2)
    expect(form.getByLabelText('Seed')).toHaveValue(null)
    expect(form.getByLabelText('Giới hạn thời gian (phút)')).toHaveValue(30)
  })

  it('offers only trainable datasets and acestep base models', async () => {
    const { user } = setup()
    const form = await openForm(user)
    const dataset = form.getByLabelText('Dataset')
    expect(within(dataset).getAllByRole('option').map((o) => o.textContent)).toEqual([
      'Lofi set — 10 mẫu · 12.6 phút',
    ])
    const model = form.getByLabelText('Base model')
    expect(within(model).getAllByRole('option').map((o) => o.textContent)).toEqual(['ACE-Step (1.5)'])
    await waitFor(() => expect(model).toHaveValue('m-ace'))
  })

  it('explains the step arithmetic from the dataset sample count and updates it live', async () => {
    const { user } = setup()
    const form = await openForm(user)
    const explainer = within(form.getByRole('region', { name: 'Cách tính số bước' }))
    const value = (label: string) => explainer.getByText(label).nextElementSibling?.textContent

    await waitFor(() => expect(value('Số mẫu')).toBe('10'))
    expect(value('Optimizer steps / epoch')).toBe('3') // ceil(10 / 4)
    expect(value('Tổng optimizer steps')).toBe('12') // 3 × 4 epochs
    expect(explainer.getByText(/optimizer step/, { selector: 'strong' })).toBeInTheDocument()
    expect(explainer.getByText(/không phải micro-batch/)).toBeInTheDocument()
    expect(explainer.getByText(/Trainer không hỗ trợ/)).toBeInTheDocument()

    const accumulation = form.getByLabelText('Gradient accumulation')
    await user.clear(accumulation)
    await user.type(accumulation, '3')
    expect(value('Optimizer steps / epoch')).toBe('4') // ceil(10 / 3)
    expect(value('Tổng optimizer steps')).toBe('16')
  })

  it('does not invent step numbers when the node did not report the sample count', async () => {
    const { user } = setup({
      datasetList: [{ key: 'raw', node_id: 'n1', node_name: 'node', online: true, can_train: true, info: { key: 'raw' } }],
    })
    const form = await openForm(user)
    expect(await form.findByText(/Node chưa báo số mẫu/)).toBeInTheDocument()
    expect(form.queryByText('Tổng optimizer steps')).not.toBeInTheDocument()
  })

  it('warns that the website cannot generate music during a run, without promising a time', async () => {
    const { user } = setup()
    const form = await openForm(user)
    const warning = form.getByRole('alert')
    expect(warning).toHaveTextContent('Trong lúc training run đang chạy, website không tạo nhạc được.')
    expect(warning).toHaveTextContent('nằm chờ trong hàng đợi')
    expect(warning).toHaveTextContent('Job tạo nhạc đang chờ luôn được chạy trước một training run mới')
    expect(warning).toHaveTextContent('Không có mốc thời gian cam kết')
  })

  it('has no vocal or lyrics fields', async () => {
    const { user } = setup()
    const form = await openForm(user)
    expect(form.queryByLabelText(/vocal|lyric|lời/i)).not.toBeInTheDocument()
  })

  it('validates before sending anything', async () => {
    const { user, posted } = setup()
    const form = await openForm(user)
    await waitFor(() => expect(form.getByLabelText('Base model')).toHaveValue('m-ace'))
    const submit = screen.getByRole('button', { name: 'Bắt đầu train' })

    await user.click(submit)
    expect(await screen.findByText('Hãy nhập Tên Train Style.')).toBeInTheDocument()

    await user.type(form.getByLabelText('Tên Train Style'), 'Lofi Style')
    const rank = form.getByLabelText('Rank')
    await user.clear(rank)
    await user.type(rank, '500')
    await user.click(submit)
    expect(await screen.findByText('Rank phải là số nguyên từ 1 đến 128.')).toBeInTheDocument()

    await user.clear(rank)
    await user.type(rank, '16')
    const saveEvery = form.getByLabelText('Save every N epochs')
    await user.clear(saveEvery)
    await user.type(saveEvery, '9')
    await user.click(submit)
    expect(await screen.findByText('Save every phải là số nguyên từ 1 đến 4.')).toBeInTheDocument()
    expect(posted).toHaveLength(0)
  })

  it('posts a body matching the contract', async () => {
    const { user, posted } = setup()
    const form = await openForm(user)
    await waitFor(() => expect(form.getByLabelText('Base model')).toHaveValue('m-ace'))
    await user.type(form.getByLabelText('Tên Train Style'), '  Lofi Style ')
    await user.type(form.getByLabelText('Seed'), '7')
    await user.click(screen.getByRole('button', { name: 'Bắt đầu train' }))

    await waitFor(() => expect(posted).toHaveLength(1))
    expect(posted[0]).toEqual({
      name: 'Lofi Style',
      dataset_key: 'lofi-set',
      base_model_id: 'm-ace',
      rank: 32,
      alpha: 64,
      learning_rate: 0.0001,
      epochs: 4,
      gradient_accumulation: 4,
      save_every: 2,
      warmup_steps: 2,
      seed: 7,
      max_minutes: 30,
    })
    // The modal closes once the run is queued.
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
  })

  it('shows the API message when another run is already active (409)', async () => {
    const { user } = setup({
      createResponse: () =>
        jsonResponse({ error: { code: 'conflict', message: 'Đang có training run khác chưa kết thúc' } }, 409),
    })
    const form = await openForm(user)
    await waitFor(() => expect(form.getByLabelText('Base model')).toHaveValue('m-ace'))
    await user.type(form.getByLabelText('Tên Train Style'), 'Lofi Style')
    await user.click(screen.getByRole('button', { name: 'Bắt đầu train' }))
    expect(await screen.findByText('Đang có training run khác chưa kết thúc')).toBeInTheDocument()
    expect(screen.getByRole('dialog')).toBeInTheDocument()
  })

  it('explains where datasets come from when none can be trained on', async () => {
    const { user } = setup({ datasetList: [datasets[1]] })
    const form = await openForm(user)
    expect(form.getByText(/thư mục trên Render Node/)).toBeInTheDocument()
    expect(form.getByText(/Trang quản lý Dataset trên web sẽ có sau/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Bắt đầu train' })).toBeDisabled()
  })
})
