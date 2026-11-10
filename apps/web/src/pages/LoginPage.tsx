import { useState } from 'react'
import type { FormEvent } from 'react'
import { Navigate, useLocation } from 'react-router-dom'
import { errorMessage } from '../api/client'
import { useAuth } from '../auth/AuthContext'
import { Logo } from '../components/Logo'
import { Field, Notice, Spinner } from '../components/ui'

export function LoginPage() {
  const { user, login } = useAuth()
  const location = useLocation()
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  if (user) {
    const from = (location.state as { from?: string } | null)?.from
    return <Navigate to={from && from !== '/login' ? from : '/create'} replace />
  }

  const onSubmit = async (e: FormEvent) => {
    e.preventDefault()
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      await login(username.trim(), password)
    } catch (err) {
      setError(errorMessage(err))
      setBusy(false)
    }
  }

  return (
    <main className="login-page">
      <form className="login-card" onSubmit={onSubmit}>
        <div className="login-brand">
          <Logo size={44} />
          <div>
            <h1>Lush AI Music Studio</h1>
            <p>Đăng nhập để bắt đầu tạo nhạc</p>
          </div>
        </div>
        <Field label="Tên đăng nhập">
          <input
            type="text"
            autoComplete="username"
            autoFocus
            required
            value={username}
            onChange={(e) => setUsername(e.target.value)}
          />
        </Field>
        <Field label="Mật khẩu">
          <input
            type="password"
            autoComplete="current-password"
            required
            value={password}
            onChange={(e) => setPassword(e.target.value)}
          />
        </Field>
        {error && (
          <Notice tone="danger">
            <span>{error}</span>
          </Notice>
        )}
        <button type="submit" className="btn btn-primary btn-lg" disabled={busy || !username || !password}>
          {busy && <Spinner size={18} label="Đang đăng nhập" />}
          Đăng nhập
        </button>
      </form>
    </main>
  )
}
