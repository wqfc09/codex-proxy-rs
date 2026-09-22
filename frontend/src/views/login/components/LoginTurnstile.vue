<script setup lang="ts">
import { onBeforeUnmount, ref, shallowRef, watch } from 'vue'

type ThemeName = 'light' | 'dark'
type TurnstileWidgetId = string | number

interface TurnstileApi {
  render: (
    container: HTMLElement,
    options: {
      'sitekey': string
      'theme': ThemeName
      'callback': (token: string) => void
      'expired-callback': () => void
      'error-callback': () => void
    },
  ) => TurnstileWidgetId
  reset: (widgetId: TurnstileWidgetId) => void
  remove: (widgetId: TurnstileWidgetId) => void
}

declare global {
  interface Window {
    turnstile?: TurnstileApi
  }
}

const props = defineProps<{
  enabled: boolean
  siteKey: string | null
  theme: ThemeName
}>()
const token = defineModel<string>({ required: true })
const containerRef = ref<HTMLElement | null>(null)
const loading = shallowRef(false)
const errorMessage = shallowRef('')
let widgetId: TurnstileWidgetId | null = null
let disposed = false
let renderGeneration = 0
let scriptPromise: Promise<TurnstileApi> | null = null
let pendingRefresh: { resolve: (value: string | null) => void, timeoutId: number } | null = null

function loadTurnstile(): Promise<TurnstileApi> {
  if (window.turnstile)
    return Promise.resolve(window.turnstile)
  if (scriptPromise)
    return scriptPromise

  const loadingPromise = new Promise<TurnstileApi>((resolve, reject) => {
    document.querySelector<HTMLScriptElement>('script[data-cpr-turnstile]')?.remove()
    const script = document.createElement('script')
    script.src = 'https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit'
    script.async = true
    script.defer = true
    script.dataset.cprTurnstile = 'true'
    script.addEventListener('load', () => {
      if (window.turnstile)
        resolve(window.turnstile)
      else reject(new Error('Turnstile API unavailable after script load'))
    }, { once: true })
    script.addEventListener('error', () => reject(new Error('Turnstile script failed to load')), { once: true })
    document.head.append(script)
  }).catch((error): never => {
    scriptPromise = null
    throw error
  })
  scriptPromise = loadingPromise
  return loadingPromise
}

function settlePendingRefresh(value: string | null) {
  if (!pendingRefresh)
    return
  window.clearTimeout(pendingRefresh.timeoutId)
  const { resolve } = pendingRefresh
  pendingRefresh = null
  resolve(value)
}

function acceptToken(value: string) {
  token.value = value
  errorMessage.value = ''
  settlePendingRefresh(value)
}

function clearToken() {
  token.value = ''
}

function handleWidgetError() {
  clearToken()
  errorMessage.value = '登录保护加载失败，请刷新页面后重试'
  settlePendingRefresh(null)
}

function removeWidget() {
  if (widgetId !== null && window.turnstile) {
    try {
      window.turnstile.remove(widgetId)
    }
    catch {
      // Widget 已由 Cloudflare 清理时忽略重复 remove。
    }
  }
  widgetId = null
}

async function renderWidget() {
  const generation = ++renderGeneration
  settlePendingRefresh(null)
  clearToken()
  removeWidget()
  errorMessage.value = ''

  if (!props.enabled)
    return
  if (!props.siteKey) {
    errorMessage.value = '登录保护配置不可用，请联系管理员'
    return
  }

  loading.value = true
  try {
    const api = await loadTurnstile()
    if (disposed || generation !== renderGeneration || !containerRef.value)
      return
    widgetId = api.render(containerRef.value, {
      'sitekey': props.siteKey,
      'theme': props.theme,
      'callback': acceptToken,
      'expired-callback': clearToken,
      'error-callback': handleWidgetError,
    })
  }
  catch {
    if (generation === renderGeneration)
      handleWidgetError()
  }
  finally {
    if (generation === renderGeneration)
      loading.value = false
  }
}

function reset() {
  settlePendingRefresh(null)
  clearToken()
  if (widgetId === null || !window.turnstile)
    return
  try {
    window.turnstile.reset(widgetId)
  }
  catch {
    handleWidgetError()
  }
}

function resetAndWait(): Promise<string | null> {
  clearToken()
  if (widgetId === null || !window.turnstile)
    return Promise.resolve(null)

  settlePendingRefresh(null)
  return new Promise((resolve) => {
    pendingRefresh = {
      resolve,
      timeoutId: window.setTimeout(() => {
        errorMessage.value = '登录保护验证超时，请重试'
        settlePendingRefresh(null)
      }, 120_000),
    }
    try {
      window.turnstile?.reset(widgetId as TurnstileWidgetId)
    }
    catch {
      handleWidgetError()
    }
  })
}

watch(
  [() => props.enabled, () => props.siteKey, () => props.theme],
  () => void renderWidget(),
  { immediate: true, flush: 'post' },
)

onBeforeUnmount(() => {
  disposed = true
  renderGeneration += 1
  settlePendingRefresh(null)
  removeWidget()
})

defineExpose({ reset, resetAndWait })
</script>

<template>
  <div v-if="enabled" class="grid min-w-0 gap-2" aria-live="polite">
    <span class="text-cp leading-[1.1] font-bold text-(--cp-login-label-color)">人机验证</span>
    <div ref="containerRef" class="min-h-16 min-w-0 overflow-hidden rounded-cp" />
    <p v-if="errorMessage" class="m-0 text-cp-sm font-emphasis text-cp-error-text">
      {{ errorMessage }}
    </p>
    <p v-else-if="loading" class="m-0 text-cp-sm font-emphasis text-(--cp-login-description-color)">
      正在加载登录保护…
    </p>
  </div>
</template>
