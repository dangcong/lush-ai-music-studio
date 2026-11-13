import { useId, useRef, useState } from 'react'
import type { DragEvent, KeyboardEvent, ReactNode } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { AudioLines, Upload, X } from 'lucide-react'
import { api } from '../../api/api'
import { artifactUrl, errorMessage } from '../../api/client'
import type { AudioInput } from '../../api/types'
import { Spinner } from '../../components/ui'
import { cx } from '../../lib/cx'
import { formatBytes } from '../../lib/format'
import { AUDIO_INPUT_EXTENSIONS, AUDIO_INPUT_MAX_MB } from './formState'

const FORMATS_TEXT = 'WAV, MP3, FLAC'

/** Client-side check before uploading; the API validates again (extension, signature, size). */
export function checkAudioFile(file: Pick<File, 'name' | 'size'>): string | null {
  const name = file.name.toLowerCase()
  if (!AUDIO_INPUT_EXTENSIONS.some((ext) => name.endsWith(ext))) {
    return `Chỉ nhận file ${FORMATS_TEXT}.`
  }
  if (file.size === 0) return 'File rỗng.'
  if (file.size > AUDIO_INPUT_MAX_MB * 1024 * 1024) {
    return `File quá lớn (${formatBytes(file.size)}). Tối đa ${AUDIO_INPUT_MAX_MB} MB.`
  }
  return null
}

const audioInputKey = (id: string) => ['audio-inputs', id] as const

interface AudioDropzoneProps {
  /** "Reference Audio" or "Source Audio": also used in the accessible names. */
  label: string
  hint?: ReactNode
  /** Id of the uploaded file, '' when none. */
  value: string
  onChange: (id: string) => void
}

/** Uploads one audio file as soon as it is chosen and previews the uploaded copy. */
export function AudioDropzone({ label, hint, value, onChange }: AudioDropzoneProps) {
  const queryClient = useQueryClient()
  const labelId = useId()
  const inputRef = useRef<HTMLInputElement>(null)
  const [dragOver, setDragOver] = useState(false)
  const [uploadingName, setUploadingName] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  // Only the id is known when the form was restored from an earlier request.
  const metaQuery = useQuery({
    queryKey: audioInputKey(value),
    queryFn: () => api.audioInputs.get(value),
    enabled: value !== '',
    staleTime: Infinity,
    retry: false,
  })

  const upload = async (file: File | undefined) => {
    if (!file || uploadingName !== null) return
    const problem = checkAudioFile(file)
    setError(problem)
    if (problem) return
    setUploadingName(file.name)
    try {
      const uploaded: AudioInput = await api.audioInputs.upload(file)
      queryClient.setQueryData(audioInputKey(uploaded.id), uploaded)
      onChange(uploaded.id)
    } catch (e) {
      setError(errorMessage(e))
    } finally {
      setUploadingName(null)
    }
  }

  const onDrop = (e: DragEvent) => {
    e.preventDefault()
    setDragOver(false)
    void upload(e.dataTransfer.files[0])
  }

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      inputRef.current?.click()
    }
  }

  const meta = metaQuery.data

  return (
    <div className="field audio-input">
      <span className="field-label" id={labelId}>
        {label}
      </span>

      {value ? (
        <div className="dropzone-file">
          <div className="dropzone-file-head">
            <AudioLines size={18} aria-hidden />
            <span className="dropzone-file-name">
              {meta?.filename ?? (metaQuery.isError ? 'File đã tải lên' : 'Đang tải thông tin file…')}
            </span>
            {meta && <span className="muted">{formatBytes(meta.size_bytes)}</span>}
            <button
              type="button"
              className="icon-btn sm"
              aria-label={`Bỏ file ${label}`}
              title="Bỏ file này để chọn file khác"
              onClick={() => {
                setError(null)
                onChange('')
              }}
            >
              <X size={15} aria-hidden />
            </button>
          </div>
          <audio
            className="dropzone-audio"
            controls
            preload="none"
            src={artifactUrl(value)}
            aria-label={`Nghe thử ${label}`}
          />
        </div>
      ) : (
        <div
          className={cx('dropzone', dragOver && 'is-over', uploadingName !== null && 'is-busy', error && 'is-error')}
          role="button"
          tabIndex={0}
          aria-labelledby={labelId}
          aria-busy={uploadingName !== null}
          onClick={() => inputRef.current?.click()}
          onKeyDown={onKeyDown}
          onDragOver={(e) => {
            e.preventDefault()
            setDragOver(true)
          }}
          onDragLeave={() => setDragOver(false)}
          onDrop={onDrop}
        >
          {uploadingName !== null ? (
            <>
              <Spinner size={20} label="Đang tải lên" />
              <span className="dropzone-text">Đang tải lên {uploadingName}…</span>
            </>
          ) : (
            <>
              <Upload size={20} aria-hidden />
              <span className="dropzone-text">Kéo thả file vào đây hoặc bấm để chọn</span>
              <span className="dropzone-sub">
                {FORMATS_TEXT} · tối đa {AUDIO_INPUT_MAX_MB} MB
              </span>
            </>
          )}
          <input
            ref={inputRef}
            type="file"
            className="sr-only"
            tabIndex={-1}
            accept={AUDIO_INPUT_EXTENSIONS.join(',')}
            aria-label={`Chọn file ${label}`}
            disabled={uploadingName !== null}
            onClick={(e) => e.stopPropagation()}
            onChange={(e) => {
              const file = e.target.files?.[0]
              e.target.value = '' // choosing the same file again must fire a change
              void upload(file)
            }}
          />
        </div>
      )}

      {error && (
        <p className="dropzone-error" role="alert">
          {error}
        </p>
      )}
      {hint && <p className="field-hint">{hint}</p>}
    </div>
  )
}
