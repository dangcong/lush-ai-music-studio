import { useId, useState } from 'react'
import { Plus } from 'lucide-react'
import { cx } from '../lib/cx'

interface ChipSelectProps {
  label: string
  /** Suggested values (always shown, toggled on/off). */
  suggestions?: string[]
  value: string[]
  onChange: (value: string[]) => void
  placeholder?: string
}

function normalize(text: string): string {
  return text.trim().toLowerCase()
}

/**
 * Toggleable chips plus a free-text "add" input. With no suggestions it behaves as a
 * plain tag editor: clicking a chip removes it.
 */
export function ChipSelect({ label, suggestions = [], value, onChange, placeholder }: ChipSelectProps) {
  const inputId = useId()
  const [draft, setDraft] = useState('')

  const selected = new Set(value.map(normalize))
  const options = [...suggestions]
  for (const v of value) {
    if (!options.some((o) => normalize(o) === normalize(v))) options.push(v)
  }

  const toggle = (option: string) => {
    if (selected.has(normalize(option))) {
      onChange(value.filter((v) => normalize(v) !== normalize(option)))
    } else {
      onChange([...value, option])
    }
  }

  const addDraft = () => {
    const parts = draft
      .split(',')
      .map((p) => p.trim())
      .filter(Boolean)
    if (parts.length === 0) return
    const next = [...value]
    for (const part of parts) {
      if (!next.some((v) => normalize(v) === normalize(part))) next.push(part)
    }
    onChange(next)
    setDraft('')
  }

  return (
    <div className="chip-select" role="group" aria-label={label}>
      <span className="field-label">{label}</span>
      <div className="chips">
        {options.map((option) => {
          const on = selected.has(normalize(option))
          return (
            <button
              key={normalize(option)}
              type="button"
              className={cx('chip', on && 'is-on')}
              aria-pressed={on}
              onClick={() => toggle(option)}
            >
              {option}
            </button>
          )
        })}
        <span className="chip-add">
          <label htmlFor={inputId} className="sr-only">
            Thêm {label}
          </label>
          <input
            id={inputId}
            value={draft}
            placeholder={placeholder ?? 'Thêm…'}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ',') {
                e.preventDefault()
                addDraft()
              }
            }}
            onBlur={addDraft}
          />
          <button
            type="button"
            className="icon-btn sm"
            aria-label={`Thêm vào ${label}`}
            disabled={!draft.trim()}
            onMouseDown={(e) => e.preventDefault()}
            onClick={addDraft}
          >
            <Plus size={14} aria-hidden />
          </button>
        </span>
      </div>
    </div>
  )
}
