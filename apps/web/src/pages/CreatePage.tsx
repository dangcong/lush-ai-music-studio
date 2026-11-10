import { useLocation } from 'react-router-dom'
import type { GenerationRequest } from '../api/types'
import { PageHeader } from '../components/ui'
import { GenerateForm } from '../features/create/GenerateForm'
import { RecentJobs } from '../features/create/RecentJobs'
import { stateFromRequest } from '../features/create/formState'

/** Router state used by "Tạo lại với thiết lập này" to pre-fill the form. */
export interface CreateLocationState {
  prefill?: Partial<GenerationRequest>
}

export function CreatePage() {
  const location = useLocation()
  const prefill = (location.state as CreateLocationState | null)?.prefill

  return (
    <div className="page create-page">
      <PageHeader title="Tạo nhạc" description="Mô tả ý tưởng, chọn phong cách và để máy render lo phần còn lại." />
      <div className="create-grid">
        <section className="panel" aria-label="Thiết lập">
          {/* Remount when navigated here with new pre-filled settings. */}
          <GenerateForm key={location.key} initial={prefill ? stateFromRequest(prefill) : undefined} />
        </section>
        <section className="create-results" aria-labelledby="recent-heading">
          <h2 id="recent-heading" className="section-title">
            Gần đây của tôi
          </h2>
          <RecentJobs />
        </section>
      </div>
    </div>
  )
}
