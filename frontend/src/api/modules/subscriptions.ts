import type { RequestOptions } from '../request'
import type { AdminBillingSummary, AdminSubscription, SubscriptionPlan } from './user'
import request from '../request'

export interface SubscriptionPlanWriteParam {
  name: string
  description: string | null
  dailyLimitUsd: string | null
  weeklyLimitUsd: string | null
  monthlyLimitUsd: string | null
}

export interface SubscriptionGrantParam {
  planId: string
  startsAt?: string
  expiresAt?: string
  durationDays?: number
  multiplier?: string
}

export interface SubscriptionPatchParam {
  startsAt?: string
  expiresAt?: string
  multiplier?: string
}

export interface SubscriptionRenewParam {
  extendByDays?: number
  expiresAt?: string
}

export interface SubscriptionHistoryPage {
  items: AdminSubscription[]
  page: number
  pageSize: number
  total: number
}

export interface UserGroupAssignment {
  groupId: string
  name: string
  enabled: boolean
  assignedAt: string
}

export interface UserGroupAssignmentResponse {
  items: UserGroupAssignment[]
  revision: number
}

export type TopupWindow = 'daily' | 'weekly' | 'monthly'

export interface BillingTopup {
  id: string
  userId: string
  planId: string
  subscriptionId: string | null
  window: TopupWindow
  windowStart: string
  windowEnd: string
  amount: string
  reason: string
  createdAt: string
}

export interface BillingTopupPage {
  items: BillingTopup[]
  page: number
  pageSize: number
  total: number
}

export function getSubscriptionPlans(options: RequestOptions = {}) {
  return request<SubscriptionPlan[]>({
    url: '/api/admin/subscription-plans',
    method: 'GET',
    ...options,
  })
}

export function createSubscriptionPlan(data: SubscriptionPlanWriteParam, options: RequestOptions = {}) {
  return request<SubscriptionPlan>({
    url: '/api/admin/subscription-plans/create',
    method: 'POST',
    data,
    ...options,
  })
}

export function updateSubscriptionPlan(id: string, data: SubscriptionPlanWriteParam, options: RequestOptions = {}) {
  return request<SubscriptionPlan>({
    url: '/api/admin/subscription-plans/update',
    method: 'POST',
    data: { id, ...data },
    ...options,
  })
}

export function setSubscriptionPlanEnabled(id: string, enabled: boolean, options: RequestOptions = {}) {
  return request<SubscriptionPlan>({
    url: enabled ? '/api/admin/subscription-plans/enable' : '/api/admin/subscription-plans/disable',
    method: 'POST',
    data: { id },
    ...options,
  })
}

export function getAdminUserBilling(userId: string, options: RequestOptions = {}) {
  return request<AdminBillingSummary>({
    url: '/api/admin/users/billing',
    method: 'GET',
    params: { userId },
    ...options,
  })
}

export function grantAdminUserSubscription(userId: string, planId: string, options: RequestOptions = {}) {
  return request<AdminSubscription>({
    url: '/api/admin/users/subscription/grant',
    method: 'POST',
    data: { userId, planId },
    ...options,
  })
}

export function grantAdminUserSubscriptionWithOptions(userId: string, data: SubscriptionGrantParam, options: RequestOptions = {}) {
  return request<AdminSubscription>({
    url: '/api/admin/users/subscription/grant',
    method: 'POST',
    data: { userId, ...data },
    ...options,
  })
}

export function patchAdminUserSubscription(userId: string, data: SubscriptionPatchParam, options: RequestOptions = {}) {
  return request<AdminSubscription>({
    url: '/api/admin/users/subscription/update',
    method: 'POST',
    data: { userId, ...data },
    ...options,
  })
}

export function renewAdminUserSubscription(userId: string, data: SubscriptionRenewParam, options: RequestOptions = {}) {
  return request<AdminSubscription>({
    url: '/api/admin/users/subscription/renew',
    method: 'POST',
    data: { userId, ...data },
    ...options,
  })
}

export function getAdminUserSubscriptionHistory(userId: string, params: { page?: number, pageSize?: number } = {}, options: RequestOptions = {}) {
  return request<SubscriptionHistoryPage>({
    url: '/api/admin/users/subscription-history',
    method: 'GET',
    params: { userId, ...params },
    ...options,
  })
}

export function getAdminUserGroups(userId: string, options: RequestOptions = {}) {
  return request<UserGroupAssignmentResponse>({
    url: '/api/admin/users/groups',
    method: 'GET',
    params: { userId },
    ...options,
  })
}

export function replaceAdminUserGroups(userId: string, groupIds: string[], options: RequestOptions = {}) {
  return request<UserGroupAssignmentResponse>({
    url: '/api/admin/users/groups/update',
    method: 'POST',
    data: { userId, groupIds },
    ...options,
  })
}

export function createAdminUserTopup(userId: string, data: { window: TopupWindow, amount: string, idempotencyKey: string, reason: string }, options: RequestOptions = {}) {
  return request<BillingTopup>({
    url: '/api/admin/users/billing/topups/create',
    method: 'POST',
    data: { userId, ...data },
    ...options,
  })
}

export function getAdminUserTopups(userId: string, params: { page?: number, pageSize?: number, window?: TopupWindow } = {}, options: RequestOptions = {}) {
  return request<BillingTopupPage>({
    url: '/api/admin/users/billing/topups',
    method: 'GET',
    params: { userId, ...params },
    ...options,
  })
}

export function revokeAdminUserSubscription(userId: string, options: RequestOptions = {}) {
  return request<AdminSubscription | null>({
    url: '/api/admin/users/subscription/revoke',
    method: 'POST',
    data: { userId },
    ...options,
  })
}
