import type { ModelCount } from '../../api/types'

const SEGMENT_COLORS = ['#7c6cf0', '#2fa37a', '#d98a2b', '#d9577a', '#3b9ac4', '#8f8f3a']

/** Proportional bar + legend of which model wrote the captions. */
export function ModelBar({ models }: { models: ModelCount[] }) {
  return (
    <>
      <div className="lora-bar" aria-hidden>
        {models.map((m, i) => (
          <span key={m.model_key} style={{ flex: m.count, background: SEGMENT_COLORS[i % SEGMENT_COLORS.length] }} />
        ))}
      </div>
      <ul className="lora-legend">
        {models.map((m, i) => (
          <li key={m.model_key}>
            <i style={{ background: SEGMENT_COLORS[i % SEGMENT_COLORS.length] }} aria-hidden />
            {`${m.name} · ${m.count} bài`}
          </li>
        ))}
      </ul>
    </>
  )
}
