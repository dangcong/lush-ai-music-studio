import { Layers } from 'lucide-react'
import { Link } from 'react-router-dom'
import { api } from '../api/api'
import { queryKeys, useLoras, useModels } from '../api/hooks'
import type { Lora, LoraStatus, LoraUpdateInput, Model } from '../api/types'
import { useCurrentUser } from '../auth/AuthContext'
import { Badge, EmptyState, ErrorState, LoadingState, Notice, PageHeader } from '../components/ui'
import { cx } from '../lib/cx'
import { formatDateTime, formatRelative } from '../lib/format'
import { useAdminMutation } from './admin/shared'

type Tone = 'neutral' | 'ok' | 'warn' | 'danger' | 'accent'

const STATUS: Record<LoraStatus, { label: string; tone: Tone }> = {
  training: { label: 'Đang train', tone: 'accent' },
  evaluation: { label: 'Đang đánh giá', tone: 'warn' },
  approved: { label: 'Đã duyệt', tone: 'ok' },
  archived: { label: 'Lưu trữ', tone: 'neutral' },
}

type TargetStatus = NonNullable<LoraUpdateInput['status']>

const ACTION_LABELS: Record<TargetStatus, string> = {
  approved: 'Duyệt',
  archived: 'Lưu trữ',
  evaluation: 'Đưa về đánh giá',
}

/** Status changes an admin may make from each status. */
const ACTIONS: Record<LoraStatus, TargetStatus[]> = {
  training: [],
  evaluation: ['approved', 'archived'],
  approved: ['evaluation', 'archived'],
  archived: ['evaluation'],
}

function LoraRow({ lora, models, isAdmin }: { lora: Lora; models: Model[]; isAdmin: boolean }) {
  const update = useAdminMutation((status: TargetStatus) => api.loras.update(lora.id, { status }), {
    invalidate: queryKeys.loras,
  })
  const status = STATUS[lora.status]
  const baseModel = models.find((m) => m.id === lora.base_model_id)
  const nodes = lora.installed_nodes ?? []

  return (
    <tr className={lora.status === 'archived' ? 'is-inactive' : undefined}>
      <td className="cell-title">
        <span>{lora.name}</span>
        {isAdmin && lora.job_id && (
          <small>
            <Link to={`/training/${lora.job_id}`}>Xem training run</Link>
          </small>
        )}
      </td>
      <td>
        <span className="mono">{lora.key}</span>
      </td>
      <td>{status ? <Badge tone={status.tone}>{status.label}</Badge> : <Badge>{lora.status}</Badge>}</td>
      <td>{baseModel?.name ?? <span className="mono">{lora.base_model_id}</span>}</td>
      <td>{lora.dataset_key ? <span className="mono">{lora.dataset_key}</span> : '—'}</td>
      <td title={formatDateTime(lora.created_at)}>{formatRelative(lora.created_at)}</td>
      <td>
        {nodes.length === 0 ? (
          <span className="muted">Chưa node nào cài</span>
        ) : (
          <ul className="node-list">
            {nodes.map((n) => (
              <li key={n.node_id}>
                <span className={cx('dot', n.online ? 'dot-on' : 'dot-off')} aria-hidden />
                {n.node_name}
                <span className="sr-only">{n.online ? ' (online)' : ' (offline)'}</span>
              </li>
            ))}
          </ul>
        )}
      </td>
      <td>
        {lora.adapter_sha256 ? (
          <span className="sha" title={lora.adapter_sha256}>
            {lora.adapter_sha256.slice(0, 12)}
          </span>
        ) : (
          '—'
        )}
      </td>
      {isAdmin && (
        <td className="cell-actions">
          {(ACTIONS[lora.status] ?? []).map((target) => (
            <button
              key={target}
              type="button"
              className={cx('btn btn-sm', target === 'approved' && 'btn-primary')}
              disabled={update.isPending}
              aria-label={`${ACTION_LABELS[target]} ${lora.name}`}
              onClick={() => update.mutate(target)}
            >
              {ACTION_LABELS[target]}
            </button>
          ))}
        </td>
      )}
    </tr>
  )
}

export function LoraLibraryPage() {
  const user = useCurrentUser()
  const isAdmin = user.role === 'admin'
  const lorasQuery = useLoras()
  const models = useModels().data ?? []

  return (
    <div className="page">
      <PageHeader
        title="Train Style Library"
        description="Các Train Style đã tạo. Train Style chỉ dùng được với đúng base model của nó."
      />
      <Notice tone="info">
        <span>
          Train Style train xong nằm ở đây với trạng thái <strong>Đang đánh giá</strong>. Bấm <strong>Duyệt</strong> thì nó
          trở thành một <strong>Style</strong> (Quản trị → Style); admin chọn genre cho Style đó, rồi mọi người chọn được
          nó ở trang Tạo nhạc.
        </span>
      </Notice>

      {lorasQuery.isPending ? (
        <LoadingState />
      ) : lorasQuery.isError ? (
        <ErrorState error={lorasQuery.error} onRetry={() => void lorasQuery.refetch()} />
      ) : lorasQuery.data.length === 0 ? (
        <EmptyState icon={<Layers size={26} aria-hidden />} title="Chưa có Train Style nào">
          {isAdmin ? 'Tạo một Train Style ở trang Train Style để nó xuất hiện tại đây.' : 'Chưa có Train Style nào.'}
        </EmptyState>
      ) : (
        <div className="table-wrap">
          <table className="table">
            <thead>
              <tr>
                <th scope="col">Tên</th>
                <th scope="col">Key</th>
                <th scope="col">Trạng thái</th>
                <th scope="col">Base model</th>
                <th scope="col">Dataset</th>
                <th scope="col">Tạo lúc</th>
                <th scope="col">Node đã cài</th>
                <th scope="col">Adapter SHA-256</th>
                {isAdmin && (
                  <th scope="col">
                    <span className="sr-only">Thao tác</span>
                  </th>
                )}
              </tr>
            </thead>
            <tbody>
              {lorasQuery.data.map((lora) => (
                <LoraRow key={lora.id} lora={lora} models={models} isAdmin={isAdmin} />
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  )
}
