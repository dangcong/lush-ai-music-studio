import type { TrainingMetric } from '../../api/types'

export interface LossPoint {
  step: number
  loss: number
}

/** Only points the trainer actually reported, in step order. Nothing is interpolated. */
export function lossPoints(metrics: TrainingMetric[] | null | undefined): LossPoint[] {
  const points: LossPoint[] = []
  for (const m of metrics ?? []) {
    if (
      typeof m.global_step === 'number' &&
      typeof m.train_loss === 'number' &&
      Number.isFinite(m.global_step) &&
      Number.isFinite(m.train_loss)
    ) {
      points.push({ step: m.global_step, loss: m.train_loss })
    }
  }
  return points.sort((a, b) => a.step - b.step)
}

const WIDTH = 640
const HEIGHT = 240
const PAD = { top: 14, right: 16, bottom: 34, left: 58 }
const Y_TICKS = 4

function formatLoss(value: number): string {
  return Math.abs(value) >= 100 ? value.toFixed(1) : value.toFixed(4)
}

/**
 * Train loss by optimizer step. Straight segments join the reported points and every
 * reported point has its own marker; no smoothing, no values between points.
 */
export function LossChart({ metrics }: { metrics: TrainingMetric[] | null | undefined }) {
  const points = lossPoints(metrics)
  if (points.length === 0) {
    return <p className="muted chart-empty">Chưa có dữ liệu</p>
  }

  const first = points[0]!
  const last = points[points.length - 1]!
  const minStep = first.step
  const maxStep = last.step
  let minLoss = Math.min(...points.map((p) => p.loss))
  let maxLoss = Math.max(...points.map((p) => p.loss))
  if (minLoss === maxLoss) {
    // A flat series (or a single point) still needs a non-empty y range to sit in.
    const pad = Math.abs(minLoss) * 0.05 || 0.5
    minLoss -= pad
    maxLoss += pad
  }

  const plotW = WIDTH - PAD.left - PAD.right
  const plotH = HEIGHT - PAD.top - PAD.bottom
  const x = (step: number) =>
    PAD.left + (maxStep === minStep ? plotW / 2 : ((step - minStep) / (maxStep - minStep)) * plotW)
  const y = (loss: number) => PAD.top + (1 - (loss - minLoss) / (maxLoss - minLoss)) * plotH

  const yTicks = Array.from({ length: Y_TICKS + 1 }, (_, i) => minLoss + ((maxLoss - minLoss) * i) / Y_TICKS)
  const xTicks = maxStep === minStep ? [minStep] : [minStep, maxStep]
  const markerRadius = points.length > 80 ? 2 : 4

  return (
    <figure className="loss-chart">
      <svg
        viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
        role="img"
        aria-label={`Train loss theo optimizer step: ${points.length} điểm, step ${minStep} đến ${maxStep}, loss mới nhất ${formatLoss(last.loss)}`}
      >
        {yTicks.map((t) => (
          <g key={t}>
            <line className="chart-grid" x1={PAD.left} x2={WIDTH - PAD.right} y1={y(t)} y2={y(t)} />
            <text className="chart-tick" x={PAD.left - 8} y={y(t)} textAnchor="end" dominantBaseline="middle">
              {formatLoss(t)}
            </text>
          </g>
        ))}
        {xTicks.map((t) => (
          <text key={t} className="chart-tick" x={x(t)} y={HEIGHT - PAD.bottom + 16} textAnchor="middle">
            {t}
          </text>
        ))}
        <text className="chart-axis-title" x={PAD.left + plotW / 2} y={HEIGHT - 4} textAnchor="middle">
          Optimizer step
        </text>
        {points.length > 1 && (
          <polyline
            className="chart-line"
            fill="none"
            points={points.map((p) => `${x(p.step).toFixed(1)},${y(p.loss).toFixed(1)}`).join(' ')}
          />
        )}
        {points.map((p, i) => (
          <g key={i} className="chart-point" data-testid="loss-point">
            <circle className="chart-marker" cx={x(p.step)} cy={y(p.loss)} r={markerRadius} />
            {/* Larger invisible hit target carrying the exact reported value. */}
            <circle className="chart-hit" cx={x(p.step)} cy={y(p.loss)} r={9}>
              <title>{`Step ${p.step}: loss ${formatLoss(p.loss)}`}</title>
            </circle>
          </g>
        ))}
      </svg>
      <figcaption className="field-hint">
        Train loss theo optimizer step — {points.length} điểm do trainer báo về, nối bằng đoạn thẳng, không nội suy.
        Rê chuột vào từng điểm để xem giá trị.
      </figcaption>
    </figure>
  )
}
