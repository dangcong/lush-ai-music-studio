import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { jsonResponse, makeGenre, makeUser, mockFetch, renderWithProviders } from '../../test/utils'
import { GenresPage } from './GenresPage'

const jazz = makeGenre({ id: 'g-jazz', name: 'Jazz', slug: 'jazz', duration_min: 60, duration_max: 120 })
const lofi = makeGenre({ id: 'g-lofi', name: 'Lo-fi', slug: 'lofi', duration_min: 90, duration_max: 90, sort_order: 1 })

function setup() {
  const sent: Record<string, unknown>[] = []
  const record = (status: number) => (init: RequestInit | undefined) => {
    const body = JSON.parse(init?.body as string) as Record<string, unknown>
    sent.push(body)
    return jsonResponse({ ...jazz, ...body }, status)
  }
  mockFetch({
    'GET /genres': [jazz, lofi],
    'PATCH /genres/g-jazz': record(200),
    'POST /genres': record(201),
  })
  renderWithProviders(<GenresPage />, { user: makeUser({ role: 'admin' }) })
  return { sent, user: userEvent.setup() }
}

describe('GenresPage duration range', () => {
  it('shows each genre range, or a single value when both ends are equal', async () => {
    setup()
    expect(await screen.findByText(/60–120s/)).toBeInTheDocument()
    expect(screen.getByText(/^90s/)).toBeInTheDocument()
  })

  it('edits the range with two typed fields and sends duration_min / duration_max', async () => {
    const { user, sent } = setup()
    await user.click(await screen.findByRole('button', { name: 'Sửa Jazz' }))
    const from = screen.getByLabelText('Duration từ (giây)')
    const to = screen.getByLabelText('Duration đến (giây)')
    expect(from).toHaveValue(60)
    expect(to).toHaveValue(120)

    await user.clear(to)
    await user.type(to, '180')
    await user.clear(from)
    await user.type(from, '90')
    await user.click(screen.getByRole('button', { name: 'Lưu' }))

    await waitFor(() => expect(sent).toHaveLength(1))
    expect(sent[0]).toMatchObject({ duration_min: 90, duration_max: 180 })
    expect(sent[0]).not.toHaveProperty('default_duration')
  })

  it('a new genre starts at 60–120 seconds; a new subgenre starts empty and follows its genre', async () => {
    const { user } = setup()
    await user.click(await screen.findByRole('button', { name: 'Thêm genre' }))
    expect(screen.getByLabelText('Duration từ (giây)')).toHaveValue(60)
    expect(screen.getByLabelText('Duration đến (giây)')).toHaveValue(120)
    expect(screen.getByLabelText('Duration đến (giây)')).toHaveAttribute('max', '300')
    await user.click(screen.getByRole('button', { name: 'Hủy bỏ' }))

    await user.click(screen.getAllByRole('button', { name: 'Subgenre' })[0]!)
    expect(screen.getByLabelText('Duration từ (giây)')).toHaveValue(null)
    expect(screen.getByLabelText('Duration đến (giây)')).toHaveValue(null)
  })

  it('clearing both fields sends an empty range', async () => {
    const { user, sent } = setup()
    await user.click(await screen.findByRole('button', { name: 'Sửa Jazz' }))
    await user.clear(screen.getByLabelText('Duration từ (giây)'))
    await user.clear(screen.getByLabelText('Duration đến (giây)'))
    await user.click(screen.getByRole('button', { name: 'Lưu' }))
    await waitFor(() => expect(sent).toHaveLength(1))
    expect(sent[0]).toMatchObject({ duration_min: null, duration_max: null })
  })
})
