import { useRef, useState } from 'react'

interface EditableCellProps {
  value: string | null
  /** Return a message to reject the value and stay in edit mode. */
  onSave: (next: string) => string | void | Promise<unknown>
  label: string
  empty?: string
  multiline?: boolean
  disabled?: boolean
  disabledTitle?: string
  error?: string | null
}

/** Click to edit; Enter or blur saves, Esc cancels. */
export function EditableCell({
  value,
  onSave,
  label,
  empty = 'Chưa điền',
  multiline = false,
  disabled = false,
  disabledTitle,
  error,
}: EditableCellProps) {
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState('')
  const [problem, setProblem] = useState<string | null>(null)
  const cancelled = useRef(false)

  const commit = () => {
    if (cancelled.current) return
    if (draft === (value ?? '')) {
      setEditing(false)
      setProblem(null)
      return
    }
    const result = onSave(draft)
    if (typeof result === 'string') {
      setProblem(result)
    } else if (result instanceof Promise) {
      // Stay in edit mode with the typed text until the save succeeds.
      setProblem(null)
      result.then(
        () => setEditing(false),
        () => {},
      )
    } else {
      setProblem(null)
      setEditing(false)
    }
  }

  const shown = problem ?? error
  if (!editing) {
    return (
      <div>
        <button
          type="button"
          className={`catalog-value catalog-editable${value ? '' : ' catalog-value-empty'}`}
          disabled={disabled}
          title={disabled ? disabledTitle : undefined}
          aria-label={`Sửa ${label}`}
          onClick={() => {
            cancelled.current = false
            setDraft(value ?? '')
            setProblem(null)
            setEditing(true)
          }}
        >
          {value ? value : empty}
        </button>
        {shown && <small className="catalog-error" role="alert">{shown}</small>}
      </div>
    )
  }

  const common = {
    className: 'catalog-input',
    autoFocus: true,
    value: draft,
    'aria-label': label,
    onChange: (e: { target: { value: string } }) => setDraft(e.target.value),
    onBlur: commit,
    onKeyDown: (e: React.KeyboardEvent) => {
      if (e.key === 'Escape') {
        cancelled.current = true
        setEditing(false)
        setProblem(null)
      } else if (e.key === 'Enter' && !(multiline && e.shiftKey)) {
        e.preventDefault()
        commit()
      }
    },
  }
  return (
    <div>
      {multiline ? <textarea rows={3} {...common} /> : <input type="text" {...common} />}
      {shown && <small className="catalog-error" role="alert">{shown}</small>}
    </div>
  )
}
