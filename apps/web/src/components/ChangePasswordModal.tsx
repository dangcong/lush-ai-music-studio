import { useState } from 'react'
import type { FormEvent } from 'react'
import { api } from '../api/api'
import { errorMessage } from '../api/client'
import { Modal } from './Modal'
import { useToast } from './Toast'
import { Field, Notice } from './ui'

export const PASSWORD_MIN = 8

export function ChangePasswordModal({ onClose }: { onClose: () => void }) {
  const toast = useToast()
  const [current, setCurrent] = useState('')
  const [next, setNext] = useState('')
  const [confirm, setConfirm] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const onSubmit = async (e: FormEvent) => {
    e.preventDefault()
    if (next.length < PASSWORD_MIN) return setError(`Mật khẩu mới cần ít nhất ${PASSWORD_MIN} ký tự.`)
    if (next !== confirm) return setError('Mật khẩu nhập lại không khớp.')
    setBusy(true)
    setError(null)
    try {
      await api.auth.changePassword(current, next)
      toast.success('Đã đổi mật khẩu.')
      onClose()
    } catch (err) {
      setError(errorMessage(err))
      setBusy(false)
    }
  }

  return (
    <Modal
      title="Đổi mật khẩu"
      size="sm"
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            Hủy bỏ
          </button>
          <button type="submit" form="change-password-form" className="btn btn-primary" disabled={busy}>
            Đổi mật khẩu
          </button>
        </>
      }
    >
      <form id="change-password-form" className="form-stack" onSubmit={onSubmit}>
        <Field label="Mật khẩu hiện tại">
          <input
            type="password"
            autoComplete="current-password"
            required
            value={current}
            onChange={(e) => setCurrent(e.target.value)}
          />
        </Field>
        <Field label="Mật khẩu mới" hint={`Ít nhất ${PASSWORD_MIN} ký tự.`}>
          <input
            type="password"
            autoComplete="new-password"
            required
            value={next}
            onChange={(e) => setNext(e.target.value)}
          />
        </Field>
        <Field label="Nhập lại mật khẩu mới">
          <input
            type="password"
            autoComplete="new-password"
            required
            value={confirm}
            onChange={(e) => setConfirm(e.target.value)}
          />
        </Field>
        {error && (
          <Notice tone="danger">
            <span>{error}</span>
          </Notice>
        )}
      </form>
    </Modal>
  )
}
