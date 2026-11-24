import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import type { TrainingMetric } from '../../api/types'
import { LossChart } from './LossChart'

function m(step: number, loss: number): TrainingMetric {
  return { sequence: step, global_step: step, epoch: 1, train_loss: loss, learning_rate: null, elapsed_seconds: 1,
    gpu_memory_mb: null, recorded_at: '2026-01-01T00:00:00.000Z' }
}

describe('LossChart epoch lines', () => {
  const metrics = [m(1, 0.9), m(8, 0.6), m(16, 0.4)]

  it('draws a vertical line and an "Epoch N" label at each checkpoint step', () => {
    render(<LossChart metrics={metrics} epochSteps={[{ epoch: 2, step: 8 }, { epoch: 4, step: 16 }]} />)
    expect(screen.getAllByTestId('epoch-line')).toHaveLength(2)
    expect(screen.getByText('Epoch 2')).toBeInTheDocument()
    expect(screen.getByText('Epoch 4')).toBeInTheDocument()
  })

  it('skips steps outside the reported range and draws nothing without the prop', () => {
    const { rerender } = render(<LossChart metrics={metrics} epochSteps={[{ epoch: 9, step: 99 }]} />)
    expect(screen.queryAllByTestId('epoch-line')).toHaveLength(0)
    rerender(<LossChart metrics={metrics} />)
    expect(screen.queryAllByTestId('epoch-line')).toHaveLength(0)
  })
})
