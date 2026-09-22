import type { RouteRecordRaw } from 'vue-router'

const adminRoles = ['admin'] as const
const userRoles = ['admin', 'user'] as const

export const routes: RouteRecordRaw[] = [
  {
    path: '/login',
    name: 'login',
    component: () => import('@/views/login/index.vue'),
  },
  {
    path: '/',
    component: () => import('@/layout/index.vue'),
    children: [
      { path: '', name: 'dashboard', meta: { roles: adminRoles }, component: () => import('@/views/dashboard/index.vue') },
      { path: 'accounts', name: 'accounts', meta: { roles: adminRoles }, component: () => import('@/views/accounts/index.vue') },
      { path: 'proxies', name: 'proxies', meta: { roles: adminRoles }, component: () => import('@/views/proxies/index.vue') },
      { path: 'groups', name: 'groups', meta: { roles: adminRoles }, component: () => import('@/views/groups/index.vue') },
      { path: 'users', name: 'users', meta: { roles: adminRoles }, component: () => import('@/views/users/index.vue') },
      { path: 'plans', name: 'plans', meta: { roles: adminRoles }, redirect: to => ({ name: 'users', query: { ...to.query, tab: 'plans' } }) },
      { path: 'usage', name: 'usage', meta: { roles: adminRoles }, component: () => import('@/views/usage/index.vue') },
      { path: 'theme', name: 'theme', meta: { roles: adminRoles }, component: () => import('@/views/theme/index.vue') },
      { path: 'settings', name: 'settings', meta: { roles: adminRoles }, component: () => import('@/views/settings/index.vue') },
      { path: 'settings/upstream', name: 'settings-upstream', meta: { roles: adminRoles }, component: () => import('@/views/settings/index.vue') },
      { path: 'settings/access', name: 'settings-access', meta: { roles: adminRoles }, component: () => import('@/views/settings/index.vue') },
      { path: 'settings/security', name: 'settings-security', meta: { roles: adminRoles }, redirect: '/settings/access' },
      { path: 'settings/backup', name: 'settings-backup', meta: { roles: adminRoles }, component: () => import('@/views/settings/index.vue') },
      { path: 'settings/pricing', name: 'settings-pricing', meta: { roles: adminRoles }, component: () => import('@/views/settings/index.vue') },
      { path: 'profile', name: 'profile', meta: { roles: userRoles }, redirect: '/user/settings' },
      { path: 'user', name: 'user-overview', meta: { roles: userRoles }, component: () => import('@/views/user/overview/index.vue') },
      { path: 'user/api-keys', name: 'user-api-keys', meta: { roles: userRoles }, component: () => import('@/views/user/api-keys/index.vue') },
      { path: 'user/usage', name: 'user-usage', meta: { roles: userRoles }, component: () => import('@/views/user/usage/index.vue') },
      { path: 'user/plan', name: 'user-plan', meta: { roles: userRoles }, component: () => import('@/views/user/plan/index.vue') },
      { path: 'user/theme', name: 'user-theme', meta: { roles: userRoles }, component: () => import('@/views/theme/index.vue') },
      { path: 'user/settings', name: 'user-settings', meta: { roles: userRoles }, component: () => import('@/views/user/settings/index.vue') },
    ],
  },
  { path: '/:pathMatch(.*)*', redirect: '/' },
]
