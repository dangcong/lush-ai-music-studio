import { useState } from 'react'
import type { Model } from '../../api/types'

const NOT_INSTALLED = 'Chưa cài trên máy'
const NO_CAPTIONER = 'Chưa cài model mô tả trên máy render'

export const isInstalled = (m: Model) => m.installed_nodes.length > 0

interface SelectionBarProps {
  selectedCount: number
  captioners: Model[]
  modelKey: string
  onModelChange: (key: string) => void
  onSelectAll: () => void
  onSelectFirst: (n: number) => void
  onCreate: () => void
  onDelete: () => void
  busy: boolean
}

export function SelectionBar({
  selectedCount,
  captioners,
  modelKey,
  onModelChange,
  onSelectAll,
  onSelectFirst,
  onCreate,
  onDelete,
  busy,
}: SelectionBarProps) {
  const [firstN, setFirstN] = useState('10')
  const canCreate = selectedCount > 0 && captioners.some((m) => m.key === modelKey && isInstalled(m)) && !busy

  return (
    <div className="catalog-bar" role="toolbar" aria-label="Thanh chọn bài">
      <strong className="catalog-bar-count">Đã chọn {selectedCount} bài</strong>
      <button type="button" className="btn btn-sm" onClick={onSelectAll}>
        Chọn tất cả
      </button>
      <span className="catalog-bar-first">
        <input
          type="number"
          min={1}
          aria-label="Số bài đầu"
          value={firstN}
          onChange={(e) => setFirstN(e.target.value)}
        />
        <button
          type="button"
          className="btn btn-sm"
          onClick={() => {
            const n = Number.parseInt(firstN, 10)
            if (n > 0) onSelectFirst(n)
          }}
        >
          Chọn bài đầu
        </button>
      </span>
      <label className="catalog-bar-model">
        <span>Model tạo mô tả</span>
        <select
          value={captioners.length === 0 ? '' : modelKey}
          disabled={captioners.length === 0}
          onChange={(e) => onModelChange(e.target.value)}
        >
          {captioners.length === 0 && <option value="">{NO_CAPTIONER}</option>}
          {captioners.map((m) => (
            <option key={m.key} value={m.key} disabled={!isInstalled(m)}>
              {isInstalled(m) ? m.name : `${m.name} — ${NOT_INSTALLED}`}
            </option>
          ))}
        </select>
      </label>
      <button type="button" className="btn btn-primary" disabled={!canCreate} onClick={onCreate}>
        Tạo mô tả cho {selectedCount} bài
      </button>
      <button
        type="button"
        className="btn btn-danger-ghost catalog-bar-delete"
        disabled={selectedCount === 0 || busy}
        onClick={onDelete}
      >
        Xóa khỏi catalog
      </button>
    </div>
  )
}
