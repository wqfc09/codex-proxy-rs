import { computed, shallowRef } from 'vue'

import {
  getAdminSessionTtlSettings,
  getAdminTurnstileSettings,
  updateAdminSessionTtlSettings,
  updateAdminTurnstileSettings,
} from '@/api'
import { toast } from '@/components/base/BaseToast'

const MAX_SESSION_TTL_MINUTES = 366 * 24 * 60

export function useLoginSecuritySettings() {
  const loading = shallowRef(false)
  const sessionSaving = shallowRef(false)
  const turnstileSaving = shallowRef(false)
  const error = shallowRef('')
  const adminMinutes = shallowRef('')
  const userMinutes = shallowRef('')
  const turnstileEnabled = shallowRef(false)
  const turnstileSiteKey = shallowRef('')
  const turnstileSecretKey = shallowRef('')
  const turnstileHasSecret = shallowRef(false)

  const sessionValid = computed(() => {
    const values = [Number(adminMinutes.value), Number(userMinutes.value)]
    return values.every(value => Number.isInteger(value) && value >= 1 && value <= MAX_SESSION_TTL_MINUTES)
  })

  const turnstileValid = computed(() => !turnstileEnabled.value
    || (Boolean(turnstileSiteKey.value.trim()) && (turnstileHasSecret.value || Boolean(turnstileSecretKey.value.trim()))))

  async function load() {
    if (loading.value)
      return
    loading.value = true
    error.value = ''
    try {
      const [session, turnstile] = await Promise.all([
        getAdminSessionTtlSettings({ silent: true }),
        getAdminTurnstileSettings({ silent: true }),
      ])
      adminMinutes.value = String(session.adminMinutes)
      userMinutes.value = String(session.userMinutes)
      turnstileEnabled.value = turnstile.enabled
      turnstileSiteKey.value = turnstile.siteKey ?? ''
      turnstileSecretKey.value = ''
      turnstileHasSecret.value = turnstile.hasSecret
    }
    catch {
      error.value = '登录与安全设置加载失败，请稍后重试'
    }
    finally {
      loading.value = false
    }
  }

  async function saveSessionTtl() {
    if (sessionSaving.value || !sessionValid.value)
      return
    sessionSaving.value = true
    try {
      const settings = await updateAdminSessionTtlSettings({
        adminMinutes: Number(adminMinutes.value),
        userMinutes: Number(userMinutes.value),
      })
      adminMinutes.value = String(settings.adminMinutes)
      userMinutes.value = String(settings.userMinutes)
      toast.success('Session TTL 已更新')
    }
    finally {
      sessionSaving.value = false
    }
  }

  async function saveTurnstile() {
    if (turnstileSaving.value || !turnstileValid.value)
      return
    turnstileSaving.value = true
    try {
      const secret = turnstileSecretKey.value.trim()
      const settings = await updateAdminTurnstileSettings({
        enabled: turnstileEnabled.value,
        siteKey: turnstileSiteKey.value.trim() || null,
        secretKey: secret || undefined,
      })
      turnstileEnabled.value = settings.enabled
      turnstileSiteKey.value = settings.siteKey ?? ''
      turnstileSecretKey.value = ''
      turnstileHasSecret.value = settings.hasSecret
      toast.success('Turnstile 设置已更新')
    }
    finally {
      turnstileSaving.value = false
    }
  }

  return {
    loading,
    sessionSaving,
    turnstileSaving,
    error,
    adminMinutes,
    userMinutes,
    turnstileEnabled,
    turnstileSiteKey,
    turnstileSecretKey,
    turnstileHasSecret,
    sessionValid,
    turnstileValid,
    load,
    saveSessionTtl,
    saveTurnstile,
  }
}
