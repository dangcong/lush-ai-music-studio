import { Navigate, Outlet, Route, Routes, useLocation } from 'react-router-dom'
import { useAuth } from './auth/AuthContext'
import { Layout } from './components/Layout'
import { LoadingState } from './components/ui'
import { ComingSoonPage, NotFoundPage } from './pages/ComingSoonPage'
import { CreatePage } from './pages/CreatePage'
import { LibraryPage } from './pages/LibraryPage'
import { LoginPage } from './pages/LoginPage'
import { QueuePage } from './pages/QueuePage'
import { TrackDetailPage } from './pages/TrackDetailPage'
import { AuditLogsPage } from './pages/admin/AuditLogsPage'
import { GenresPage } from './pages/admin/GenresPage'
import { ModelsPage } from './pages/admin/ModelsPage'
import { NodesPage } from './pages/admin/NodesPage'
import { PresetsPage } from './pages/admin/PresetsPage'
import { UsersPage } from './pages/admin/UsersPage'

/** Every route below requires a session; signed-out users (or any 401) land on /login. */
function RequireAuth() {
  const { user } = useAuth()
  const location = useLocation()
  if (user === undefined) {
    return (
      <div className="boot">
        <LoadingState label="Đang kiểm tra phiên đăng nhập…" />
      </div>
    )
  }
  if (user === null) {
    return <Navigate to="/login" replace state={{ from: location.pathname + location.search }} />
  }
  return <Layout />
}

function RequireAdmin() {
  const { user } = useAuth()
  if (user?.role !== 'admin') return <Navigate to="/create" replace />
  return <Outlet />
}

export function App() {
  return (
    <Routes>
      <Route path="/login" element={<LoginPage />} />
      <Route element={<RequireAuth />}>
        <Route index element={<Navigate to="/create" replace />} />
        <Route path="/create" element={<CreatePage />} />
        <Route path="/library" element={<LibraryPage />} />
        <Route path="/tracks/:id" element={<TrackDetailPage />} />
        <Route path="/queue" element={<QueuePage />} />
        <Route
          path="/datasets"
          element={
            <ComingSoonPage
              title="Dataset"
              description="Quản lý bộ dữ liệu audio để huấn luyện sẽ có ở Phase 3."
            />
          }
        />
        <Route
          path="/training"
          element={
            <ComingSoonPage title="Training" description="Huấn luyện LoRA từ dataset sẽ có ở Phase 3." />
          }
        />
        <Route
          path="/lora-library"
          element={
            <ComingSoonPage
              title="LoRA Library"
              description="Thư viện LoRA đã huấn luyện sẽ có ở Phase 3."
            />
          }
        />
        <Route path="/admin" element={<RequireAdmin />}>
          <Route index element={<Navigate to="genres" replace />} />
          <Route path="genres" element={<GenresPage />} />
          <Route path="presets" element={<PresetsPage />} />
          <Route path="models" element={<ModelsPage />} />
          <Route path="nodes" element={<NodesPage />} />
          <Route path="users" element={<UsersPage />} />
          <Route path="audit-logs" element={<AuditLogsPage />} />
        </Route>
        <Route path="*" element={<NotFoundPage />} />
      </Route>
    </Routes>
  )
}
