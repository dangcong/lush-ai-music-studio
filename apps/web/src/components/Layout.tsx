import { useEffect, useState } from 'react'
import { Menu } from 'lucide-react'
import { Outlet, useLocation } from 'react-router-dom'
import { cx } from '../lib/cx'
import { PlayerBar } from '../player/PlayerBar'
import { usePlayer } from '../player/PlayerContext'
import { Logo } from './Logo'
import { Sidebar } from './Sidebar'

/** App shell: sidebar, scrolling work area and the fixed player bar. */
export function Layout() {
  const location = useLocation()
  const [navOpen, setNavOpen] = useState(false)

  // The shell only unmounts on sign-out: stop playback then.
  const { stop } = usePlayer()
  useEffect(() => stop, [stop])

  // Close the mobile drawer after navigating.
  useEffect(() => setNavOpen(false), [location.pathname])

  return (
    <div className={cx('app', navOpen && 'nav-open')}>
      <header className="topbar">
        <button
          type="button"
          className="icon-btn"
          aria-label={navOpen ? 'Đóng menu' : 'Mở menu'}
          aria-expanded={navOpen}
          onClick={() => setNavOpen((o) => !o)}
        >
          <Menu size={20} aria-hidden />
        </button>
        <Logo size={26} />
        <span className="brand-name">Lush AI Music Studio</span>
      </header>
      <Sidebar />
      {navOpen && <div className="nav-scrim" onClick={() => setNavOpen(false)} aria-hidden />}
      <main className="main" id="main">
        <Outlet />
      </main>
      <PlayerBar />
    </div>
  )
}
