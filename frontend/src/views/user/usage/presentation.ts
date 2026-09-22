import type { UsageListRecord, UserUsageRecord } from '@/api'
import { formatCompactNumber } from '@/utils/number'
import { displayDate } from '../utils'

export interface UserUsageDisplayRecord extends UsageListRecord {
  outcome: string
  clientStatusCode: number | null
  original: UserUsageRecord
}
const number = (value: number | null) => value === null ? '—' : formatCompactNumber(value)

// 只映射 User API 已授权字段；管理员专有字段保持未知，不从管理接口补全。
export function presentUserUsage(record: UserUsageRecord): UserUsageDisplayRecord {
  return {
    id: record.id,
    userId: null,
    username: null,
    clientApiKeyId: record.clientApiKeyId,
    clientApiKeyName: record.clientApiKeyName,
    subscriptionId: null,
    billingGroupId: null,
    downstreamRateMultiplier: record.downstreamRateMultiplier,
    downstreamBilledAmount: record.billedAmountUsd,
    imageRequestedSize: record.imageRequestedSize,
    imageRequestedCount: record.imageRequestedCount,
    imageOutputSize: record.imageOutputSize,
    imageCount: record.imageCount,
    imageBillingTier: record.imageBillingTier,
    imageBaseCostSource: null,
    provider: null,
    authenticationKind: null,
    accountId: null,
    accountEmail: null,
    accountName: null,
    accountNotes: null,
    route: record.operation,
    model: record.requestedModel,
    requestedModel: record.requestedModel,
    upstreamModel: null,
    upstreamResponseModel: null,
    serviceTier: null,
    clientTransport: '',
    upstreamTransport: null,
    reasoningEffort: null,
    reasoningPreset: null,
    subagentKind: null,
    compact: record.requestKind === 'compact',
    tokenDetails: {
      inputTokens: record.inputTokens,
      outputTokens: record.outputTokens,
      cachedTokens: record.cachedTokens,
      cacheWriteTokens: record.cacheWriteTokens,
      reasoningTokens: record.reasoningTokens,
      imageInputTokens: record.imageInputTokens,
      imageOutputTokens: record.imageOutputTokens,
      totalTokens: record.totalTokens,
      inputTokensDisplay: number(record.inputTokens),
      outputTokensDisplay: number(record.outputTokens),
      cachedTokensDisplay: number(record.cachedTokens),
      cacheWriteTokensDisplay: number(record.cacheWriteTokens),
      reasoningTokensDisplay: number(record.reasoningTokens),
      imageInputTokensDisplay: number(record.imageInputTokens),
      imageOutputTokensDisplay: number(record.imageOutputTokens),
      totalTokensDisplay: number(record.totalTokens),
    },
    billing: null,
    latencyDetails: {},
    firstTokenLatencyMs: null,
    latencyMs: record.latencyMs,
    createdAt: record.startedAt,
    createdAtDisplay: displayDate(record.startedAt),
    clientIp: null,
    userAgent: null,
    outcome: record.outcome,
    clientStatusCode: record.clientStatusCode,
    original: record,
  }
}
