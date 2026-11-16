import { Navigate, Outlet, Route, Routes, useLocation } from 'react-router-dom'
import { useAuth } from './auth/AuthContext'
import { Layout } from './components/Layout'
import { LoadingState } from './components/ui'
import { ComingSoonPage, NotFoundPage } from './pages/ComingSoonPage'
import { CreatePage } from './pages/CreatePage'
import { LibraryPage } from './pages/LibraryPage'
import { LoginPage } from './pages/LoginPage'
import { LoraLibraryPage } from './pages/LoraLibraryPage'
import { QueuePage } from './pages/QueuePage'
import { TrackDetailPage } from './pages/TrackDetailPage'
import { TrainingPage } from './pages/TrainingPage'
import { TrainingRunPage } from './pages/TrainingRunPage'
import { AuditLogsPage } from './pages/admin/AuditLogsPage'
import { GenresPage } from './pages/admin/GenresPage'
import { ModelsPage } from './pages/admin/ModelsPage'
import { NodesPage } from './pages/admin/NodesPage'
import { StylesPage } from './pages/admin/StylesPage'
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
              description="Trang quản lý dataset sẽ có sau. Hiện dataset là các thư mục trên Render Node."
            />
          }
        />
        {/* Admin only; the pages themselves show a notice to everyone else. */}
        <Route path="/training" element={<TrainingPage />} />
        <Route path="/training/:id" element={<TrainingRunPage />} />
        <Route path="/lora-library" element={<LoraLibraryPage />} />
        <Route
          path="/model-training"
          element={
            <ComingSoonPage
              title="Training Model"
              description="Train một base model mới từ dataset của studio. Model train xong sẽ vào Model Library để nghe thử và duyệt."
            />
          }
        />
        <Route
          path="/model-library"
          element={
            <ComingSoonPage
              title="Model Library"
              description="Các model đã train xong sẽ nằm ở đây. Model được duyệt sẽ xuất hiện ở Quản trị → Models và chọn được ở trang Tạo nhạc."
            />
          }
        />
        <Route path="/admin" element={<RequireAdmin />}>
          <Route index element={<Navigate to="genres" replace />} />
          <Route path="genres" element={<GenresPage />} />
          <Route path="styles" element={<StylesPage />} />
          {/* Old address of the Style page. */}
          <Route path="presets" element={<Navigate to="/admin/styles" replace />} />
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
