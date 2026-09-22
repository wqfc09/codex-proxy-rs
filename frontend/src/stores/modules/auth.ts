import type { AuthSession, LoginParam, SessionUser } from '@/api'
import type { AppRole } from '@/router/access'

import { defineStore } from 'pinia'
import { computed, shallowRef } from 'vue'

import {
  login as apiLogin,
  logout as apiLogout,
  getAuthStatus,
  getCurrentUser,
} from '@/api'
import { resetUnauthorizedHandling } from '@/api/request'
import { defaultRouteForRole } from '@/router/access'

export const useAuthStore = defineStore('auth', () => {
  const session = shallowRef<AuthSession | null>(null)
  const currentUser = shallowRef<SessionUser | null>(null)
  const sessionChecked = shallowRef(false)
  const loading = shallowRef(false)
  let revision = 0
  let pendingCheck: Promise<boolean> | undefined

  const isAuthenticated = computed(() => session.value !== null && currentUser.value !== null)
  const role = computed<AppRole | null>(() => session.value?.role ?? null)
  const isAdmin = computed(() => role.value === 'admin')
  const isUser = computed(() => role.value === 'user')
  const defaultRoute = computed(() => defaultRouteForRole(role.value))

  function clearSession() {
    session.value = null
    currentUser.value = null
  }

  function checkAuth(): Promise<boolean> {
    if (pendingCheck)
      return pendingCheck

    const currentRevision = revision
    const check = (async () => {
      const status = await getAuthStatus({ silent: true })
      if (currentRevision !== revision)
        return isAuthenticated.value

      if (!status.authenticated || !status.session) {
        clearSession()
        sessionChecked.value = true
        return false
      }

      const user = await getCurrentUser({ silent: true })
      if (currentRevision !== revision)
        return isAuthenticated.value

      session.value = status.session
      currentUser.value = user
      sessionChecked.value = true
      resetUnauthorizedHandling()
      return true
    })().finally(() => {
      if (pendingCheck === check)
        pendingCheck = undefined
    })

    pendingCheck = check
    return check
  }

  async function login(payload: LoginParam) {
    if (loading.value)
      return null

    revision += 1
    pendingCheck = undefined
    loading.value = true
    try {
      const result = await apiLogin(payload)
      const user = await getCurrentUser({ silent: true })
      revision += 1
      pendingCheck = undefined
      session.value = result
      currentUser.value = user
      sessionChecked.value = true
      resetUnauthorizedHandling()
      return result
    }
    catch {
      clearSession()
      sessionChecked.value = true
      return null
    }
    finally {
      loading.value = false
    }
  }

  async function logout() {
    if (loading.value)
      return false

    loading.value = true
    revision += 1
    pendingCheck = undefined
    try {
      await apiLogout({ silent: true })
      invalidateSession()
      return true
    }
    catch {
      // 只有服务端确认撤销后才清空本地身份，避免刷新后恢复未撤销会话。
      return false
    }
    finally {
      loading.value = false
    }
  }

  function updateCurrentUser(user: SessionUser) {
    if (currentUser.value?.id === user.id)
      currentUser.value = user
  }

  function invalidateSession() {
    revision += 1
    pendingCheck = undefined
    clearSession()
    sessionChecked.value = true
    resetUnauthorizedHandling()
  }

  return {
    session,
    currentUser,
    isAuthenticated,
    sessionChecked,
    loading,
    role,
    isAdmin,
    isUser,
    defaultRoute,
    checkAuth,
    login,
    logout,
    updateCurrentUser,
    invalidateSession,
  }
})
