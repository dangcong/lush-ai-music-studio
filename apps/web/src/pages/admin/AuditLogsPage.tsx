import { useQuery } from '@tanstack/react-query'
import { RefreshCw, ScrollText } from 'lucide-react'
import { api } from '../../api/api'
import { queryKeys } from '../../api/hooks'
import { EmptyState, ErrorState, LoadingState, PageHeader } from '../../components/ui'
import { formatDateTime } from '../../lib/format'

const AUDIT_LIMIT = 200

export function AuditLogsPage() {
  const logsQuery = useQuery({
    queryKey: queryKeys.auditLogs,
    queryFn: () => api.auditLogs.list(AUDIT_LIMIT),
  })

  return (
    <div className="page">
      <PageHeader
        title="Audit Logs"
        description={`${AUDIT_LIMIT} thao tác quản trị gần nhất. Chỉ đọc.`}
        actions={
          <button type="button" className="btn" onClick={() => void logsQuery.refetch()}>
            <RefreshCw size={16} aria-hidden className={logsQuery.isFetching ? 'spin' : undefined} />
            Làm mới
          </button>
        }
      />

      {logsQuery.isPending ? (
        <LoadingState />
      ) : logsQuery.isError ? (
        <ErrorState error={logsQuery.error} onRetry={() => void logsQuery.refetch()} />
      ) : logsQuery.data.length === 0 ? (
        <EmptyState icon={<ScrollText size={26} aria-hidden />} title="Chưa có bản ghi nào" />
      ) : (
        <div className="table-wrap">
          <table className="table">
            <thead>
              <tr>
                <th scope="col">Thời gian</th>
                <th scope="col">Người thực hiện</th>
                <th scope="col">Action</th>
                <th scope="col">Đối tượng</th>
                <th scope="col">Chi tiết</th>
              </tr>
            </thead>
            <tbody>
              {logsQuery.data.map((log) => {
                const hasDetail = log.detail !== null && Object.keys(log.detail).length > 0
                return (
                  <tr key={log.id}>
                    <td className="nowrap">{formatDateTime(log.created_at)}</td>
                    <td>{log.actor_name ?? '—'}</td>
                    <td>
                      <span className="mono">{log.action}</span>
                    </td>
                    <td className="cell-title">
                      <span>{log.target_type ?? '—'}</span>
                      {log.target_id && <small className="mono">{log.target_id}</small>}
                    </td>
                    <td>{hasDetail ? <code className="detail-json">{JSON.stringify(log.detail)}</code> : '—'}</td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  )
}
