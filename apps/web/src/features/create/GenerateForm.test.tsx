import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import type { GenerationRequest } from '../../api/types'
import {
  jsonResponse,
  makeGenre,
  makeJob,
  makeModel,
  makePreset,
  mockFetch,
  renderWithProviders,
} from '../../test/utils'
import { GenerateForm } from './GenerateForm'

const genres = [
  makeGenre({
    id: 'g-jazz',
    name: 'Jazz',
    slug: 'jazz',
    instruments: ['piano', 'upright bass'],
    moods: ['warm'],
    default_duration: 90,
    bpm_min: 70,
    bpm_max: 110,
  }),
  makeGenre({ id: 'g-lofi', name: 'Lo-fi', slug: 'lofi', sort_order: 1 }),
  makeGenre({ id: 'g-bebop', parent_id: 'g-jazz', name: 'Bebop', slug: 'bebop' }),
  makeGenre({ id: 'g-chillhop', parent_id: 'g-lofi', name: 'Chillhop', slug: 'chillhop' }),
]

const models = [
  makeModel({ id: 'm-a', name: 'Model A' }),
  makeModel({ id: 'm-b', name: 'Model B', installed_nodes: [{ node_id: 'n2', node_name: 'node-2', online: false }] }),
  makeModel({ id: 'm-off', name: 'Model Off', status: 'disabled' }),
]

const presets = [
  makePreset({
    id: 'p-smoky',
    genre_id: 'g-jazz',
    name: 'Smoky Club',
    model_id: 'm-b',
    duration: 180,
    bpm: 84,
    instruments: ['saxophone'],
    moods: ['smoky'],
    params: { inference_steps: 40 },
  }),
  makePreset({ id: 'p-tape', genre_id: 'g-lofi', name: 'Tape Hiss' }),
]

function setup(loras: unknown[] = []) {
  const posted: GenerationRequest[] = []
  mockFetch({
    'GET /genres': genres,
    'GET /presets': presets,
    'GET /models': models,
    'GET /loras': loras,
    'POST /generations': (init: RequestInit | undefined) => {
      posted.push(JSON.parse(init?.body as string) as GenerationRequest)
      return jsonResponse(makeJob(), 201)
    },
  })
  renderWithProviders(<GenerateForm />)
  return { posted, user: userEvent.setup() }
}

function optionNames(select: HTMLElement): string[] {
  return within(select)
    .getAllByRole('option')
    .map((o) => o.textContent ?? '')
}

