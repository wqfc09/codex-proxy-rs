import type { RequestOptions } from '../request'
import request from '../request'

export type ApiKeyBudgetPeriod = 'daily' | 'weekly' | 'all'

export interface ApiKey {

  id: string
  name: string
  label: string | null
  prefix: string
  enabled: boolean
  maxConcurrency: number
  requestsPerMinute: number
  dailyLimitUsd: string
  weeklyLimitUsd: string
  dailyUsedUsd: string
  weeklyUsedUsd: string
  dailyResetsAt: string | null
  weeklyResetsAt: string | null
  createdAt: string
  updatedAt: string
  lastUsedAt: string | null
  providerKinds: string[]
}

export interface ApiKeyListResponse {
  items: ApiKey[]
  nextCursor: string | null
  total: number
}

export interface ApiKeyCreateResponse {
  id: string
  prefix: string
  plaintextKey: string
}

export interface ApiKeyRevealResponse {
  id: string
  plaintextKey: string
}

export interface ApiKeyMutationResponse {
  id: string
}

// 请求参数类型：仅定义 API 边界的形状，调用方不依赖显式声明。
interface ApiKeyListParams {
  cursor?: string
  limit: number
  search?: string
  sortBy?: string
  sortDirection?: string
}

export interface ApiKeyWriteParam {

  name: string
  label: string | null
  maxConcurrency: number
  requestsPerMinute: number
  dailyLimitUsd: string
  weeklyLimitUsd: string
}

interface ApiKeyUpdateParam extends ApiKeyWriteParam {
  id: string
}

interface ApiKeyCreateParam extends ApiKeyWriteParam {
  customKey?: string
}

interface ApiKeyIdParam {
  id: string
}

export function getApiKeys(data: ApiKeyListParams = { limit: 100 }, options: RequestOptions = {}) {
  return request<ApiKeyListResponse>({
    url: '/api/user/client-keys',
    method: 'GET',
    params: data,
    ...options,
  })
}

export function createApiKey(data: ApiKeyCreateParam) {
  return request<ApiKeyCreateResponse>({
    url: '/api/user/client-keys/create',
    method: 'POST',
    data,
  })
}

export function updateApiKey(data: ApiKeyUpdateParam) {
  return request<ApiKeyMutationResponse>({
    url: '/api/user/client-keys/update',
    method: 'POST',
    data,
  })
}

export function revealApiKey(data: ApiKeyIdParam) {
  return request<ApiKeyRevealResponse>({
    url: '/api/user/client-keys/reveal',
    method: 'GET',
    params: data,
  })
}

export function deleteApiKey(data: ApiKeyIdParam) {
  return request<ApiKeyMutationResponse>({
    url: '/api/user/client-keys/delete',
    method: 'POST',
    data,
  })
}

export function resetApiKeyBudget(data: ApiKeyIdParam & { period: ApiKeyBudgetPeriod }) {
  return request<ApiKeyMutationResponse>({
    url: '/api/user/client-keys/reset-budget',
    method: 'POST',
    data,
  })
}

export function disableApiKey(data: ApiKeyIdParam) {
  return request<ApiKeyMutationResponse>({
    url: '/api/user/client-keys/disable',
    method: 'POST',
    data,
  })
}

export function enableApiKey(data: ApiKeyIdParam) {
  return request<ApiKeyMutationResponse>({
    url: '/api/user/client-keys/enable',
    method: 'POST',
    data,
  })
}
