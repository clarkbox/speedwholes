import { ref, readonly } from 'vue'

// Cached config - loaded once and shared across all components
let configPromise = null
const config = ref(null)
const configError = ref(null)
const configLoading = ref(false)

/**
 * Load config from /config.json (runtime configuration)
 * Config is loaded once and cached for subsequent calls
 */
async function loadConfig() {
  if (configPromise) {
    return configPromise
  }

  configLoading.value = true
  
  configPromise = fetch('/config.json')
    .then(async (response) => {
      if (!response.ok) {
        throw new Error(`Failed to load config: ${response.status}`)
      }
      const data = await response.json()
      config.value = data
      configError.value = null
      return data
    })
    .catch((error) => {
      console.error('Failed to load config.json:', error)
      configError.value = error.message
      // Provide fallback config
      config.value = {
        apiUrl: `${window.location.protocol}//${window.location.hostname}:3000`
      }
      return config.value
    })
    .finally(() => {
      configLoading.value = false
    })

  return configPromise
}

/**
 * Get the API URL - waits for config to load if needed
 */
async function getApiUrl() {
  const cfg = await loadConfig()
  return cfg.apiUrl
}

/**
 * Composable for accessing runtime config
 * Call loadConfig() early in app initialization, then use config.value
 */
export function useConfig() {
  return {
    config: readonly(config),
    configError: readonly(configError),
    configLoading: readonly(configLoading),
    loadConfig,
    getApiUrl
  }
}
