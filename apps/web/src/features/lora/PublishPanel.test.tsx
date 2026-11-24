import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { jsonResponse, makeCheckpoint, makeTrainingRun, mockFetch, renderWithProviders } from '../../test/utils'
import { PublishPanel } from './PublishPanel'

const lora = { id: 'l1', key: 'k', base_key: 'k', name: 'Lofi', status: 'approved' as const, chosen_checkpoint_id: 'a' }
const cp = makeCheckpoint({ id: 'a', epoch: 3, step: 12, status: 'chosen' })

describe('PublishPanel', () => {
  it('defaults the name and strength to the existing Style and says when only the Style was updated', async () => {
    const bodies: unknown[] = []
    mockFetch({
      'POST /loras/l1/publish': (init: RequestInit | undefined) => {
        bodies.push(JSON.parse(String(init?.body)))
        return jsonResponse({ job_id: null, style_preset_id: 's1' })
      },
    })
    const run = makeTrainingRun({ status: 'completed', lora, checkpoints: [cp],
      style_preset: { id: 's1', name: 'Lofi Chill', lora_scale: 0.55, is_active: true } })
    renderWithProviders(<PublishPanel run={run} selected={cp} scale={0.55} onScale={() => {}} />)
    const panel = within(screen.getByRole('region', { name: 'Đưa vào Style Preset' }))
    expect(panel.getByLabelText('Tên hiện ở Style Preset')).toHaveValue('Lofi Chill')
    expect(panel.getByText('Nhẹ')).toBeInTheDocument()
    expect(panel.getByText('Mạnh')).toBeInTheDocument()
    expect(panel.getByText(/giữ 30 ngày rồi tự xóa/)).toBeInTheDocument()
    await userEvent.setup().click(panel.getByRole('button', { name: 'Đưa Epoch 3 vào Style Preset' }))
    await waitFor(() => expect(bodies).toEqual([{ checkpoint_id: 'a', style_name: 'Lofi Chill', lora_scale: 0.55 }]))
    expect(await panel.findByText('Bản này đã có trên máy, đã cập nhật Style Preset.')).toBeInTheDocument()
  })

  it('reports how many checkpoints "Xóa ngay" removes', async () => {
    mockFetch({ 'POST /loras/l1/cleanup': () => jsonResponse({ job_id: 'f2', count: 2 }, 202) })
    const run = makeTrainingRun({ status: 'completed', lora, checkpoints: [cp] })
    renderWithProviders(<PublishPanel run={run} selected={cp} scale={0.7} onScale={() => {}} />)
    const user = userEvent.setup()
    await user.click(screen.getByRole('button', { name: 'Xóa ngay' }))
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Xóa ngay' }))
    expect(await screen.findByText('Đang xóa 2 bản trên máy…')).toBeInTheDocument()
  })
})
