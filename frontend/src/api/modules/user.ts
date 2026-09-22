import type { RequestOptions } from '../request'
import type { ApiKey } from './api-keys'
import request from '../request'

export interface SubscriptionPlan {
  id: string
  name: string
  description: string | null
  enabled: boolean
  isBase: boolean
  dailyLimitUsd: string | null
  weeklyLimitUsd: string | null
  monthlyLimitUsd: string | null
  createdAt: string
  updatedAt: string
}

export interface Subscription {
  id: string
  planId: string
  planName: string
  multiplier: string
  status: 'active' | 'revoked'
  effectiveStatus: 'active' | 'pending' | 'expired' | 'revoked'
  startsAt: string
  expiresAt: string
  revokedAt: string | null
  createdAt: string
  updatedAt: string
}

export interface AdminSubscription extends Subscription {
  userId: string | null
}

export type UserSubscription = Subscription

export interface UserBudgetWindow {
  limit: string | null
  used: string
  credit: string
  effectiveLimit: string | null
  resetsAt: string
}

export interface UserBudgetStatus {
  daily: UserBudgetWindow
  weekly: UserBudgetWindow
  monthly: UserBudgetWindow
}

export interface UserGroup {
  groupId: string
  name: string
  enabled: boolean
}

export interface UserBillingSummary {
  subscription: UserSubscription | null
  plan: SubscriptionPlan | null
  effectiveSource: 'basePlan' | 'subscription'
  history: UserSubscription[]
  groups: UserGroup[]
  multiplier: string
  effectiveMaxConcurrency: number | null
  effectiveRequestsPerMinute: number | null
  budget: UserBudgetStatus
}

export interface UserSubscriptionPage {
  items: UserSubscription[]
  page: number
  pageSize: number
  total: number
}

export interface AdminBillingSummary extends Omit<UserBillingSummary, 'subscription' | 'history'> {
  subscription: AdminSubscription | null
  history: AdminSubscription[]
  groups: UserGroup[]
}

export interface UserUsageRecord {
  id: string
  clientApiKeyId: string
  clientApiKeyName: string
  operation: string
  requestKind: string | null
  requestedModel: string | null
  inputTokens: number | null
  outputTokens: number | null
  cachedTokens: number | null
  cacheWriteTokens: number | null
  reasoningTokens: number | null
  imageInputTokens: number | null
  imageOutputTokens: number | null
  totalTokens: number | null
  imageRequestedSize: string | null
  imageRequestedCount: number | null
  imageOutputSize: string | null
  imageCount: number | null
  imageBillingTier: string | null
  downstreamRateMultiplier: string | null
  billedAmountUsd: string | null
  outcome: string
  clientStatusCode: number | null
  latencyMs: number | null
  startedAt: string
  completedAt: string | null
}

export interface UserUsagePage {
  items: UserUsageRecord[]
  currentPage: number
  pageSize: number
  total: number
}

export interface UserUsageDailyPoint {
  bucketStart: string
  requestCount: number
  successCount: number
  failureCount: number
  totalTokens: number
  billedUsd: string | null
  billedKnownCount: number
  billedUnknownCount: number
}

export interface UserUsageBreakdown {
  id: string | null
  name: string
  requestCount: number
  totalTokens: number
  isOther: boolean
}

export interface UserUsageSummary {
  startTime: string
  endTime: string
  requestCount: number
  successCount: number
  failureCount: number
  totalTokens: number
  billedUsd: string | null
  billedKnownCount: number
  billedUnknownCount: number
  inputTokens: number
  outputTokens: number
  cachedTokens: number
  averageLatencyMs: number | null
  trendGranularity: '1h' | '1d'
  trend: UserUsageDailyPoint[]
  daily: UserUsageDailyPoint[]
  models: UserUsageBreakdown[]
  clientKeys: UserUsageBreakdown[]
}

export interface UserUsageQuery {
  currentPage?: number
  pageSize?: number
  clientApiKeyId?: string
  model?: string
  outcome?: string
  statusCode?: number
  startTime?: string
  endTime?: string
}

export function getUserBilling(options: RequestOptions = {}) {
  return request<UserBillingSummary>({
    url: '/api/user/billing',
    method: 'GET',
    ...options,
  })
}

export function getUserSubscriptionHistory(params: { page: number, pageSize: number }, options: RequestOptions = {}) {
  return request<UserSubscriptionPage>({
    url: '/api/user/billing/subscriptions',
    method: 'GET',
    params,
    ...options,
  })
}

export function getUserUsageRecords(params: UserUsageQuery, options: RequestOptions = {}) {
  return request<UserUsagePage>({
    url: '/api/user/usage/records',
    method: 'GET',
    params,
    ...options,
  })
}

export function getUserUsageDetail(id: string, options: RequestOptions = {}) {
  return request<UserUsageRecord>({
    url: '/api/user/usage/records/detail',
    method: 'GET',
    params: { id },
    ...options,
  })
}

export function getUserUsageSummary(params: Omit<UserUsageQuery, 'currentPage' | 'pageSize'>, options: RequestOptions = {}) {
  return request<UserUsageSummary>({
    url: '/api/user/usage/summary',
    method: 'GET',
    params,
    ...options,
  })
}

export type UserClientKey = ApiKey
