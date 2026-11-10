import { useState } from 'react'
import type { FormEvent } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Check, Copy, KeyRound, Plus, Power, Server, TriangleAlert } from 'lucide-react'
import { Link } from 'react-router-dom'
import { api } from '../../api/api'
import { queryKeys } from '../../api/hooks'
import type { NodeUpdateInput, RenderNode } from '../../api/types'
import { ConfirmDialog, Modal } from '../../components/Modal'
import {
  Badge,
  EmptyState,
  ErrorState,
  Field,
  LoadingState,
  Notice,
  PageHeader,
  Switch,
} from '../../components/ui'
import { cx } from '../../lib/cx'
import { formatDateTime, formatRelative } from '../../lib/format'
import { useAdminMutation } from './shared'

const NODES_POLL_MS = 10000

/** Shows a node token exactly once; it cannot be retrieved again after closing. */
function TokenModal({ nodeName, token, onClose }: { nodeName: string; token: string; onClose: () => void }) {
  const [copied, setCopied] = useState(false)
  const [copyFailed, setCopyFailed] = useState(false)

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(token)
      setCopied(true)
      setCopyFailed(false)
    } catch {
      setCopyFailed(true)
    }
  }

  return (
    <Modal
      title={`Token của node ${nodeName}`}
      onClose={onClose}
      footer={
        <button type="button" className="btn btn-primary" onClick={onClose}>
          Tôi đã lưu token
        </button>
      }
    >
      <div className="form-stack">
        <Notice tone="warn">
          <TriangleAlert size={16} aria-hidden />
          <span>
            Token chỉ hiển thị <strong>một lần duy nhất</strong>. Sau khi đóng hộp thoại này bạn sẽ không xem lại
            được — nếu làm mất, hãy dùng Rotate Token để cấp token mới.
          </span>
        </Notice>
        <div className="token-box">
          <code className="token-value" aria-label="Node token">
            {token}
          </code>
          <button type="button" className="btn btn-sm" onClick={() => void copy()}>
            {copied ? <Check size={14} aria-hidden /> : <Copy size={14} aria-hidden />}
            {copied ? 'Đã copy' : 'Copy'}
          </button>
        </div>
        {copyFailed && (
          <p className="field-hint">Trình duyệt không cho phép copy tự động — hãy bôi đen token và copy thủ công.</p>
        )}
        <p className="field-hint">
          Dán token vào cấu hình worker trên máy render (header <span className="mono">Authorization: Bearer …</span>).
        </p>
      </div>
    </Modal>
  )
}

function AddNodeModal({
  onClose,
  onCreated,
}: {
  onClose: () => void
  onCreated: (nodeName: string, token: string) => void
}) {
  const [name, setName] = useState('')
  const create = useAdminMutation((nodeName: string) => api.nodes.create(nodeName), {
    invalidate: queryKeys.nodes,
    onSuccess: (result) => onCreated(result.node.name, result.token),
  })

  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    if (name.trim()) create.mutate(name.trim())
  }

  return (
    <Modal
      title="Thêm Render Node"
      size="sm"
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            Hủy bỏ
          </button>
          <button type="submit" form="node-form" className="btn btn-primary" disabled={create.isPending || !name.trim()}>
            Tạo node
          </button>
        </>
      }
    >
      <form id="node-form" className="form-stack" onSubmit={onSubmit}>
        <Field label="Tên node" hint="Ví dụ: render-node-01">
          <input type="text" required value={name} onChange={(e) => setName(e.target.value)} />
        </Field>
      </form>
    </Modal>
  )
}

function Meter({ label, percent, text }: { label: string; percent: number; text: string }) {
  const clamped = Math.min(100, Math.max(0, percent))
  return (
    <div className="meter">
      <div className="meter-head">
        <span>{label}</span>
        <span className="mono">{text}</span>
      </div>
      <div
        className="progress"
        role="meter"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(clamped)}
      >
        <div className={cx('progress-fill', clamped >= 90 && 'is-hot')} style={{ width: `${clamped}%` }} />
      </div>
    </div>
  )
}

