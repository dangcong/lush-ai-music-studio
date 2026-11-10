import type { ReactNode } from 'react'
import { CircleAlert, LoaderCircle } from 'lucide-react'
import { errorMessage } from '../api/client'
import type { JobStatus } from '../api/types'
import { cx } from '../lib/cx'

export function Spinner({ size = 18, label = 'Đang tải' }: { size?: number; label?: string }) {
  return <LoaderCircle className="spin" size={size} role="status" aria-label={label} />
}

export function LoadingState({ label = 'Đang tải…' }: { label?: string }) {
  return (
    <div className="state">
      <Spinner size={22} label={label} />
      <p>{label}</p>
    </div>
  )
}

export function EmptyState({
  icon,
  title,
  children,
}: {
  icon?: ReactNode
  title: string
  children?: ReactNode
}) {
  return (
    <div className="state">
      {icon && <div className="state-icon">{icon}</div>}
      <p className="state-title">{title}</p>
      {children && <div className="state-text">{children}</div>}
    </div>
  )
}

export function ErrorState({ error, onRetry }: { error: unknown; onRetry?: () => void }) {
  return (
    <div className="state state-error" role="alert">
      <div className="state-icon">
        <CircleAlert size={22} aria-hidden />
      </div>
      <p className="state-title">Không tải được dữ liệu</p>
      <p className="state-text">{errorMessage(error)}</p>
      {onRetry && (
        <button type="button" className="btn btn-sm" onClick={onRetry}>
          Thử lại
        </button>
      )}
    </div>
  )
}

/** Label wraps the control (implicit association); the hint stays outside the label. */
export function Field({
  label,
  hint,
  aside,
  className,
  children,
}: {
  label: string
  hint?: ReactNode
  aside?: ReactNode
  className?: string
  children: ReactNode
}) {
  return (
    <div className={cx('field', className)}>
      <label>
        <span className="field-label">{label}</span>
        {children}
      </label>
      {aside && <span className="field-aside">{aside}</span>}
      {hint && <p className="field-hint">{hint}</p>}
    </div>
  )
}

export function Switch({
  checked,
  onChange,
  label,
  disabled,
  hideLabel,
}: {
  checked: boolean
  onChange: (checked: boolean) => void
  label: string
  disabled?: boolean
  hideLabel?: boolean
}) {
  return (
    <label className={cx('switch', disabled && 'is-disabled')}>
      <input
        type="checkbox"
        role="switch"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span className="switch-track" aria-hidden />
      <span className={hideLabel ? 'sr-only' : 'switch-label'}>{label}</span>
    </label>
  )
}

export const JOB_STATUS_LABELS: Record<JobStatus, string> = {
  queued: 'Đang chờ',
  preparing: 'Chuẩn bị',
  running: 'Đang chạy',
  completed: 'Hoàn tất',
  failed: 'Lỗi',
  cancelled: 'Đã hủy',
}

export function StatusBadge({ status }: { status: JobStatus }) {
  return <span className={cx('badge', `badge-${status}`)}>{JOB_STATUS_LABELS[status]}</span>
}

export function Badge({
  tone = 'neutral',
  children,
}: {
  tone?: 'neutral' | 'ok' | 'warn' | 'danger' | 'accent'
  children: ReactNode
}) {
  return <span className={cx('badge', `tone-${tone}`)}>{children}</span>
}

export function ProgressBar({ percent, label }: { percent: number; label: string }) {
  return (
    <div
      className="progress"
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent}
    >
      <div className="progress-fill" style={{ width: `${percent}%` }} />
    </div>
  )
}

export function PageHeader({
  title,
  description,
  actions,
}: {
  title: string
  description?: ReactNode
  actions?: ReactNode
}) {
  return (
    <header className="page-header">
      <div>
        <h1>{title}</h1>
        {description && <p className="page-desc">{description}</p>}
      </div>
      {actions && <div className="page-actions">{actions}</div>}
    </header>
  )
}

export function Notice({
  tone = 'info',
  children,
}: {
  tone?: 'info' | 'warn' | 'danger'
  children: ReactNode
}) {
  return (
    <div className={cx('notice', `notice-${tone}`)} role={tone === 'info' ? 'note' : 'alert'}>
      {children}
    </div>
  )
}
