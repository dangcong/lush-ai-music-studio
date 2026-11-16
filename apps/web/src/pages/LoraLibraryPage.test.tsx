import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import type { Lora } from '../api/types'
import { jsonResponse, makeLora, makeModel, makeUser, mockFetch, renderWithProviders } from '../test/utils'
import { LoraLibraryPage } from './LoraLibraryPage'

const loras = [
  makeLora({ id: 'l-eval', name: 'Lofi Style', key: 'lofi-style-v1', status: 'evaluation', job_id: 'run-1' }),
  makeLora({ id: 'l-ok', name: 'Jazz Style', key: 'jazz-style-v1', status: 'approved', installed_nodes: [] }),
  makeLora({ id: 'l-old', name: 'Old Style', key: 'old-style', status: 'archived', adapter_sha256: null }),
  makeLora({ id: 'l-train', name: 'Busy Style', key: 'busy-style', status: 'training' }),
]

function setup(role: 'admin' | 'member', list: Lora[] = loras) {
  const patched: { id: string; body: unknown }[] = []
  const patch = (id: string) => (init: RequestInit | undefined) => {
    const body = JSON.parse(init?.body as string) as { status: Lora['status'] }
    patched.push({ id, body })
    return jsonResponse({ ...list.find((l) => l.id === id), ...body })
  }
  mockFetch({
    'GET /loras': list,
    'GET /models': [makeModel({ id: 'model-1', name: 'ACE-Step' })],
    ...Object.fromEntries(list.map((l) => [`PATCH /loras/${l.id}`, patch(l.id)])),
  })
  renderWithProviders(<LoraLibraryPage />, { user: makeUser({ role }) })
  return { patched, user: userEvent.setup() }
}

async function rowOf(name: string) {
  return within((await screen.findByText(name)).closest('tr') as HTMLElement)
}

describe('LoraLibraryPage', () => {
  it('lists Train Styles with key, status, base model name, dataset, nodes and a short sha256', async () => {
    setup('member')
    const row = await rowOf('Lofi Style')
    expect(row.getByText('lofi-style-v1')).toBeInTheDocument()
    expect(row.getByText('Đang đánh giá')).toBeInTheDocument()
    await waitFor(() => expect(row.getByText('ACE-Step')).toBeInTheDocument())
    expect(row.getByText('lofi-set')).toBeInTheDocument()
    expect(row.getByText('render-node-01')).toBeInTheDocument()
    expect(row.getByText('abcdef012345')).toHaveAttribute(
      'title',
      'abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789',
    )

    expect((await rowOf('Jazz Style')).getByText('Đã duyệt')).toBeInTheDocument()
    expect((await rowOf('Jazz Style')).getByText('Chưa node nào cài')).toBeInTheDocument()
    expect((await rowOf('Old Style')).getByText('Lưu trữ')).toBeInTheDocument()
    expect((await rowOf('Busy Style')).getByText('Đang train')).toBeInTheDocument()
  })

  it('explains that approving a Train Style turns it into a Style', async () => {
    setup('member')
    expect(await screen.findByRole('note')).toHaveTextContent(/Bấm Duyệt thì nó trở thành một Style/)
  })

  it('non-admin: no actions and no link to the training run', async () => {
    setup('member')
    await rowOf('Lofi Style')
    expect(screen.queryByRole('button', { name: /Duyệt|Lưu trữ|Đưa về đánh giá/ })).not.toBeInTheDocument()
    expect(screen.queryByRole('link', { name: 'Xem training run' })).not.toBeInTheDocument()
  })

  it('admin: offers the status changes that make sense for each status', async () => {
    setup('admin')
    const names = async (name: string) =>
      (await rowOf(name)).queryAllByRole('button').map((b) => b.textContent)
    expect(await names('Lofi Style')).toEqual(['Duyệt', 'Lưu trữ'])
    expect(await names('Jazz Style')).toEqual(['Đưa về đánh giá', 'Lưu trữ'])
    expect(await names('Old Style')).toEqual(['Đưa về đánh giá'])
    expect(await names('Busy Style')).toEqual([])
    expect((await rowOf('Lofi Style')).getByRole('link', { name: 'Xem training run' })).toHaveAttribute(
      'href',
      '/training/run-1',
    )
  })

  it('admin: Duyệt / Lưu trữ / Đưa về đánh giá send PATCH /loras/{id} with the new status', async () => {
    const { user, patched } = setup('admin')
    await user.click(await screen.findByRole('button', { name: 'Duyệt Lofi Style' }))
    await waitFor(() => expect(patched).toEqual([{ id: 'l-eval', body: { status: 'approved' } }]))

    await user.click(screen.getByRole('button', { name: 'Lưu trữ Jazz Style' }))
    await waitFor(() => expect(patched[1]).toEqual({ id: 'l-ok', body: { status: 'archived' } }))

    await user.click(screen.getByRole('button', { name: 'Đưa về đánh giá Old Style' }))
    await waitFor(() => expect(patched[2]).toEqual({ id: 'l-old', body: { status: 'evaluation' } }))
  })

  it('shows an empty state when there is no Train Style', async () => {
    setup('member', [])
    expect(await screen.findByText('Chưa có Train Style nào')).toBeInTheDocument()
  })
})
