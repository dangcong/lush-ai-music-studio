import { Boxes } from 'lucide-react'
import { api } from '../../api/api'
import { useModels } from '../../api/hooks'
import type { Model, ModelStatus } from '../../api/types'
import { Badge, EmptyState, ErrorState, LoadingState, PageHeader, Switch } from '../../components/ui'
import { cx } from '../../lib/cx'
import { useAdminMutation } from './shared'

function ModelRow({ model }: { model: Model }) {
  const toggle = useAdminMutation((status: ModelStatus) => api.models.update(model.id, { status }), {
    invalidate: ['models'],
  })
  const active = model.status === 'active'
  return (
    <tr className={active ? undefined : 'is-inactive'}>
      <td className="cell-title">
        <span>
          {model.name}
          {model.version ? <span className="muted"> · {model.version}</span> : null}
        </span>
        {model.description && <small>{model.description}</small>}
      </td>
      <td>
        <span className="mono">
          {model.engine} / {model.key}
        </span>
      </td>
      <td>
        <div className="badge-row">
          {model.capabilities.length === 0 ? '—' : model.capabilities.map((c) => <Badge key={c}>{c}</Badge>)}
        </div>
      </td>
      <td>
        {model.installed_nodes.length === 0 ? (
          <span className="muted">Chưa node nào cài</span>
        ) : (
          <ul className="node-list">
            {model.installed_nodes.map((n) => (
              <li key={n.node_id}>
                <span className={cx('dot', n.online ? 'dot-on' : 'dot-off')} aria-hidden />
                {n.node_name}
                <span className="sr-only">{n.online ? ' (online)' : ' (offline)'}</span>
              </li>
            ))}
          </ul>
        )}
      </td>
      <td className="cell-actions">
        <Switch
          label={active ? 'Đang bật' : 'Đang tắt'}
          checked={active}
          disabled={toggle.isPending}
          onChange={(v) => toggle.mutate(v ? 'active' : 'disabled')}
        />
      </td>
    </tr>
  )
}

export function ModelsPage() {
  const modelsQuery = useModels()

  return (
    <div className="page">
      <PageHeader
        title="Models"
        description="Model được Render Node tự đăng ký khi khởi động. Model bị tắt sẽ không chọn được ở trang Tạo nhạc."
      />
      {modelsQuery.isPending ? (
        <LoadingState />
      ) : modelsQuery.isError ? (
        <ErrorState error={modelsQuery.error} onRetry={() => void modelsQuery.refetch()} />
      ) : modelsQuery.data.length === 0 ? (
        <EmptyState icon={<Boxes size={26} aria-hidden />} title="Chưa có model nào">
          Model sẽ xuất hiện sau khi một Render Node đăng ký với hệ thống.
        </EmptyState>
      ) : (
        <div className="table-wrap">
          <table className="table">
            <thead>
              <tr>
                <th scope="col">Model</th>
                <th scope="col">Engine / Key</th>
                <th scope="col">Capabilities</th>
                <th scope="col">Node đã cài</th>
                <th scope="col">Trạng thái</th>
              </tr>
            </thead>
            <tbody>
              {modelsQuery.data.map((model) => (
                <ModelRow key={model.id} model={model} />
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  )
}
