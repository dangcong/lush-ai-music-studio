import { useEffect, useRef, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { api } from '../../api/api'
import { errorMessage } from '../../api/client'
import type { Job } from '../../api/types'
import { Spinner } from '../../components/ui'
import { PROMPT_MAX } from './formState'

/** Spec: docs/superpowers/specs/2026-11-24-reference-auto-prompt-design.md */

const POLL_MS = 2000
const DONE = new Set(['completed', 'failed', 'cancelled'])

interface Request {
  inputId: string
  fileName: string
  /** "replace": a "Viết lại" click, the result replaces the prompt (undo keeps the old one). */
  replace: boolean
}

export interface ReferencePromptApi {
  /** Start for a freshly uploaded file. */
  start: (inputId: string, fileName: string) => void
  /** The file was removed: forget any pending answer and hide the panel. */
  reset: () => void
  panel: React.ReactNode
}

const clamp = (text: string) => text.slice(0, PROMPT_MAX)

/**
 * Asks a captioner to write a prompt from the uploaded Reference Audio and fills it in. A prompt the user has
 * typed is never overwritten: the text is offered as a suggestion instead.
 */
export function useReferencePrompt(
  prompt: string,
  setPrompt: (text: string) => void,
  applyTempoKey: (bpm: number | null, key: string | null) => void,
): ReferencePromptApi {
  const [req, setReq] = useState<Request | null>(null)
  const [jobId, setJobId] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [filledFrom, setFilledFrom] = useState<string | null>(null)
  const [previous, setPrevious] = useState<string | null>(null)
  const [suggestion, setSuggestion] = useState<string | null>(null)
  const [measured, setMeasured] = useState<{ bpm: number | null; key: string | null } | null>(null)
  const handled = useRef<string | null>(null)
  // Only the latest request may act on its reply (a slow answer for an older file is dropped).
  const seq = useRef(0)
  // The text this hook last put into the prompt: while the prompt still equals it, the user has not typed.
  const autoText = useRef<string | null>(null)
  const promptRef = useRef(prompt)
  promptRef.current = prompt

  const fill = (text: string) => {
    const t = clamp(text)
    autoText.current = t
    setPrompt(t)
  }

  const reset = () => {
    seq.current++
    handled.current = null
    setReq(null)
    setJobId(null)
    setError(null)
    setSuggestion(null)
    setMeasured(null)
    setFilledFrom(null)
    setPrevious(null)
  }

  const ask = (next: Request) => {
    const mine = ++seq.current
    handled.current = null // a new request: its answer is handled even when the API hands back the same job
    setReq(next)
    setJobId(null)
    setError(null)
    setSuggestion(null)
    setMeasured(null)
    api.audioInputs.promptJob(next.inputId).then(
      (r) => {
        if (seq.current === mine) setJobId(r.job_id)
      },
      (e) => {
        if (seq.current === mine) setError(errorMessage(e))
      },
    )
  }

  const jobQuery = useQuery({
    queryKey: ['jobs', jobId],
    queryFn: () => api.jobs.get(jobId!),
    enabled: jobId !== null,
    refetchInterval: (q) => {
      const status = (q.state.data as Job | undefined)?.status
      return status && DONE.has(status) ? false : POLL_MS
    },
  })
  const job = jobQuery.data

  useEffect(() => {
    if (!job || !req || !DONE.has(job.status) || handled.current === job.id) return
    handled.current = job.id
    if (job.status !== 'completed') {
      setError(job.error || 'Máy render không trả lời.')
      return
    }
    const caption = String(job.result?.caption ?? '').trim()
    if (!caption) {
      setError('Model không viết ra chữ nào.')
      return
    }
    const bpm = typeof job.result?.tempo_bpm === 'number' ? (job.result.tempo_bpm as number) : null
    const key = typeof job.result?.key === 'string' ? (job.result.key as string) : null
    setMeasured(bpm !== null || key !== null ? { bpm, key } : null)
    setError(null)
    const current = promptRef.current
    const untouched = current.trim() === '' || current === autoText.current
    if (req.replace || untouched) {
      setPrevious(req.replace ? current : null)
      fill(caption)
      setFilledFrom(req.fileName)
    } else {
      setSuggestion(caption)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- fill only wraps setPrompt
  }, [job, req, setPrompt])

  useEffect(() => {
    // A failed poll is shown, but only while no answer has arrived (polling goes on and may still succeed).
    if (jobQuery.isError && !(job && DONE.has(job.status))) setError(errorMessage(jobQuery.error))
  }, [jobQuery.isError, jobQuery.error, job])

  const busy = req !== null && error === null && !(job && DONE.has(job.status))
  const fileName = req?.fileName ?? ''

  const panel = req ? (
    <div className="ref-prompt" aria-live="polite">
      {busy && (
        <p className="ref-prompt-badge">
          <Spinner size={14} label="" />{' '}
          {job?.status === 'running' || job?.status === 'preparing'
            ? `Đang nghe ${fileName} để viết prompt…`
            : 'Đang chờ máy render để viết prompt…'}
        </p>
      )}
      {error && (
        <p className="ref-prompt-error" role="alert">
          Không viết được prompt: {error}{' '}
          <button type="button" className="btn btn-sm" onClick={() => ask({ ...req, replace: false })}>
            Thử lại
          </button>
        </p>
      )}
      {suggestion && (
        <div className="ref-prompt-suggest">
          <p className="ref-prompt-title">Gợi ý từ {fileName}</p>
          <p>{suggestion}</p>
          <div className="ref-prompt-actions">
            <button
              type="button"
              className="btn btn-sm btn-primary"
              onClick={() => {
                setPrevious(promptRef.current)
                fill(suggestion)
                setFilledFrom(fileName)
                setSuggestion(null)
              }}
            >
              Thay prompt
            </button>
            <button
              type="button"
              className="btn btn-sm"
              onClick={() => {
                setPrompt(clamp(`${promptRef.current.trim()}, ${suggestion}`))
                setSuggestion(null)
              }}
            >
              Thêm vào cuối
            </button>
            <button type="button" className="btn btn-sm" onClick={() => setSuggestion(null)}>
              Bỏ
            </button>
          </div>
        </div>
      )}
      {filledFrom && !busy && (
        <>
          <p className="ref-prompt-badge">✓ Tự tạo từ {filledFrom} · sửa thoải mái</p>
          <div className="ref-prompt-actions">
            <button type="button" className="btn btn-sm" onClick={() => ask({ ...req, replace: true })}>
              Viết lại
            </button>
            {previous !== null && (
              <button
                type="button"
                className="btn btn-sm"
                onClick={() => {
                  setPrompt(previous)
                  setPrevious(null)
                }}
              >
                Hoàn tác
              </button>
            )}
            <button type="button" className="btn btn-sm" onClick={() => setPrompt('')}>
              Xóa prompt
            </button>
          </div>
        </>
      )}
      {measured && !busy && (
        <p className="ref-prompt-measured">
          Đo được: {measured.bpm !== null && <span className="chip">Tempo {Math.round(measured.bpm)}</span>}{' '}
          {measured.key && <span className="chip">Key {measured.key}</span>}{' '}
          <button type="button" className="btn btn-sm" onClick={() => applyTempoKey(measured.bpm, measured.key)}>
            Dùng Tempo/Key này
          </button>
        </p>
      )}
    </div>
  ) : null

  return { start: (inputId, name) => ask({ inputId, fileName: name, replace: false }), reset, panel }
}
