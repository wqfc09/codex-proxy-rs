import type { AppRole } from './access'
import { createRouter, createWebHistory } from 'vue-router'

import { useAuthStore } from '@/stores/modules/auth'
import { canAccessRoles } from './access'
import { routes } from './routes'

export const router = createRouter({
  history: createWebHistory('/'),
  routes,
})

router.beforeEach(async (to) => {
  const authStore = useAuthStore()

  if (!authStore.sessionChecked) {
    try {
      await authStore.checkAuth()
    }
    catch {
      if (to.path === '/login')
        return true
      return { name: 'login', query: { redirect: to.fullPath } }
    }
  }

  if (to.path === '/login') {
    if (authStore.isAuthenticated)
      return authStore.defaultRoute
    return true
  }

  if (!authStore.isAuthenticated)
    return { name: 'login', query: { redirect: to.fullPath } }

  const requiredRoles = to.matched.flatMap(record => (record.meta.roles as readonly AppRole[] | undefined) ?? [])
  if (!canAccessRoles(requiredRoles, authStore.role))
    return authStore.defaultRoute

  return true
})
