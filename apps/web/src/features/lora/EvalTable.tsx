import { useEffect, useState } from 'react'
import { Pause, Play } from 'lucide-react'
import { errorMessage } from '../../api/client'
import { useSaveCheckpointNote } from '../../api/mutations'
import type { Checkpoint, TrainingRunDetail } from '../../api/types'
import { Badge } from '../../components/ui'
import { cx } from '../../lib/cx'
import { usePlayer } from '../../player/PlayerContext'
import { checkpointVerdict, rendersFor } from './review'

const DASH = '—'

/** A checkpoint whose files are gone (or going) from the node: listenable, not choosable. */
export function isRemoved(c: Checkpoint): boolean {
  return c.status === 'deleted' || c.delete_requested_at !== null
}

export function epochLabel(c: Pick<Checkpoint, 'epoch'>): string {
  return `Epoch ${c.epoch ?? '?'}`
}

/** Play/pause one artifact in the shared player bar, with a short tag next to the button. */
export function AudioButton({ artifactId, title, tag }: { artifactId: string; title: string; tag: string }) {
  const player = usePlayer()
  const active = player.current?.artifactId === artifactId
  const playing = active && player.isPlaying
  const item = { artifactId, title }
  return (
    <div className={cx('eval-audio', active && 'is-active')}>
      <button
        type="button"
        className={cx('play-btn', active && 'is-active')}
        aria-label={playing ? `Tạm dừng ${title}` : `Phát ${title}`}
        onClick={(e) => {
          e.stopPropagation()
          player.toggle(item)
        }}
      >
        {playing ? <Pause size={14} aria-hidden /> : <Play size={14} aria-hidden />}
      </button>
      <span className="eval-audio-tag">{tag}</span>
    </div>
  )
}

function ScoreBar({ label, value, max, text }: { label: string; value: number | null; max: number; text: string }) {
  const percent = value === null ? 0 : Math.max(0, Math.min(100, (value / max) * 100))
  return (
    <div className="score">
      <div className="score-head">
        <span>{label}</span>
        <span className="score-value">{value === null ? DASH : text}</span>
      </div>
      <div className="score-track" aria-hidden>
        <div className="score-fill" style={{ width: `${percent}%` }} />
      </div>
    </div>
  )
}

/** Note field, saved on blur when changed. Follows the server value while not being edited. */
function NoteField({ checkpoint }: { checkpoint: Checkpoint }) {
  const save = useSaveCheckpointNote()
  const [value, setValue] = useState(checkpoint.note)
  const [editing, setEditing] = useState(false)
  useEffect(() => {
    if (!editing) setValue(checkpoint.note)
  }, [checkpoint.note, editing])
  return (
    <div className="eval-note">
      <textarea
        rows={2}
        maxLength={500}
        placeholder="Ghi chú…"
        aria-label={`Ghi chú ${epochLabel(checkpoint)}`}
        value={value}
        onClick={(e) => e.stopPropagation()}
        onFocus={() => setEditing(true)}
        onChange={(e) => setValue(e.target.value)}
        onBlur={() => {
          setEditing(false)
          if (value !== checkpoint.note) save.mutate({ id: checkpoint.id, note: value })
        }}
      />
      {save.isError && (
        <p className="job-error" role="alert">
          {errorMessage(save.error)}
        </p>
      )}
    </div>
  )
}

interface EvalTableProps {
  run: TrainingRunDetail
  /** Row the user picked locally (for publish / quick try); not the chosen checkpoint of the API. */
  selectedId: string | null
  onSelect: (id: string) => void
}

