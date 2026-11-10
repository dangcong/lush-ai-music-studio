import { useState } from 'react'
import type { FormEvent } from 'react'
import { useQuery } from '@tanstack/react-query'
import { KeyRound, Lock, LockOpen, Plus } from 'lucide-react'
import { api } from '../../api/api'
import { queryKeys } from '../../api/hooks'
import { ROLES } from '../../api/types'
import type { Role, User, UserCreateInput, UserUpdateInput } from '../../api/types'
import { useCurrentUser } from '../../auth/AuthContext'
import { PASSWORD_MIN } from '../../components/ChangePasswordModal'
import { Modal } from '../../components/Modal'
import { ROLE_LABELS } from '../../components/Sidebar'
import { Badge, ErrorState, Field, LoadingState, Notice, PageHeader } from '../../components/ui'
import { formatDate } from '../../lib/format'
import { useAdminMutation } from './shared'

function AddUserModal({ onClose }: { onClose: () => void }) {
  const [username, setUsername] = useState('')
  const [displayName, setDisplayName] = useState('')
  const [password, setPassword] = useState('')
  const [role, setRole] = useState<Role>('member')
  const create = useAdminMutation((input: UserCreateInput) => api.users.create(input), {
    invalidate: queryKeys.users,
    successMessage: 'Đã thêm người dùng.',
    onSuccess: onClose,
  })

  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    create.mutate({
      username: username.trim(),
      password,
      display_name: displayName.trim() || undefined,
      role,
    })
  }

  return (
    <Modal
      title="Thêm người dùng"
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            Hủy bỏ
          </button>
          <button type="submit" form="user-form" className="btn btn-primary" disabled={create.isPending}>
            Thêm
          </button>
        </>
      }
    >
      <form id="user-form" className="form-stack" onSubmit={onSubmit}>
        <div className="field-row">
          <Field label="Tên đăng nhập">
            <input
              type="text"
              required
              autoComplete="off"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
            />
          </Field>
          <Field label="Tên hiển thị">
            <input type="text" value={displayName} onChange={(e) => setDisplayName(e.target.value)} />
          </Field>
        </div>
        <div className="field-row">
          <Field label="Mật khẩu" hint={`Ít nhất ${PASSWORD_MIN} ký tự.`}>
            <input
              type="password"
              required
              minLength={PASSWORD_MIN}
              autoComplete="new-password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
            />
          </Field>
          <Field label="Role">
            <select value={role} onChange={(e) => setRole(e.target.value as Role)}>
              {ROLES.map((r) => (
                <option key={r} value={r}>
                  {ROLE_LABELS[r]}
                </option>
              ))}
            </select>
          </Field>
        </div>
      </form>
    </Modal>
  )
}

function ResetPasswordModal({ user, onClose }: { user: User; onClose: () => void }) {
  const [password, setPassword] = useState('')
  const [confirm, setConfirm] = useState('')
  const [error, setError] = useState<string | null>(null)
  const reset = useAdminMutation((value: string) => api.users.update(user.id, { password: value }), {
    invalidate: queryKeys.users,
    successMessage: `Đã đặt lại mật khẩu cho ${user.username}.`,
    onSuccess: onClose,
  })

  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    if (password.length < PASSWORD_MIN) return setError(`Mật khẩu cần ít nhất ${PASSWORD_MIN} ký tự.`)
    if (password !== confirm) return setError('Mật khẩu nhập lại không khớp.')
    setError(null)
    reset.mutate(password)
  }

  return (
    <Modal
      title={`Đặt lại mật khẩu: ${user.username}`}
      size="sm"
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            Hủy bỏ
          </button>
          <button type="submit" form="reset-form" className="btn btn-primary" disabled={reset.isPending}>
            Đặt lại
          </button>
        </>
      }
    >
      <form id="reset-form" className="form-stack" onSubmit={onSubmit}>
        <Field label="Mật khẩu mới">
          <input
            type="password"
            required
            autoComplete="new-password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
          />
        </Field>
        <Field label="Nhập lại mật khẩu mới">
          <input
            type="password"
            required
            autoComplete="new-password"
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

function UserRow({ user, isSelf, onReset }: { user: User; isSelf: boolean; onReset: (user: User) => void }) {
  const update = useAdminMutation((input: UserUpdateInput) => api.users.update(user.id, input), {
    invalidate: queryKeys.users,
  })
  return (
    <tr className={user.is_active ? undefined : 'is-inactive'}>
      <td className="cell-title">
        <span>
          {user.display_name || user.username}
          {isSelf && <span className="muted"> (bạn)</span>}
        </span>
        <small className="mono">{user.username}</small>
      </td>
      <td>
        <select
          aria-label={`Role của ${user.username}`}
          value={user.role}
          // Changing your own role or locking yourself could leave the studio without an admin.
          disabled={isSelf || update.isPending}
          onChange={(e) => update.mutate({ role: e.target.value as Role })}
        >
          {ROLES.map((r) => (
            <option key={r} value={r}>
              {ROLE_LABELS[r]}
            </option>
          ))}
        </select>
      </td>
      <td>{user.is_active ? <Badge tone="ok">Hoạt động</Badge> : <Badge tone="danger">Đã khóa</Badge>}</td>
      <td>{formatDate(user.created_at)}</td>
      <td className="cell-actions">
        <button type="button" className="btn btn-sm" onClick={() => onReset(user)}>
          <KeyRound size={14} aria-hidden />
          Đặt lại mật khẩu
        </button>
        <button
          type="button"
          className="btn btn-sm"
          disabled={isSelf || update.isPending}
          onClick={() => update.mutate({ is_active: !user.is_active })}
        >
          {user.is_active ? <Lock size={14} aria-hidden /> : <LockOpen size={14} aria-hidden />}
          {user.is_active ? 'Khóa' : 'Mở khóa'}
        </button>
      </td>
    </tr>
  )
}

export function UsersPage() {
  const me = useCurrentUser()
  const usersQuery = useQuery({ queryKey: queryKeys.users, queryFn: api.users.list })
  const [adding, setAdding] = useState(false)
  const [resetting, setResetting] = useState<User | null>(null)

  return (
    <div className="page">
      <PageHeader
        title="Users"
        description="Tài khoản nhân viên và phân quyền."
        actions={
          <button type="button" className="btn btn-primary" onClick={() => setAdding(true)}>
            <Plus size={16} aria-hidden />
            Thêm người dùng
          </button>
        }
      />

      {usersQuery.isPending ? (
        <LoadingState />
      ) : usersQuery.isError ? (
        <ErrorState error={usersQuery.error} onRetry={() => void usersQuery.refetch()} />
      ) : (
        <div className="table-wrap">
          <table className="table">
            <thead>
              <tr>
                <th scope="col">Người dùng</th>
                <th scope="col">Role</th>
                <th scope="col">Trạng thái</th>
                <th scope="col">Ngày tạo</th>
                <th scope="col">
                  <span className="sr-only">Thao tác</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {usersQuery.data.map((user) => (
                <UserRow key={user.id} user={user} isSelf={user.id === me.id} onReset={setResetting} />
              ))}
            </tbody>
          </table>
        </div>
      )}

      {adding && <AddUserModal onClose={() => setAdding(false)} />}
      {resetting && <ResetPasswordModal user={resetting} onClose={() => setResetting(null)} />}
    </div>
  )
}
