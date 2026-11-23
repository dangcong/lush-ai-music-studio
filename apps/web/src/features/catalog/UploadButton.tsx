import { useRef, useState } from 'react'
import type { ChangeEvent } from 'react'
import { Upload } from 'lucide-react'
import { api } from '../../api/api'
import { ApiError, errorMessage } from '../../api/client'
import { checkAudioFile } from '../create/AudioDropzone'
import { AUDIO_INPUT_EXTENSIONS } from '../create/formState'

const DUPLICATE_PREFIX = 'File này đã có trong catalog'

/** "<file>: đã có trong catalog (<tên cũ>)"; the API message ends with ": <existing name>". */
function duplicateMessage(file: string, apiMessage: string): string {
  const at = apiMessage.indexOf(DUPLICATE_PREFIX + ': ')
  const existing = at >= 0 ? apiMessage.slice(at + DUPLICATE_PREFIX.length + 2).trim() : ''
  return existing ? `${file}: đã có trong catalog (${existing})` : `${file}: đã có trong catalog`
}

interface UploadButtonProps {
  catalogId: string
  /** Called after the whole batch with one message per failed file. */
  onFinished: (errors: string[]) => void
}

/** Multi-file upload: one request per file, in order; a failed file does not stop the rest. */
export function UploadButton({ catalogId, onFinished }: UploadButtonProps) {
  const inputRef = useRef<HTMLInputElement>(null)
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null)

  const onChange = async (e: ChangeEvent<HTMLInputElement>) => {
    const files = Array.from(e.target.files ?? [])
    e.target.value = ''
    if (files.length === 0 || progress) return
    const errors: string[] = []
    setProgress({ done: 0, total: files.length })
    for (const [index, file] of files.entries()) {
      setProgress({ done: index + 1, total: files.length })
      const problem = checkAudioFile(file)
      if (problem) {
        errors.push(`${file.name}: ${problem}`)
        continue
      }
      try {
        await api.catalogs.upload(catalogId, file)
      } catch (error) {
        errors.push(
          error instanceof ApiError && error.status === 409
            ? duplicateMessage(file.name, error.message)
            : `${file.name}: ${errorMessage(error)}`,
        )
      }
    }
    setProgress(null)
    onFinished(errors)
  }

  return (
    <>
      <input
        ref={inputRef}
        type="file"
        multiple
        hidden
        accept={AUDIO_INPUT_EXTENSIONS.join(',')}
        data-testid="catalog-upload-input"
        onChange={(e) => void onChange(e)}
      />
      <button
        type="button"
        className="btn"
        disabled={progress !== null}
        onClick={() => inputRef.current?.click()}
      >
        <Upload size={16} aria-hidden />
        {progress ? `Đang tải ${progress.done}/${progress.total}` : 'Tải nhạc lên'}
      </button>
    </>
  )
}