export function EvalTable({ run, selectedId, onSelect }: EvalTableProps) {
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set())
  const chosenId = run.lora?.chosen_checkpoint_id ?? null
  const base = rendersFor(run, null)

  const toggleExpanded = (id: string) =>
    setExpanded((prev) => {
      const next = new Set(prev)
      if (!next.delete(id)) next.add(id)
      return next
    })

  return (
    <section className="panel eval-panel" aria-label="Bảng bản lưu">
      <div className="table-wrap">
        <table className="table eval-table">
          <thead>
            <tr>
              <th scope="col">Bản lưu</th>
              <th scope="col">Câu lệnh nghe thử (LoRA vs không LoRA)</th>
              <th scope="col">Bài giữ lại (bài thật vs LoRA tạo)</th>
              <th scope="col">Tự chấm</th>
              <th scope="col">Ghi chú</th>
            </tr>
          </thead>
          <tbody>
            <tr className="eval-ref">
              <th scope="row">Bài mẫu</th>
              <td>
                {base.length > 0 ? (
                  base.map((r) => (
                    <AudioButton key={r.id} artifactId={r.artifact_id} title="Bài mẫu · Không LoRA" tag="Không LoRA" />
                  ))
                ) : (
                  <span className="muted">Chưa có</span>
                )}
              </td>
              <td>
                {run.held_out_tracks.length > 0 ? (
                  run.held_out_tracks.map((t) => (
                    <AudioButton
                      key={t.artifact_id}
                      artifactId={t.artifact_id}
                      title={`${t.filename} (thật)`}
                      tag={`${t.filename} (thật)`}
                    />
                  ))
                ) : (
                  <span className="muted">Không có bài giữ lại</span>
                )}
              </td>
              <td colSpan={2} className="muted">
                Nghe bài mẫu để so với từng epoch.
              </td>
            </tr>
            {run.checkpoints.map((c) => {
              const removed = isRemoved(c)
              const selected = !removed && c.id === selectedId
              const renders = rendersFor(run, c.id)
              const prompt = renders.filter((r) => r.kind === 'prompt')
              const heldOut = renders.filter((r) => r.kind === 'held_out')
              const open = expanded.has(c.id)
              const shown = open ? heldOut : heldOut.slice(0, 1)
              const verdict = checkpointVerdict(c, run.checkpoints)
              const label = epochLabel(c)
              const select = () => {
                if (!removed) onSelect(c.id)
              }
              return (
                <tr
                  key={c.id}
                  className={cx('eval-row', selected && 'is-selected', removed && 'is-removed')}
                  onClick={select}
                >
                  <th scope="row">
                    <label className="eval-pick" onClick={(e) => e.stopPropagation()}>
                      <input
                        type="radio"
                        name="eval-checkpoint"
                        aria-label={`Chọn ${label}`}
                        checked={selected}
                        disabled={removed}
                        onChange={select}
                      />
                      <span>{`${label} · bước ${c.step ?? DASH}`}</span>
                    </label>
                    {c.id === chosenId && <Badge tone="ok">Đang chọn</Badge>}
                    {removed && <span className="muted eval-removed">Đã xóa khỏi máy</span>}
                  </th>
                  <td>
                    {prompt.length > 0 ? (
                      prompt.map((r) => (
                        <AudioButton key={r.id} artifactId={r.artifact_id} title={`${label} · LoRA`} tag="LoRA" />
                      ))
                    ) : (
                      <span className="muted">Chưa có bài nghe thử</span>
                    )}
                  </td>
                  <td>
                    {shown.map((r) => (
                      <AudioButton
                        key={r.id}
                        artifactId={r.artifact_id}
                        title={`${label} · LoRA tạo · ${r.sample_filename ?? ''}`}
                        tag={`LoRA tạo · ${r.sample_filename ?? ''}`}
                      />
                    ))}
                    {heldOut.length > 1 && (
                      <button
                        type="button"
                        className="link-btn"
                        aria-expanded={open}
                        onClick={(e) => {
                          e.stopPropagation()
                          toggleExpanded(c.id)
                        }}
                      >
                        {open ? 'Thu gọn' : `+ ${heldOut.length - 1} bài nữa`}
                      </button>
                    )}
                  </td>
                  <td>
                    <ScoreBar label="Giống style" value={c.style_score} max={1} text={c.style_score?.toFixed(2) ?? ''} />
                    <ScoreBar
                      label="Giống bài train"
                      value={c.train_score}
                      max={1}
                      text={c.train_score?.toFixed(2) ?? ''}
                    />
                    <ScoreBar
                      label="Giai điệu"
                      value={c.melody_score}
                      max={1}
                      text={c.melody_score?.toFixed(2) ?? ''}
                    />
                    <ScoreBar
                      label="Chất lượng âm thanh"
                      value={c.quality_score}
                      max={10}
                      text={c.quality_score === null ? '' : `${c.quality_score.toFixed(1)}/10`}
                    />
                    {verdict.text && <p className={cx('eval-verdict', `is-${verdict.tone}`)}>{verdict.text}</p>}
                  </td>
                  <td>
                    <NoteField checkpoint={c} />
                  </td>
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>
      <p className="field-hint">
        Bấm vào một dòng để chọn bản đó. Các thanh điểm là máy so sánh âm thanh, chỉ để tham khảo; quyết định là tai bạn.
      </p>
    </section>
  )
}
