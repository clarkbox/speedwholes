import { computed } from 'vue'

// Parse subdomain from hostname (same logic as backend)
function parseSubdomain(hostname) {
  // IP addresses (v4) are treated as DEVLOCAL
  if (/^(\d{1,3}\.){3}\d{1,3}$/.test(hostname)) {
    return 'DEVLOCAL'
  }
  // localhost is DEVLOCAL
  if (hostname === 'localhost') {
    return 'DEVLOCAL'
  }
  // Split by dots - need at least 3 parts for a subdomain
  const parts = hostname.split('.')
  if (parts.length >= 3) {
    // Return everything except the last two parts
    return parts.slice(0, -2).join('.')
  }
  return null
}

export function useSubdomain() {
  const currentSubdomain = computed(() => parseSubdomain(window.location.hostname))
  const currentOrigin = computed(() => window.location.origin)
  
  return {
    currentSubdomain,
    currentOrigin,
    parseSubdomain
  }
}