function formatGb(mb: number): string {
  return `${(mb / 1024).toFixed(1)} GB`
}

function GpuInfo({ gpu }: { gpu: RenderNode['gpu'] }) {
  const name = typeof gpu?.name === 'string' ? gpu.name : null
  const utilization = typeof gpu?.utilization === 'number' ? gpu.utilization : null
  const used = typeof gpu?.vram_used_mb === 'number' ? gpu.vram_used_mb : null
  const total = typeof gpu?.vram_total_mb === 'number' ? gpu.vram_total_mb : null

  if (name === null && utilization === null && used === null && total === null) {
    return <p className="muted">Chưa có thông tin GPU.</p>
  }
  return (
    <div className="gpu">
      {name && <p className="gpu-name">{name}</p>}
      {utilization !== null && (
        <Meter label="GPU Utilization" percent={utilization} text={`${Math.round(utilization)}%`} />
      )}
      {used !== null && total !== null && total > 0 ? (
        <Meter label="VRAM" percent={(used / total) * 100} text={`${formatGb(used)} / ${formatGb(total)}`} />
      ) : total !== null ? (
        <p className="node-line">
          <span>VRAM</span>
          <span className="mono">{formatGb(total)}</span>
        </p>
      ) : null}
    </div>
  )
}

function NodeCard({ node, onToken }: { node: RenderNode; onToken: (nodeName: string, token: string) => void }) {
  const [confirm, setConfirm] = useState<'rotate' | 'disable' | null>(null)
  const update = useAdminMutation((input: NodeUpdateInput) => api.nodes.update(node.id, input), {
    invalidate: queryKeys.nodes,
    onSuccess: () => setConfirm(null),
  })
  const rotate = useAdminMutation(() => api.nodes.rotateToken(node.id), {
    invalidate: queryKeys.nodes,
    onSuccess: (result) => {
      setConfirm(null)
      onToken(node.name, result.token)
    },
  })
  const disabled = node.status === 'disabled'

  return (
    <article className={cx('node-card', disabled && 'is-inactive')} aria-label={node.name}>
      <header className="node-head">
        <div>
          <h2 className="node-name">{node.name}</h2>
          <p className="node-sub">
            {node.last_seen ? `Last seen ${formatRelative(node.last_seen)}` : 'Chưa từng kết nối'}
          </p>
        </div>
        <div className="badge-row">
          <Badge tone={node.online ? 'ok' : 'neutral'}>
            <span className={cx('dot', node.online ? 'dot-on' : 'dot-off')} aria-hidden />
            {node.online ? 'Online' : 'Offline'}
          </Badge>
          {disabled && <Badge tone="danger">Disabled</Badge>}
          {node.maintenance && <Badge tone="warn">Maintenance</Badge>}
        </div>
      </header>

      <GpuInfo gpu={node.gpu} />

      <dl className="node-facts">
        <div className="node-line">
          <dt>Worker version</dt>
          <dd className="mono">
            {node.worker_version ?? '—'}
            {node.protocol_version !== null ? ` · protocol ${node.protocol_version}` : ''}
          </dd>
        </div>
        <div className="node-line">
          <dt>Job hiện tại</dt>
          <dd>
            {node.current_job_id ? (
              <Link to="/queue" className="mono" title={node.current_job_id}>
                {node.current_job_id.slice(0, 8)}…
              </Link>
            ) : (
              'Đang rảnh'
            )}
          </dd>
        </div>
        <div className="node-line">
          <dt>Đăng ký lần cuối</dt>
          <dd>{formatDateTime(node.registered_at)}</dd>
        </div>
        <div className="node-line stacked">
          <dt>Engines</dt>
          <dd className="badge-row">
            {node.engines.length === 0
              ? '—'
              : node.engines.map((engine, i) => (
                  <Badge key={engine.name ?? i}>
                    {engine.name ?? 'engine'}
                    {engine.version ? ` ${engine.version}` : ''}
                  </Badge>
                ))}
          </dd>
        </div>
        <div className="node-line stacked">
          <dt>Models đã cài</dt>
          <dd className="badge-row">
            {node.models.length === 0 ? '—' : node.models.map((m) => <Badge key={m.id}>{m.key}</Badge>)}
          </dd>
        </div>
        <div className="node-line stacked">
          <dt>LoRA đã cài</dt>
          <dd className="badge-row">
            {node.loras.length === 0 ? '—' : node.loras.map((l) => <Badge key={l.id}>{l.key}</Badge>)}
          </dd>
        </div>
      </dl>

      <footer className="node-actions">
        <Switch
          label="Maintenance"
          checked={node.maintenance}
          disabled={update.isPending || disabled}
          onChange={(maintenance) => update.mutate({ maintenance })}
        />
        <div className="node-buttons">
          <button type="button" className="btn btn-sm" onClick={() => setConfirm('rotate')}>
            <KeyRound size={14} aria-hidden />
            Rotate Token
          </button>
          {disabled ? (
            <button
              type="button"
              className="btn btn-sm"
              disabled={update.isPending}
              onClick={() => update.mutate({ status: 'active' })}
            >
              <Power size={14} aria-hidden />
              Enable
            </button>
          ) : (
            <button type="button" className="btn btn-sm btn-danger-ghost" onClick={() => setConfirm('disable')}>
              <Power size={14} aria-hidden />
              Disable
            </button>
          )}
        </div>
      </footer>

      {confirm === 'rotate' && (
        <ConfirmDialog
          title="Rotate token?"
          message={
            <>
              Token hiện tại của <strong>{node.name}</strong> sẽ mất hiệu lực ngay. Worker trên máy render phải được
              cấu hình lại bằng token mới thì mới kết nối tiếp được.
            </>
          }
          confirmLabel="Rotate Token"
          danger
          busy={rotate.isPending}
          onConfirm={() => rotate.mutate(undefined)}
          onClose={() => setConfirm(null)}
        />
      )}
      {confirm === 'disable' && (
        <ConfirmDialog
          title="Disable node?"
          message={
            <>
              <strong>{node.name}</strong> sẽ bị từ chối mọi kết nối và không nhận job mới cho tới khi được bật lại.
            </>
          }
          confirmLabel="Disable"
          danger
          busy={update.isPending}
          onConfirm={() => update.mutate({ status: 'disabled' })}
          onClose={() => setConfirm(null)}
        />
      )}
    </article>
  )
}

