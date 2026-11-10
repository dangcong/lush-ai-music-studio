import { screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { Route, Routes } from 'react-router-dom'
import { describe, expect, it } from 'vitest'
import { App } from '../App'
import { makeUser, mockFetch, renderWithProviders } from '../test/utils'
import { Sidebar } from './Sidebar'

const summary = { queued: 2, preparing: 1, running: 1, nodes_online: 1 }

describe('Sidebar', () => {
  it('hides the admin menu from members', async () => {
    mockFetch({ 'GET /queue/summary': summary })
    renderWithProviders(<Sidebar />, { user: makeUser({ role: 'member' }) })

    expect(screen.getByRole('link', { name: 'Tạo nhạc' })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Thư viện' })).toBeInTheDocument()
    expect(screen.queryByText('Quản trị')).not.toBeInTheDocument()
    expect(screen.queryByRole('link', { name: 'Render Nodes' })).not.toBeInTheDocument()
    expect(screen.queryByRole('link', { name: 'Users' })).not.toBeInTheDocument()
  })

  it('hides the admin menu from trainers', () => {
    mockFetch({ 'GET /queue/summary': summary })
    renderWithProviders(<Sidebar />, { user: makeUser({ role: 'trainer' }) })
    expect(screen.queryByText('Quản trị')).not.toBeInTheDocument()
  })

  it('shows every admin section to admins', () => {
    mockFetch({ 'GET /queue/summary': summary })
    renderWithProviders(<Sidebar />, { user: makeUser({ role: 'admin' }) })

    expect(screen.getByText('Quản trị')).toBeInTheDocument()
    for (const name of ['Genres', 'Presets', 'Models', 'Render Nodes', 'Users', 'Audit Logs']) {
      expect(screen.getByRole('link', { name })).toBeInTheDocument()
    }
  })

  it('shows queue counts and online nodes from /queue/summary', async () => {
    mockFetch({ 'GET /queue/summary': summary })
    renderWithProviders(<Sidebar />)

    const indicator = await screen.findByRole('status', { name: 'Trạng thái hàng đợi' })
    expect(indicator).toHaveTextContent('2 đang chờ')
    expect(indicator).toHaveTextContent('2 đang chạy')
    expect(indicator).toHaveTextContent('1 node online')
    expect(screen.getByLabelText('4 job chưa xong')).toBeInTheDocument()
  })

  it('marks Phase 3 sections as coming soon', () => {
    mockFetch({ 'GET /queue/summary': summary })
    renderWithProviders(<Sidebar />)
    expect(screen.getByRole('link', { name: /Dataset/ })).toHaveTextContent('Sắp có')
    expect(screen.getByRole('link', { name: /Training/ })).toHaveTextContent('Sắp có')
    expect(screen.getByRole('link', { name: /LoRA Library/ })).toHaveTextContent('Sắp có')
  })

  it('shows the user name and role with password / sign-out actions', async () => {
    mockFetch({ 'GET /queue/summary': summary })
    renderWithProviders(<Sidebar />, { user: makeUser({ display_name: 'Linh', role: 'trainer' }) })

    const user = userEvent.setup()
    await user.click(screen.getByRole('button', { name: /Linh.*Trainer/ }))
    expect(screen.getByRole('menuitem', { name: 'Đổi mật khẩu' })).toBeInTheDocument()
    expect(screen.getByRole('menuitem', { name: 'Đăng xuất' })).toBeInTheDocument()
  })
})

describe('route guards', () => {
  const routes = {
    'GET /queue/summary': summary,
    'GET /genres': [],
    'GET /presets': [],
    'GET /models': [],
    'GET /loras': [],
    'GET /jobs': [],
    'GET /tracks': [],
    'GET /users': [],
  }

  function renderApp(route: string, user: ReturnType<typeof makeUser> | null) {
    mockFetch(routes)
    renderWithProviders(
      <Routes>
        <Route path="*" element={<App />} />
      </Routes>,
      { user, route },
    )
  }

  it('redirects signed-out visitors to the login page', async () => {
    renderApp('/library', null)
    expect(await screen.findByRole('button', { name: 'Đăng nhập' })).toBeInTheDocument()
  })

  it('keeps non-admins out of /admin routes', async () => {
    renderApp('/admin/users', makeUser({ role: 'member' }))
    expect(await screen.findByRole('heading', { name: 'Tạo nhạc' })).toBeInTheDocument()
    expect(screen.queryByRole('heading', { name: 'Users' })).not.toBeInTheDocument()
  })

  it('lets admins open /admin routes', async () => {
    renderApp('/admin/users', makeUser({ role: 'admin' }))
    expect(await screen.findByRole('heading', { name: 'Users' })).toBeInTheDocument()
  })
})