describe('GenerateForm', () => {
  it('lists only active models and selects one by default', async () => {
    setup()
    const model = await screen.findByLabelText('Model')
    expect(optionNames(model)).toEqual(['Model A (1)', 'Model B (1)'])
    await waitFor(() => expect(model).toHaveValue('m-a'))
  })

  it('filters subgenres and presets by the selected genre', async () => {
    const { user } = setup()
    const genre = await screen.findByLabelText('Genre')
    const subgenre = screen.getByLabelText('Subgenre')
    expect(optionNames(genre)).toEqual(['— Không chọn —', 'Jazz', 'Lo-fi'])
    expect(subgenre).toBeDisabled()

    await user.selectOptions(genre, 'g-jazz')
    expect(subgenre).toBeEnabled()
    expect(optionNames(subgenre)).toEqual(['— Không chọn —', 'Bebop'])
    expect(optionNames(screen.getByLabelText('Style Preset'))).toEqual(['— Không dùng preset —', 'Smoky Club'])

    await user.selectOptions(genre, 'g-lofi')
    expect(optionNames(subgenre)).toEqual(['— Không chọn —', 'Chillhop'])
    expect(optionNames(screen.getByLabelText('Style Preset'))).toEqual(['— Không dùng preset —', 'Tape Hiss'])
  })

  it('applies genre defaults and suggests its instruments and moods', async () => {
    const { user } = setup()
    await user.selectOptions(await screen.findByLabelText('Genre'), 'g-jazz')

    expect(screen.getByLabelText('Duration')).toHaveValue('90')
    const piano = screen.getByRole('button', { name: 'piano' })
    expect(piano).toHaveAttribute('aria-pressed', 'false')
    await user.click(piano)
    expect(piano).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByRole('button', { name: 'warm' })).toBeInTheDocument()
  })

  it('pre-fills from a preset but leaves every field editable', async () => {
    const { user } = setup()
    await user.selectOptions(await screen.findByLabelText('Genre'), 'g-jazz')
    await user.selectOptions(screen.getByLabelText('Style Preset'), 'p-smoky')

    expect(screen.getByLabelText('Model')).toHaveValue('m-b')
    expect(screen.getByLabelText('Duration')).toHaveValue('180')
    expect(screen.getByRole('button', { name: 'saxophone' })).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByRole('button', { name: 'smoky' })).toHaveAttribute('aria-pressed', 'true')

    await user.click(screen.getByRole('button', { name: 'Nâng cao' }))
    const bpm = screen.getByLabelText('BPM')
    expect(bpm).toHaveValue(84)
    expect(screen.getByLabelText('Inference Steps')).toHaveValue(40)

    await user.clear(bpm)
    await user.type(bpm, '90')
    expect(bpm).toHaveValue(90)
    await user.selectOptions(screen.getByLabelText('Model'), 'm-a')
    expect(screen.getByLabelText('Model')).toHaveValue('m-a')
  })

  it('warns when no online node has the model, without blocking submit', async () => {
    const { user } = setup()
    const model = await screen.findByLabelText('Model')
    await waitFor(() => expect(model).toHaveValue('m-a'))
    expect(screen.queryByText(/không có Render Node nào online/i)).not.toBeInTheDocument()

    await user.selectOptions(model, 'm-b')
    expect(screen.getByText(/không có Render Node nào online/i)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Generate' })).toBeEnabled()
  })

  it('explains that BPM / Key / Time Signature are only guidance, and hides LoRA without data', async () => {
    const { user } = setup()
    await user.click(await screen.findByRole('button', { name: 'Nâng cao' }))
    expect(screen.getByText(/hướng dẫn cho model, không bảo đảm chính xác/)).toBeInTheDocument()
    expect(screen.queryByLabelText('LoRA')).not.toBeInTheDocument()
  })

  it('shows LoRA controls when /loras has data', async () => {
    const { user } = setup([
      { id: 'l-1', key: 'lofi-v1', name: 'Lofi LoRA', version: '1', genre_id: null, base_model_id: 'm-a', status: 'active', installed_nodes: [] },
    ])
    await user.click(await screen.findByRole('button', { name: 'Nâng cao' }))
    expect(await screen.findByLabelText('LoRA')).toBeInTheDocument()
    expect(screen.getByLabelText('LoRA Scale')).toBeDisabled()
  })

  it('requires a prompt before sending anything', async () => {
    const { user, posted } = setup()
    await user.click(await screen.findByRole('button', { name: 'Generate' }))
    expect(await screen.findByText('Hãy nhập Prompt.')).toBeInTheDocument()
    expect(posted).toHaveLength(0)
  })

  it('posts a body matching the contract, with a new idempotency_key per click', async () => {
    const { user, posted } = setup()
    await user.type(await screen.findByLabelText('Prompt'), '  Warm jazz piano  ')
    await user.type(screen.getByLabelText('Tiêu đề'), 'Rainy night')
    await user.selectOptions(screen.getByLabelText('Genre'), 'g-jazz')
    await user.selectOptions(screen.getByLabelText('Subgenre'), 'g-bebop')
    await user.click(screen.getByRole('button', { name: 'piano' }))
    await user.type(screen.getByLabelText('Thêm Moods'), 'late night{Enter}')
    await user.click(screen.getByRole('button', { name: 'Nâng cao' }))
    await user.type(screen.getByLabelText('BPM'), '80')
    await user.type(screen.getByLabelText('Key'), 'A minor')
    await user.type(screen.getByLabelText('Seed'), '1234')
    await user.type(screen.getByLabelText('Guidance Scale'), '7.5')

    const generate = screen.getByRole('button', { name: 'Generate' })
    await user.click(generate)
    await waitFor(() => expect(posted).toHaveLength(1))

    const { idempotency_key: firstKey, ...body } = posted[0]!
    expect(body).toEqual({
      prompt: 'Warm jazz piano',
      title: 'Rainy night',
      project_id: null,
      genre_id: 'g-jazz',
      subgenre_id: 'g-bebop',
      preset_id: null,
      model_id: 'm-a',
      lora_id: null,
      lora_scale: null,
      duration: 90,
      bpm: 80,
      key: 'A minor',
      time_signature: null,
      seed: 1234,
      instruments: ['piano'],
      moods: ['late night'],
      instrumental: true,
      params: { inference_steps: null, guidance_scale: 7.5 },
    })
    expect(firstKey).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/)

    await waitFor(() => expect(generate).toBeEnabled())
    await user.click(generate)
    await waitFor(() => expect(posted).toHaveLength(2))
    expect(posted[1]!.idempotency_key).not.toBe(firstKey)
  })

  it('shows the API error message when the request is rejected', async () => {
    mockFetch({
      'GET /genres': genres,
      'GET /presets': presets,
      'GET /models': models,
      'GET /loras': [],
      'POST /generations': () =>
        jsonResponse({ error: { code: 'validation', message: 'Model đang bị tắt' } }, 422),
    })
    renderWithProviders(<GenerateForm />)
    const user = userEvent.setup()
    await user.type(await screen.findByLabelText('Prompt'), 'Dark ambient drone')
    await user.click(screen.getByRole('button', { name: 'Generate' }))
    expect(await screen.findByText('Model đang bị tắt')).toBeInTheDocument()
  })
})
