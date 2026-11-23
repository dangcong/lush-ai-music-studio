import { useParams } from 'react-router-dom'

/** Placeholder; the real catalog detail page replaces it in the next task. */
export function CatalogPage() {
  const { id } = useParams()
  return (
    <div className="page" data-testid="catalog-placeholder">
      {id}
    </div>
  )
}
