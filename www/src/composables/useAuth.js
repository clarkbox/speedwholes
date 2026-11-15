import { ref } from 'vue'
import { useRouter } from 'vue-router'
import { useConfig } from './useConfig'

const isAuthenticated = ref(false)

export function useAuth() {
  const router = useRouter()
  const { getApiUrl } = useConfig()
  
  const login = async (adminToken) => {
    try {
      const apiUrl = await getApiUrl()
      
      // Call login endpoint with admin token to get a session token
      const response = await fetch(`${apiUrl}/config007/login`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({ token: adminToken })
      })
      
      if (response.ok) {
        const data = await response.json()
        // Store the session token (not the admin token)
        localStorage.setItem('speedwholes_session_token', data.session_token)
        sessionStorage.setItem('authenticated', 'true')
        isAuthenticated.value = true
        return { success: true }
      } else if (response.status === 401) {
        isAuthenticated.value = false
        const data = await response.json().catch(() => ({}))
        return { success: false, error: data.error || 'Invalid admin token' }
      } else {
        isAuthenticated.value = false
        return { success: false, error: `Server error: ${response.status}` }
      }
    } catch (error) {
      console.error('Login error:', error)
      isAuthenticated.value = false
      
      const apiUrl = await getApiUrl()
      
      // Check if it's likely a certificate error (TypeError: Failed to fetch on HTTPS)
      if (error instanceof TypeError && error.message === 'Failed to fetch' && apiUrl.startsWith('https://')) {
        return { 
          success: false, 
          error: 'Certificate error: The server may be using an untrusted certificate.',
          certError: true,
          serverUrl: apiUrl
        }
      }
      
      return { success: false, error: 'Network error. Please check your connection.' }
    }
  }
  
  const logout = async () => {
    try {
      const apiUrl = await getApiUrl()
      const sessionToken = localStorage.getItem('speedwholes_session_token')
      
      // Call logout endpoint to invalidate session on server
      if (sessionToken) {
        await fetch(`${apiUrl}/config007/logout`, {
          method: 'POST',
          headers: {
            'Authorization': `Bearer ${sessionToken}`,
          }
        }).catch(() => {}) // Ignore errors - we're logging out anyway
      }
    } finally {
      // Always clear local state
      localStorage.removeItem('speedwholes_session_token')
      // Also clear old key if present (migration)
      localStorage.removeItem('speedwholes_admin_token')
      sessionStorage.removeItem('authenticated')
      isAuthenticated.value = false
      router.push('/login')
    }
  }
  
  const checkAuth = () => {
    const hasToken = !!localStorage.getItem('speedwholes_session_token')
    const wasAuthenticated = sessionStorage.getItem('authenticated') === 'true'
    isAuthenticated.value = hasToken && wasAuthenticated
    return isAuthenticated.value
  }
  
  const getToken = () => {
    return localStorage.getItem('speedwholes_session_token')
  }
  
  return {
    isAuthenticated,
    login,
    logout,
    checkAuth,
    getToken
  }
}
