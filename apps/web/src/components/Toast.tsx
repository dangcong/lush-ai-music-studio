import { createContext, useCallback, useContext, useMemo, useRef, useState } from 'react'
import type { ReactNode } from 'react'
import { CircleAlert, Check, X } from 'lucide-react'
import { errorMessage } from '../api/client'

type ToastKind = 'success' | 'error'

interface ToastItem {
  id: number
  kind: ToastKind
  text: string
}

interface ToastApi {
  success: (text: string) => void
  /** Accepts a message or any thrown value (ApiError messages are shown as-is). */
  error: (error: unknown) => void
}

const noop: ToastApi = { success: () => {}, error: () => {} }
const ToastContext = createContext<ToastApi>(noop)

const TOAST_TTL_MS = 5000

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([])
  const nextId = useRef(1)

  const dismiss = useCallback((id: number) => {
    setItems((list) => list.filter((t) => t.id !== id))
  }, [])

  const push = useCallback(
    (kind: ToastKind, text: string) => {
      const id = nextId.current++
      setItems((list) => [...list.slice(-3), { id, kind, text }])
      window.setTimeout(() => dismiss(id), TOAST_TTL_MS)
    },
    [dismiss],
  )

  const value = useMemo<ToastApi>(
    () => ({
      success: (text) => push('success', text),
      error: (error) => push('error', typeof error === 'string' ? error : errorMessage(error)),
    }),
    [push],
  )

  return (
    <ToastContext.Provider value={value}>
      {children}
      <div className="toast-stack" aria-live="polite">
        {items.map((t) => (
          <div key={t.id} className={`toast toast-${t.kind}`} role={t.kind === 'error' ? 'alert' : 'status'}>
            {t.kind === 'error' ? <CircleAlert size={16} aria-hidden /> : <Check size={16} aria-hidden />}
            <span>{t.text}</span>
            <button type="button" className="icon-btn sm" aria-label="Đóng thông báo" onClick={() => dismiss(t.id)}>
              <X size={14} aria-hidden />
            </button>
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  )
}

export function useToast(): ToastApi {
  return useContext(ToastContext)
}
