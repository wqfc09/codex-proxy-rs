export type AppRole = 'admin' | 'user'

export type AppNavIcon
  = | 'dashboard'
    | 'accounts'
    | 'proxies'
    | 'groups'
    | 'keys'
    | 'usage'
    | 'users'
    | 'plans'
    | 'profile'
    | 'theme'
    | 'settings'

export interface AppNavItem {
  label: string
  path: string
  icon: AppNavIcon
}

const adminNavigation: AppNavItem[] = [
  { label: '概览', icon: 'dashboard', path: '/' },
  { label: '账号管理', icon: 'accounts', path: '/accounts' },
  { label: '代理管理', icon: 'proxies', path: '/proxies' },
  { label: '分组管理', icon: 'groups', path: '/groups' },
  { label: '用户管理', icon: 'users', path: '/users' },
  { label: '使用统计', icon: 'usage', path: '/usage' },
  { label: '主题设置', icon: 'theme', path: '/theme' },
  { label: '系统设置', icon: 'settings', path: '/settings' },
]

const userNavigation: AppNavItem[] = [
  { label: '概览', icon: 'dashboard', path: '/user' },
  { label: 'API 密钥', icon: 'keys', path: '/user/api-keys' },
  { label: '使用记录', icon: 'usage', path: '/user/usage' },
  { label: '我的套餐', icon: 'plans', path: '/user/plan' },
  { label: '账户设置', icon: 'profile', path: '/user/settings' },
  { label: '主题设置', icon: 'theme', path: '/user/theme' },
]

// 工作空间由已授权路由决定，刷新和深链接不会与另一份本地状态冲突。
export function workspaceForRoute(role: AppRole | null, path: string): AppRole | null {
  if (!role)
    return null
  return role === 'admin' && path !== '/user' && !path.startsWith('/user/') ? 'admin' : 'user'
}

export function defaultRouteForRole(role: AppRole | null | undefined) {
  return role === 'user' ? '/user' : '/'
}

export function navigationForRole(role: AppRole | null | undefined, workspace: AppRole | null | undefined = role) {
  if (role === 'admin' && workspace === 'admin')
    return adminNavigation
  if (workspace === 'user')
    return role === 'admin' ? userNavigation.filter(item => item.path !== '/user/theme') : userNavigation
  return []
}

export function canAccessRoles(roles: readonly AppRole[] | undefined, role: AppRole | null) {
  return !roles?.length || (role !== null && roles.includes(role))
}

export function isNavigationItemActive(currentPath: string, itemPath: string) {
  if (itemPath === '/' || itemPath === '/user')
    return currentPath === itemPath
  return currentPath === itemPath || currentPath.startsWith(`${itemPath}/`)
}
