import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import type { Lora, Preset } from '../../api/types'
import {
  jsonResponse,
  makeGenre,
  makeLora,
  makeModel,
  makePreset,
  makeUser,
  mockFetch,
  renderWithProviders,
} from '../../test/utils'
import { StylesPage } from './StylesPage'

const loras = [
  makeLora({ id: 'l-new', name: 'Lofi Style', base_model_id: 'model-1', status: 'approved' }),
  makeLora({ id: 'l-set', name: 'Jazz Style', base_model_id: 'model-1', status: 'approved' }),
  makeLora({ id: 'l-eval', name: 'Trial Style', status: 'evaluation' }),
  makeLora({ id: 'l-old', name: 'Old Style', status: 'archived' }),
]

const presets = [
  makePreset({ id: 'p-jazz', name: 'Smoky Club', genre_id: 'g-jazz', kind: 'trained', lora_id: 'l-set', lora_scale: 0.8, duration_min: 60, duration_max: 180 }),
  makePreset({ id: 'p-plain', name: 'Plain Preset', genre_id: 'g-jazz' }),
  makePreset({ id: 'p-trial', name: 'Trial Preset', genre_id: 'g-jazz', kind: 'trained', lora_id: 'l-eval' }),
]

function setup(loraList: Lora[] = loras, presetList: Preset[] = presets, capabilities = ['generate', 'lora']) {
  const sent: { method: string; id?: string; body: Record<string, unknown> }[] = []
  mockFetch({
    'GET /loras': loraList,
    'GET /presets': presetList,
    'GET /genres': [makeGenre({ id: 'g-jazz', name: 'Jazz', slug: 'jazz' })],
    'GET /models': [makeModel({ id: 'model-1', name: 'ACE-Step', capabilities })],
    'POST /presets': (init: RequestInit | undefined) => {
      const body = JSON.parse(init?.body as string) as Record<string, unknown>
      sent.push({ method: 'POST', body })
      return jsonResponse({ ...makePreset(), ...body, id: 'p-created' }, 201)
    },
    'PATCH /presets/p-jazz': (init: RequestInit | undefined) => {
      const body = JSON.parse(init?.body as string) as Record<string, unknown>
      sent.push({ method: 'PATCH', id: 'p-jazz', body })
      return jsonResponse({ ...presets[0], ...body })
    },
  })
  renderWithProviders(<StylesPage />, { user: makeUser({ role: 'admin' }) })
  return { sent, user: userEvent.setup() }
}

async function rowOf(name: string) {
  return within((await screen.findByText(name)).closest('tr') as HTMLElement)
}

describe('StylesPage', () => {
  it('lists approved Train Styles only; other presets and unapproved Train Styles are not Styles', async () => {
    setup()
    const row = await rowOf('Smoky Club')
    expect(row.getByText('Jazz Style')).toBeInTheDocument()
    expect(row.getByText('Jazz')).toBeInTheDocument()
    expect(row.getByText('0.8')).toBeInTheDocument()
    expect(row.getByText('60–180s')).toBeInTheDocument()
    await waitFor(() => expect(row.getByText('ACE-Step')).toBeInTheDocument())

    expect(screen.queryByText('Plain Preset')).not.toBeInTheDocument()
    expect(screen.queryByText('Trial Preset')).not.toBeInTheDocument()
    expect(screen.queryByText('Old Style')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /Thêm/ })).not.toBeInTheDocument()
  })

  it('a newly approved Train Style appears and asks for a genre before it can be used', async () => {
    const { user, sent } = setup()
    // The Style has no name of its own yet, so the row shows the Train Style's name twice.
    const button = await screen.findByRole('button', { name: 'Thiết lập Lofi Style' })
    const row = within(button.closest('tr') as HTMLElement)
    expect(row.getAllByText('Lofi Style')).toHaveLength(2)
    expect(row.getByText('Cần chọn genre')).toBeInTheDocument()
    expect(screen.getByRole('alert')).toHaveTextContent(/chưa dùng được ở trang Tạo nhạc/)

    await user.click(button)
    expect(screen.getByLabelText('Name')).toHaveValue('Lofi Style')
    await user.selectOptions(screen.getByLabelText('Genre'), 'g-jazz')
    await user.type(screen.getByLabelText('Duration từ (giây)'), '90')
    await user.type(screen.getByLabelText('Duration đến (giây)'), '300')
    await user.click(screen.getByRole('button', { name: 'Lưu' }))

    await waitFor(() => expect(sent).toHaveLength(1))
    expect(sent[0]).toMatchObject({
      method: 'POST',
      body: { name: 'Lofi Style', genre_id: 'g-jazz', kind: 'trained', lora_id: 'l-new', model_id: 'model-1', lora_scale: 1, duration_min: 90, duration_max: 300 },
    })
  })

  it('saves Thinking (LM) when the base model has the LM', async () => {
    const { user, sent } = setup(loras, presets, ['generate', 'lora', 'thinking'])
    await user.click(await screen.findByRole('button', { name: 'Thiết lập Lofi Style' }))
    await user.selectOptions(screen.getByLabelText('Genre'), 'g-jazz')
    await user.click(screen.getByLabelText('Thinking (LM)'))
    await user.click(screen.getByRole('button', { name: 'Lưu' }))
    await waitFor(() => expect(sent).toHaveLength(1))
    expect(sent[0]!.body.params).toMatchObject({ thinking: true })
  })

  it('has no Thinking switch when the base model has no LM', async () => {
    const { user } = setup()
    await user.click(await screen.findByRole('button', { name: 'Thiết lập Lofi Style' }))
    expect(screen.queryByLabelText('Thinking (LM)')).toBeNull()
  })

  it('a Style saved with Thinking can still turn it off after the LM is gone', async () => {
    const lmPresets = [{ ...presets[0]!, params: { inference_steps: null, guidance_scale: null, thinking: true } }]
    const { user, sent } = setup(loras, lmPresets)
    await user.click(await screen.findByRole('button', { name: 'Sửa Smoky Club' }))
    const box = screen.getByLabelText('Thinking (LM)')
    expect(box).toBeChecked()
    await user.click(box)
    await user.click(screen.getByRole('button', { name: 'Lưu' }))
    await waitFor(() => expect(sent).toHaveLength(1))
    expect(sent[0]!.body.params).toMatchObject({ thinking: false })
  })

  it('turns a Style off and on', async () => {
    const { user, sent } = setup()
    await user.click(await screen.findByRole('switch', { name: 'Bật Smoky Club' }))
    await waitFor(() => expect(sent).toEqual([{ method: 'PATCH', id: 'p-jazz', body: { is_active: false } }]))
  })

  it('points to the Train Style Library when nothing is approved yet', async () => {
    setup([loras[2]!], [])
    expect(await screen.findByText('Chưa có Style nào')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Train Style Library' })).toHaveAttribute('href', '/lora-library')
  })
})
