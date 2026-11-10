import { useEffect, useRef, useState } from 'react'
import type { ReactNode } from 'react'
import {
  Boxes,
  ChevronDown,
  Cpu,
  Database,
  GraduationCap,
  KeyRound,
  Layers,
  Library,
  ListMusic,
  LogOut,
  ScrollText,
  Server,
  SlidersHorizontal,
  Sparkles,
  Tags,
  Users,
} from 'lucide-react'
import { NavLink } from 'react-router-dom'
import { useQueueSummary } from '../api/hooks'
import type { Role } from '../api/types'
import { useAuth, useCurrentUser } from '../auth/AuthContext'
import { cx } from '../lib/cx'
import { ChangePasswordModal } from './ChangePasswordModal'
import { Logo } from './Logo'

export const ROLE_LABELS: Record<Role, string> = {
  admin: 'Admin',
  trainer: 'Trainer',
  member: 'Member',
}

function NavItem({
  to,
  icon,
  children,
  badge,
}: {
  to: string
  icon: ReactNode
  children: ReactNode
  badge?: ReactNode
}) {
  return (
    <li>
      <NavLink to={to} className={({ isActive }) => cx('nav-item', isActive && 'is-active')}>
        {icon}
        <span className="nav-label">{children}</span>
        {badge}
      </NavLink>
    </li>
  )
}

function QueueIndicator() {
  const { data, isError } = useQueueSummary()
  if (isError && !data) {
    return (
      <div className="queue-indicator" role="status">
        <span className="dot dot-off" aria-hidden />
        Không lấy được trạng thái hàng đợi
      </div>
    )
  }
  if (!data) return null
  const running = data.preparing + data.running
  return (
    <div className="queue-indicator" role="status" aria-label="Trạng thái hàng đợi">
      <div className="queue-counts">
        <span>
          <strong>{data.queued}</strong> đang chờ
        </span>
        <span>
          <strong>{running}</strong> đang chạy
        </span>
      </div>
      <div className="queue-nodes">
        <span className={cx('dot', data.nodes_online > 0 ? 'dot-on' : 'dot-off')} aria-hidden />
        {data.nodes_online > 0 ? `${data.nodes_online} node online` : 'Không có node online'}
      </div>
    </div>
  )
}

function UserMenu() {
  const user = useCurrentUser()
  const { logout } = useAuth()
  const [open, setOpen] = useState(false)
  const [changingPassword, setChangingPassword] = useState(false)
  const rootRef = useRef<HTMLDivElement>(null)
  const name = user.display_name || user.username

  useEffect(() => {
    if (!open) return
    const onPointerDown = (e: PointerEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false)
    }
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false)
    }
    document.addEventListener('pointerdown', onPointerDown)
    document.addEventListener('keydown', onKeyDown)
    return () => {
      document.removeEventListener('pointerdown', onPointerDown)
      document.removeEventListener('keydown', onKeyDown)
    }
  }, [open])

  return (
    <div className="user-menu" ref={rootRef}>
      <button
        type="button"
        className="user-button"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      >
        <span className="avatar" aria-hidden>
          {name.charAt(0).toUpperCase()}
        </span>
        <span className="user-text">
          <span className="user-name">{name}</span>
          <span className="user-role">{ROLE_LABELS[user.role]}</span>
        </span>
        <ChevronDown size={16} aria-hidden />
      </button>
      {open && (
        <div className="menu" role="menu">
          <button
            type="button"
            role="menuitem"
            className="menu-item"
            onClick={() => {
              setOpen(false)
              setChangingPassword(true)
            }}
          >
            <KeyRound size={16} aria-hidden />
            Đổi mật khẩu
          </button>
          <button type="button" role="menuitem" className="menu-item" onClick={() => void logout()}>
            <LogOut size={16} aria-hidden />
            Đăng xuất
          </button>
        </div>
      )}
      {changingPassword && <ChangePasswordModal onClose={() => setChangingPassword(false)} />}
    </div>
  )
}

const soonBadge = <span className="soon">Sắp có</span>

export function Sidebar() {
  const user = useCurrentUser()
  const summary = useQueueSummary().data
  const pending = summary ? summary.queued + summary.preparing + summary.running : 0

  return (
    <aside className="sidebar">
      <div className="brand">
        <Logo />
        <div className="brand-text">
          <span className="brand-name">Lush</span>
          <span className="brand-sub">AI Music Studio</span>
        </div>
      </div>

      <nav className="nav" aria-label="Điều hướng chính">
        <ul>
          <NavItem to="/create" icon={<Sparkles size={18} aria-hidden />}>
            Tạo nhạc
          </NavItem>
          <NavItem to="/library" icon={<Library size={18} aria-hidden />}>
            Thư viện
          </NavItem>
          <NavItem
            to="/queue"
            icon={<ListMusic size={18} aria-hidden />}
            badge={pending > 0 ? <span className="count" aria-label={`${pending} job chưa xong`}>{pending}</span> : undefined}
          >
            Hàng đợi
          </NavItem>
        </ul>

        <h2 className="nav-heading">Huấn luyện</h2>
        <ul>
          <NavItem to="/datasets" icon={<Database size={18} aria-hidden />} badge={soonBadge}>
            Dataset
          </NavItem>
          <NavItem to="/training" icon={<GraduationCap size={18} aria-hidden />} badge={soonBadge}>
            Training
          </NavItem>
          <NavItem to="/lora-library" icon={<Layers size={18} aria-hidden />} badge={soonBadge}>
            LoRA Library
          </NavItem>
        </ul>

        {user.role === 'admin' && (
          <>
            <h2 className="nav-heading">Quản trị</h2>
            <ul>
              <NavItem to="/admin/genres" icon={<Tags size={18} aria-hidden />}>
                Genres
              </NavItem>
              <NavItem to="/admin/presets" icon={<SlidersHorizontal size={18} aria-hidden />}>
                Presets
              </NavItem>
              <NavItem to="/admin/models" icon={<Boxes size={18} aria-hidden />}>
                Models
              </NavItem>
              <NavItem to="/admin/nodes" icon={<Server size={18} aria-hidden />}>
                Render Nodes
              </NavItem>
              <NavItem to="/admin/users" icon={<Users size={18} aria-hidden />}>
                Users
              </NavItem>
              <NavItem to="/admin/audit-logs" icon={<ScrollText size={18} aria-hidden />}>
                Audit Logs
              </NavItem>
            </ul>
          </>
        )}
      </nav>

      <div className="sidebar-foot">
        <div className="queue-box">
          <span className="queue-box-title">
            <Cpu size={14} aria-hidden />
            Hàng đợi render
          </span>
          <QueueIndicator />
        </div>
        <UserMenu />
      </div>
    </aside>
  )
}
