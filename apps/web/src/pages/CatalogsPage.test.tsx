import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { Route, Routes } from 'react-router-dom'
import { describe, expect, it } from 'vitest'
import { jsonResponse, makeCatalog, makeGenre, makeUser, mockFetch, renderWithProviders } from '../test/utils'
import { CatalogPage } from './CatalogPage'
import { CatalogsPage } from './CatalogsPage'

const genres = [makeGenre({ id: 'g-lofi', name: 'Lo-fi' }), makeGenre({ id: 'g-jazz', name: 'Jazz' })]

function setup({
  role = 'trainer' as 'admin' | 'trainer' | 'member',
  catalogs = [] as unknown[],
  createResponse,
}: { role?: 'admin' | 'trainer' | 'member'; catalogs?: unknown[]; createResponse?: () => Response } = {}) {
  const posted: unknown[] = []
  const fetchMock = mockFetch({
    'GET /catalogs': catalogs,
    'GET /genres': genres,
    'POST /catalogs': (init: RequestInit | undefined) => {
      posted.push(JSON.parse(init?.body as string))
      return createResponse ? createResponse() : jsonResponse(makeCatalog({ id: 'cat-new' }), 201)
    },
  })
  renderWithProviders(
    <Routes>
      <Route path="/datasets" element={<CatalogsPage />} />
      <Route path="/catalogs/:id" element={<CatalogPage />} />
    </Routes>,
    { user: makeUser({ role }), route: '/datasets' },
  )
  return { posted, fetchMock, user: userEvent.setup() }
}

async function fillAndSubmit(user: ReturnType<typeof userEvent.setup>, name: string) {
  await user.click(await screen.findByRole('button', { name: 'Tạo catalog' }))
  const form = within(await screen.findByRole('form', { name: 'Tạo catalog' }))
  await user.type(form.getByLabelText('Tên catalog'), name)
  await waitFor(() => expect(form.getByRole('option', { name: 'Jazz' })).toBeInTheDocument())
  await user.selectOptions(form.getByLabelText('Thể loại'), 'g-jazz')
  await user.click(screen.getByRole('button', { name: 'Tạo' }))
}

describe('CatalogsPage', () => {
  it('lists catalogs with genre, track count and rounded total minutes', async () => {
    setup({
      catalogs: [makeCatalog({ id: 'c1', name: 'Lo-fi pilot', genre_name: 'Lo-fi', track_count: 3, total_seconds: 905 })],
    })
    const link = await screen.findByRole('link', { name: 'Lo-fi pilot' })
    expect(link).toHaveAttribute('href', '/catalogs/c1')
    const row = link.closest('tr') as HTMLElement
    expect(within(row).getByText('Lo-fi')).toBeInTheDocument()
    expect(within(row).getByText('3')).toBeInTheDocument()
    expect(within(row).getByText('15')).toBeInTheDocument()
  })

  it('creates a catalog with the exact body and navigates to it', async () => {
    const { posted, user } = setup()
    await fillAndSubmit(user, 'Jazz set')
    await waitFor(() => expect(posted).toEqual([{ name: 'Jazz set', genre_id: 'g-jazz' }]))
    await waitFor(() => expect(screen.queryByRole('form')).not.toBeInTheDocument())
    expect(await screen.findByTestId('catalog-placeholder')).toHaveTextContent('cat-new')
  })

  it('shows "Tên catalog đã có" on 409', async () => {
    const { user } = setup({
      createResponse: () => jsonResponse({ error: { code: 'conflict', message: 'duplicate' } }, 409),
    })
    await fillAndSubmit(user, 'Jazz set')
    expect(await screen.findByText('Tên catalog đã có')).toBeInTheDocument()
  })

  it('shows members a notice and requests nothing', () => {
    const { fetchMock } = setup({ role: 'member' })
    expect(screen.getByText('Chỉ admin và trainer dùng được Catalog')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Tạo catalog' })).not.toBeInTheDocument()
    expect(fetchMock).not.toHaveBeenCalled()
  })
})
