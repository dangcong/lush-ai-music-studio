import { screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { makeCheckpoint, makeTrainingRun, mockFetch, renderWithProviders } from '../../test/utils'
import { EvalTable } from './EvalTable'

const lora = { id: 'l1', key: 'k', base_key: 'k', name: 'L', status: 'evaluation' as const, chosen_checkpoint_id: null }

describe('EvalTable', () => {
  it('formats scores: two decimals for similarity, x/10 for audio quality, melody memorize warning', () => {
    mockFetch({})
    const run = makeTrainingRun({
      status: 'completed', lora,
      checkpoints: [
        makeCheckpoint({ id: 'a', epoch: 1, step: 4, style_score: 0.734, train_score: 0.5, quality_score: 7.12,
          melody_score: 0.85, melody_match: 'tune.mp3' }),
      ],
    })
    renderWithProviders(<EvalTable run={run} selectedId={null} onSelect={() => {}} />)
    const row = within(screen.getByText('Epoch 1 · bước 4').closest('tr') as HTMLElement)
    expect(row.getByText('0.73')).toBeInTheDocument()
    expect(row.getByText('0.50')).toBeInTheDocument()
    expect(row.getByText('7.1/10')).toBeInTheDocument()
    expect(row.getByText('Có dấu hiệu học thuộc: gần giống tune.mp3')).toBeInTheDocument()
    expect(screen.getByText(/quyết định là tai bạn/)).toBeInTheDocument()
  })

  it('selects a row through its radio and by clicking the row', async () => {
    mockFetch({})
    const onSelect = vi.fn()
    const run = makeTrainingRun({ status: 'completed', lora, checkpoints: [makeCheckpoint({ id: 'a', epoch: 1, step: 4 })] })
    renderWithProviders(<EvalTable run={run} selectedId={null} onSelect={onSelect} />)
    const user = userEvent.setup()
    await user.click(screen.getByRole('radio', { name: 'Chọn Epoch 1' }))
    expect(onSelect).toHaveBeenLastCalledWith('a')
  })
})
