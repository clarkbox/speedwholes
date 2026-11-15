<template>
  <div class="login-view">
    <div class="login-card" :class="{ shake: isShaking }">
      <div class="logo">
        <div class="logo-icon">⚡</div>
        <h1>speedwholes</h1>
        <p>Admin Configuration Panel</p>
      </div>
      
      <div v-if="errorMessage" class="error-message">
        {{ errorMessage }}
        <a v-if="certErrorUrl" :href="certErrorUrl" target="_blank" class="cert-link">
          Click here to accept the certificate
        </a>
      </div>
      
      <form @submit.prevent="handleLogin">
        <div class="form-group">
          <label for="token">Admin Token</label>
          <input 
            type="password" 
            id="token" 
            v-model="token"
            placeholder="Enter your admin token"
            autocomplete="current-password"
            required
            ref="tokenInput"
          >
        </div>
        
        <button type="submit" class="submit-btn">
          Sign In
        </button>
      </form>
      
      <p class="footer-text">
        Token is set via admin_token in speedwholes.config.json
      </p>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { useAuth } from '../composables/useAuth'

const router = useRouter()
const route = useRoute()
const { login } = useAuth()

const token = ref('')
const errorMessage = ref('')
const certErrorUrl = ref('')
const isShaking = ref(false)
const tokenInput = ref(null)

const handleLogin = async () => {
  errorMessage.value = ''
  certErrorUrl.value = ''
  
  const result = await login(token.value)
  
  if (result.success) {
    // Login successful, redirect to settings page
    router.push('/settings')
  } else {
    // Login failed, show error
    errorMessage.value = result.error || 'Invalid admin token. Please try again.'
    
    // If it's a certificate error, provide the URL to visit
    if (result.certError && result.serverUrl) {
      certErrorUrl.value = result.serverUrl
    }
    
    isShaking.value = true
    token.value = ''
    setTimeout(() => {
      isShaking.value = false
    }, 300)
    
    // Focus input after error
    if (tokenInput.value) {
      tokenInput.value.focus()
    }
  }
}

onMounted(() => {
  // Check for error param in URL
  if (route.query.error === '1') {
    errorMessage.value = 'Invalid admin token. Please try again.'
    isShaking.value = true
    setTimeout(() => {
      isShaking.value = false
    }, 300)
  }
  
  // Focus input on load
  if (tokenInput.value) {
    tokenInput.value.focus()
  }
})
</script>

<style scoped>
.login-view {
  min-height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 2rem;
}

.login-card {
  width: 100%;
  max-width: 420px;
  background: linear-gradient(135deg, rgba(30, 30, 40, 0.9) 0%, rgba(20, 20, 30, 0.95) 100%);
  border: 1px solid rgba(139, 92, 246, 0.2);
  border-radius: 24px;
  padding: 3rem 2.5rem;
  box-shadow: 
    0 4px 60px rgba(0, 0, 0, 0.5),
    0 0 0 1px rgba(255, 255, 255, 0.03) inset,
    0 0 80px rgba(139, 92, 246, 0.1);
  position: relative;
  overflow: hidden;
}

.login-card::before {
  content: '';
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 1px;
  background: linear-gradient(90deg, transparent, rgba(139, 92, 246, 0.5), transparent);
}

.logo {
  text-align: center;
  margin-bottom: 2rem;
}

.logo-icon {
  width: 64px;
  height: 64px;
  background: linear-gradient(135deg, #8b5cf6 0%, #06b6d4 100%);
  border-radius: 16px;
  display: flex;
  align-items: center;
  justify-content: center;
  margin: 0 auto 1rem;
  font-size: 2rem;
  box-shadow: 0 8px 32px rgba(139, 92, 246, 0.3);
}

.logo h1 {
  font-family: 'Outfit', sans-serif;
  font-size: 1.5rem;
  font-weight: 600;
  background: linear-gradient(135deg, #8b5cf6 0%, #06b6d4 100%);
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
  background-clip: text;
  letter-spacing: -0.02em;
}

.logo p {
  color: #71717a;
  font-size: 0.875rem;
  margin-top: 0.5rem;
}

.error-message {
  background: rgba(239, 68, 68, 0.1);
  border: 1px solid rgba(239, 68, 68, 0.3);
  color: #fca5a5;
  padding: 0.875rem 1rem;
  border-radius: 12px;
  margin-bottom: 1.5rem;
  font-size: 0.875rem;
}

.cert-link {
  display: block;
  margin-top: 0.75rem;
  padding: 0.5rem 0.75rem;
  background: rgba(139, 92, 246, 0.2);
  border: 1px solid rgba(139, 92, 246, 0.4);
  border-radius: 8px;
  color: #c4b5fd;
  text-decoration: none;
  font-weight: 500;
  transition: all 0.2s ease;
}

.cert-link:hover {
  background: rgba(139, 92, 246, 0.3);
  color: #ddd6fe;
}

.form-group {
  margin-bottom: 1.5rem;
}

.form-group label {
  display: block;
  color: #a1a1aa;
  font-size: 0.8rem;
  font-weight: 500;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  margin-bottom: 0.5rem;
}

.form-group input {
  width: 100%;
  padding: 1rem 1.25rem;
  background: rgba(0, 0, 0, 0.3);
  border: 1px solid rgba(113, 113, 122, 0.3);
  border-radius: 12px;
  color: #e4e4e7;
  font-family: 'IBM Plex Mono', monospace;
  font-size: 1rem;
  transition: all 0.2s ease;
}

.form-group input:focus {
  outline: none;
  border-color: rgba(139, 92, 246, 0.6);
  box-shadow: 0 0 0 3px rgba(139, 92, 246, 0.15);
  background: rgba(0, 0, 0, 0.4);
}

.form-group input::placeholder {
  color: #52525b;
}

.submit-btn {
  width: 100%;
  padding: 1rem 1.5rem;
  background: linear-gradient(135deg, #8b5cf6 0%, #7c3aed 100%);
  border: none;
  border-radius: 12px;
  color: white;
  font-family: 'Outfit', sans-serif;
  font-size: 1rem;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.2s ease;
  position: relative;
  overflow: hidden;
}

.submit-btn:hover {
  transform: translateY(-2px);
  box-shadow: 0 8px 24px rgba(139, 92, 246, 0.4);
}

.submit-btn:active {
  transform: translateY(0);
}

.submit-btn::after {
  content: '';
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: linear-gradient(135deg, rgba(255,255,255,0.1) 0%, transparent 50%);
  pointer-events: none;
}

.footer-text {
  text-align: center;
  margin-top: 1.5rem;
  color: #52525b;
  font-size: 0.75rem;
}
</style>
