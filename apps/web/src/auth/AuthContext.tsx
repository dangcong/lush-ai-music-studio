import { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react'
import type { ReactNode } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { api } from '../api/api'
import { setUnauthorizedHandler } from '../api/client'
import type { User } from '../api/types'

export interface AuthContextValue {
  /** `undefined` while the session is being checked, `null` when signed out. */
  user: User | null | undefined
  login: (username: string, password: string) => Promise<void>
  logout: () => Promise<void>
}

export const AuthContext = createContext<AuthContextValue | null>(null)

export function AuthProvider({ children }: { children: ReactNode }) {
  const queryClient = useQueryClient()
  const [user, setUser] = useState<User | null | undefined>(undefined)

  const signOutLocally = useCallback(() => {
    setUser(null)
    queryClient.clear()
  }, [queryClient])

  useEffect(() => {
    // Any 401 anywhere in the app drops the session; route guards then redirect to /login.
    setUnauthorizedHandler(signOutLocally)
    let cancelled = false
    api.auth
      .me()
      .catch((err) => {
        // Dev server only: sign in with the credentials from .env.development.local, if any.
        // `import.meta.env.DEV` is false in `vite build`, so this branch is not in production.
        const username = import.meta.env.VITE_DEV_LOGIN_USERNAME
        const password = import.meta.env.VITE_DEV_LOGIN_PASSWORD
        if (import.meta.env.DEV && username && password) return api.auth.login(username, password)
        throw err
      })
      .then((me) => {
        if (!cancelled) setUser(me)
      })
      .catch(() => {
        if (!cancelled) setUser(null)
      })
    return () => {
      cancelled = true
      setUnauthorizedHandler(null)
    }
  }, [signOutLocally])

  const login = useCallback(
    async (username: string, password: string) => {
      const me = await api.auth.login(username, password)
      queryClient.clear()
      setUser(me)
    },
    [queryClient],
  )

  const logout = useCallback(async () => {
    try {
      await api.auth.logout()
    } finally {
      signOutLocally()
    }
  }, [signOutLocally])

  const value = useMemo(() => ({ user, login, logout }), [user, login, logout])
  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>
}

export function useAuth(): AuthContextValue {
  const ctx = useContext(AuthContext)
  if (!ctx) throw new Error('useAuth must be used inside AuthProvider')
  return ctx
}

/** For components rendered only behind the auth guard. */
export function useCurrentUser(): User {
  const { user } = useAuth()
  if (!user) throw new Error('useCurrentUser requires a signed-in user')
  return user
}
