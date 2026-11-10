import { Hourglass } from 'lucide-react'
import { EmptyState, PageHeader } from '../components/ui'

/** Placeholder for Phase 3 features. Deliberately has no mock UI. */
export function ComingSoonPage({ title, description }: { title: string; description: string }) {
  return (
    <div className="page">
      <PageHeader title={title} />
      <EmptyState icon={<Hourglass size={26} aria-hidden />} title="Sắp có">
        {description}
      </EmptyState>
    </div>
  )
}

export function NotFoundPage() {
  return (
    <div className="page">
      <EmptyState title="Không tìm thấy trang">Đường dẫn này không tồn tại.</EmptyState>
    </div>
  )
}
