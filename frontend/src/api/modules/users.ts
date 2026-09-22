import type { RequestOptions } from '../request'
import type { ClientProfileSelection, XaiClientProfileSelection } from './client-profiles'
import type { AdminSubscription, SubscriptionPlan, UserBudgetStatus } from './user'
import request from '../request'

export interface AdminUser {
  id: string
  username: string
  role: 'admin' | 'user'
  enabled: boolean
  maxConcurrency: number | null
  requestsPerMinute: number | null
  createdAt: string
  updatedAt: string
}

export interface CreateAdminUserParam {
  username: string
  password: string
  enabled: boolean
}

export interface UpdateAdminUserParam {
  username?: string
  role?: 'admin' | 'user'
  enabled?: boolean
  maxConcurrency?: number | null
  requestsPerMinute?: number | null
}

export interface UserManagementSummary {
  userId: string
  username: string
  email: string | null
  role: 'admin' | 'user'
  enabled: boolean
  effectivePlan: SubscriptionPlan
  effectiveSource: 'basePlan' | 'subscription'
  subscription: AdminSubscription | null
  multiplier: string
  effectiveMaxConcurrency: number | null
  effectiveRequestsPerMinute: number | null
  groups: Array<{ groupId: string, name: string, enabled: boolean }>
  budget: UserBudgetStatus
  updatedAt: string
}

export interface UserManagementPage {
  items: UserManagementSummary[]
  page: number
  pageSize: number
  total: number
}

export interface AdminUsersManagementQuery {
  page?: number
  pageSize?: number
  query?: string
  role?: string
  status?: string
  planId?: string
  groupId?: string
}

export function getAdminUsers(options: RequestOptions = {}) {
  return request<AdminUser[]>({
    url: '/api/admin/users',
    method: 'GET',
    ...options,
  })
}

/** 用户列表身份接口保持兼容；管理页面使用聚合接口避免逐行 billing 请求。 */
export function getAdminUsersManagement(params: AdminUsersManagementQuery = {}, options: RequestOptions = {}) {
  return request<UserManagementPage>({
    url: '/api/admin/users/management',
    method: 'GET',
    params,
    ...options,
  })
}

export function createAdminUser(data: CreateAdminUserParam, options: RequestOptions = {}) {
  return request<AdminUser>({
    url: '/api/admin/users/create',
    method: 'POST',
    data,
    ...options,
  })
}

export function updateAdminUser(id: string, data: UpdateAdminUserParam, options: RequestOptions = {}) {
  return request<AdminUser>({
    url: '/api/admin/users/update',
    method: 'POST',
    data: { userId: id, ...data },
    ...options,
  })
}

export function resetAdminUserPassword(id: string, password: string, options: RequestOptions = {}) {
  return request<void>({
    url: '/api/admin/users/reset-password',
    method: 'POST',
    data: { userId: id, password },
    ...options,
  })
}

export function revokeAdminUserSessions(id: string, options: RequestOptions = {}) {
  return request<{ invalidated: boolean }>({
    url: '/api/admin/users/revoke-sessions',
    method: 'POST',
    data: { userId: id },
    ...options,
  })
}

export function deleteAdminUser(userId: string, options: RequestOptions = {}) {
  return request<void>({
    url: '/api/admin/users/delete',
    method: 'POST',
    data: { userId },
    ...options,
  })
}

export interface ResetAdminUserBudgetParam {
  daily: boolean
  weekly: boolean
  monthly: boolean
}

export function resetAdminUserBudget(userId: string, data: ResetAdminUserBudgetParam, options: RequestOptions = {}) {
  return request<ResetAdminUserBudgetParam>({
    url: '/api/admin/users/billing/reset',
    method: 'POST',
    data: { userId, ...data },
    ...options,
  })
}

/** User 默认上游身份；null 表示跟随系统。owned Key 可由管理员单独覆盖。 */
export interface UserKeyIdentityWrite {
  openaiClientProfileOverride: ClientProfileSelection | null
  xaiClientProfileOverride: XaiClientProfileSelection | null
}

export interface UserKeyIdentityKey extends UserKeyIdentityWrite {
  id: string
  name: string
  label: string | null
  prefix: string
  enabled: boolean
}

export interface UserKeyIdentity extends UserKeyIdentityWrite {
  keyCount: number
  keys: UserKeyIdentityKey[]
}

export function getAdminUserKeyIdentity(userId: string, options: RequestOptions = {}) {
  return request<UserKeyIdentity>({ url: '/api/admin/users/key-identity', method: 'GET', params: { userId }, ...options })
}

export function updateAdminUserKeyIdentity(userId: string, data: UserKeyIdentityWrite, options: RequestOptions = {}) {
  return request<void>({ url: '/api/admin/users/key-identity/update', method: 'POST', data: { userId, ...data }, ...options })
}

export function updateAdminUserClientKeyIdentity(userId: string, keyId: string, data: UserKeyIdentityWrite, options: RequestOptions = {}) {
  return request<void>({
    url: '/api/admin/users/key-identity/key/update',
    method: 'POST',
    data: { userId, keyId, ...data },
    ...options,
  })
}
