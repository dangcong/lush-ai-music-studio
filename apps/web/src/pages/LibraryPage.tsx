import { useState } from 'react'
import { Library, Search } from 'lucide-react'
import { useGenres, useTracks } from '../api/hooks'
import type { TrackListParams } from '../api/types'
import { TrackCard } from '../components/TrackCard'
import { EmptyState, ErrorState, LoadingState, PageHeader, Switch } from '../components/ui'
import { topLevelGenres } from '../features/create/formState'
import { useDebounced } from '../lib/useDebounced'

const LIBRARY_LIMIT = 200

export function LibraryPage() {
  const [search, setSearch] = useState('')
  const [genreId, setGenreId] = useState('')
  const [favorite, setFavorite] = useState(false)
  const [mine, setMine] = useState(false)
  const q = useDebounced(search.trim())

  const params: TrackListParams = { q, genre_id: genreId, favorite, mine, limit: LIBRARY_LIMIT }
  const tracksQuery = useTracks(params)
  const genresQuery = useGenres()
  const filtered = Boolean(q || genreId || favorite || mine)

  return (
    <div className="page">
      <PageHeader title="Thư viện" description="Mọi bản nhạc đã tạo, sẵn sàng để nghe lại và tải xuống." />

      <div className="toolbar" role="search">
        <div className="search-box">
          <Search size={16} aria-hidden />
          <input
            type="search"
            aria-label="Tìm kiếm track"
            placeholder="Tìm theo tiêu đề, prompt…"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
        </div>
        <select aria-label="Lọc theo Genre" value={genreId} onChange={(e) => setGenreId(e.target.value)}>
          <option value="">Mọi Genre</option>
          {topLevelGenres(genresQuery.data ?? []).map((g) => (
            <option key={g.id} value={g.id}>
              {g.name}
            </option>
          ))}
        </select>
        <Switch label="Chỉ yêu thích" checked={favorite} onChange={setFavorite} />
        <Switch label="Chỉ của tôi" checked={mine} onChange={setMine} />
      </div>

      {tracksQuery.isPending ? (
        <LoadingState />
      ) : tracksQuery.isError ? (
        <ErrorState error={tracksQuery.error} onRetry={() => void tracksQuery.refetch()} />
      ) : tracksQuery.data.length === 0 ? (
        <EmptyState
          icon={<Library size={26} aria-hidden />}
          title={filtered ? 'Không có track nào khớp bộ lọc' : 'Thư viện còn trống'}
        >
          {filtered ? 'Thử đổi từ khóa hoặc bỏ bớt bộ lọc.' : 'Các bản nhạc tạo xong sẽ xuất hiện ở đây.'}
        </EmptyState>
      ) : (
        <ul className="track-grid">
          {tracksQuery.data.map((track) => (
            <li key={track.id}>
              <TrackCard track={track} showOwner={!mine} />
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