export function NodesPage() {
  // Node health changes on its own, so keep it fresh while this page is open.
  const nodesQuery = useQuery({
    queryKey: queryKeys.nodes,
    queryFn: api.nodes.list,
    refetchInterval: NODES_POLL_MS,
  })
  const [adding, setAdding] = useState(false)
  const [token, setToken] = useState<{ nodeName: string; token: string } | null>(null)
  const showToken = (nodeName: string, value: string) => setToken({ nodeName, token: value })

  return (
    <div className="page">
      <PageHeader
        title="Render Nodes"
        description="Các máy GPU nhận job từ hàng đợi. Node tự kết nối tới hệ thống bằng token."
        actions={
          <button type="button" className="btn btn-primary" onClick={() => setAdding(true)}>
            <Plus size={16} aria-hidden />
            Thêm node
          </button>
        }
      />

      {nodesQuery.isPending ? (
        <LoadingState />
      ) : nodesQuery.isError ? (
        <ErrorState error={nodesQuery.error} onRetry={() => void nodesQuery.refetch()} />
      ) : nodesQuery.data.length === 0 ? (
        <EmptyState icon={<Server size={26} aria-hidden />} title="Chưa có Render Node nào">
          Thêm node để lấy token, rồi cấu hình worker trên máy render.
        </EmptyState>
      ) : (
        <div className="node-grid">
          {nodesQuery.data.map((node) => (
            <NodeCard key={node.id} node={node} onToken={showToken} />
          ))}
        </div>
      )}

      {adding && (
        <AddNodeModal
          onClose={() => setAdding(false)}
          onCreated={(nodeName, value) => {
            setAdding(false)
            showToken(nodeName, value)
          }}
        />
      )}
      {token && <TokenModal nodeName={token.nodeName} token={token.token} onClose={() => setToken(null)} />}
    </div>
  )
}
