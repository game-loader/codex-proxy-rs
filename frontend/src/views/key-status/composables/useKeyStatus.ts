import type { KeyUsageStatus } from '@/api/modules/key-usage'
import { useDocumentVisibility, useIntervalFn } from '@vueuse/core'
import { shallowRef, watch } from 'vue'
import { getKeyUsageStatus } from '@/api/modules/key-usage'
import { useRequestState } from '@/composables/useRequestState'

export function useKeyStatus() {
  const snapshot = shallowRef<KeyUsageStatus>()
  const request = useRequestState()

  async function refresh() {
    if (request.loading.value)
      return

    const id = request.start()
    try {
      const result = await getKeyUsageStatus({ signal: request.signal, silent: true, timeout: 10_000 })
      if (request.isCurrent(id))
        snapshot.value = result
    }
    catch (cause) {
      if (request.isCurrent(id))
        snapshot.value = undefined
      request.fail(id, cause)
    }
    finally {
      request.finish(id)
    }
  }

  const visibility = useDocumentVisibility()
  const { pause, resume } = useIntervalFn(refresh, 30_000, { immediate: false, immediateCallback: true })
  watch(visibility, (value) => {
    if (value === 'visible')
      resume()
    else
      pause()
  }, { immediate: true })

  return {
    snapshot,
    refresh,
    loading: request.loading,
    error: request.error,
  }
}
