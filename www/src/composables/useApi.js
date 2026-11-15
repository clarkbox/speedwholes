import { useRouter } from 'vue-router'
import { useConfig } from './useConfig'

export function useApi() {
  const router = useRouter()
  const { getApiUrl } = useConfig()
  
  const authFetch = async (url, options = {}) => {
    const apiUrl = await getApiUrl()
    const fullUrl = url.startsWith('http') ? url : `${apiUrl}${url}`
    
    // Get the session token from localStorage
    const token = localStorage.getItem('speedwholes_session_token')
    
    // Add Authorization header with Bearer token if available
    const headers = {
      ...options.headers,
    }
    
    if (token) {
      headers['Authorization'] = `Bearer ${token}`
    }
    
    const response = await fetch(fullUrl, { 
      ...options,
      headers
    })
    
    if (response.status === 401) {
      // Session invalid or expired, clear auth and redirect to login
      localStorage.removeItem('speedwholes_session_token')
      sessionStorage.removeItem('authenticated')
      router.push('/login')
    }
    
    return response
  }
  
  return {
    authFetch
  }
}
