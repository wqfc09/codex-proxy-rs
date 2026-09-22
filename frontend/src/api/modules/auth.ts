import type { RequestOptions } from '../request'
import request from '../request'

export type UserRole = 'admin' | 'user'

export interface SessionUser {
  id: string
  username: string
  role: UserRole
  enabled: boolean
  maxConcurrency: number | null
  requestsPerMinute: number | null
}

export interface AuthSession {
  role: UserRole
  expiresAt: string
}

export interface AuthStatusResponse {
  authenticated: boolean
  session: AuthSession | null
}

export interface LoginParam {
  username: string
  password: string
  turnstileToken?: string
}

export interface PublicAuthConfig {
  turnstileEnabled: boolean
  turnstileSiteKey: string | null
}

export interface AdminSessionTtlSettings {
  adminMinutes: number
  userMinutes: number
}

export interface AdminTurnstileSettings {
  enabled: boolean
  siteKey: string | null
  hasSecret: boolean
}

export interface LogoutResponse {
  message?: string
}

export function login(data: LoginParam, options: RequestOptions = {}) {
  return request<AuthSession>({
    url: '/api/auth/login',
    method: 'POST',
    data,
    ...options,
  })
}

export function getAuthStatus(options: RequestOptions = {}) {
  return request<AuthStatusResponse>({
    url: '/api/auth/status',
    method: 'GET',
    ...options,
  })
}

export function logout(options: RequestOptions = {}) {
  return request<LogoutResponse>({
    url: '/api/auth/logout',
    method: 'POST',
    ...options,
  })
}

export function getAdminSessionTtlSettings(options: RequestOptions = {}) {
  return request<AdminSessionTtlSettings>({
    url: '/api/admin/auth/session',
    method: 'GET',
    ...options,
  })
}

export function updateAdminSessionTtlSettings(data: AdminSessionTtlSettings, options: RequestOptions = {}) {
  return request<AdminSessionTtlSettings>({
    url: '/api/admin/auth/session/update',
    method: 'POST',
    data,
    ...options,
  })
}

export function getAdminTurnstileSettings(options: RequestOptions = {}) {
  return request<AdminTurnstileSettings>({
    url: '/api/admin/auth/turnstile',
    method: 'GET',
    ...options,
  })
}

export function updateAdminTurnstileSettings(
  data: { enabled: boolean, siteKey?: string | null, secretKey?: string },
  options: RequestOptions = {},
) {
  return request<AdminTurnstileSettings>({
    url: '/api/admin/auth/turnstile/update',
    method: 'POST',
    data,
    ...options,
  })
}

export function getPublicAuthConfig(options: RequestOptions = {}) {
  return request<PublicAuthConfig>({
    url: '/api/auth/config',
    method: 'GET',
    ...options,
  })
}

export function getCurrentUser(options: RequestOptions = {}) {
  return request<SessionUser>({
    url: '/api/user/me',
    method: 'GET',
    ...options,
  })
}

export function updateCurrentUsername(data: { currentPassword: string, username: string }, options: RequestOptions = {}) {
  return request<SessionUser>({
    url: '/api/user/profile',
    method: 'POST',
    data,
    ...options,
  })
}

export function updateCurrentPassword(data: { currentPassword: string, newPassword: string }, options: RequestOptions = {}) {
  return request<void>({
    url: '/api/user/password',
    method: 'POST',
    data,
    ...options,
  })
}

export function changeAdminPassword(data: { currentPassword: string, newPassword: string }, options: RequestOptions = {}) {
  return request<{ message: string }>({
    url: '/api/auth/password',
    method: 'POST',
    data,
    ...options,
  })
}
