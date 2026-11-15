<template>
  <div class="admin-layout">
    <div class="page">
      <!-- Auth Bar -->
      <div class="auth-bar auth-ok">
        <span class="auth-icon">⚡</span>
        <div style="flex: 1; font-weight: 600; color: #7fdbca;">speedwholes admin</div>
        <div class="auth-dropdown">
          <span class="auth-status authenticated">● Authenticated</span>
          <div class="auth-dropdown-menu">
            <button @click="handleLogout" class="logout-btn">🚪 Logout</button>
          </div>
        </div>
      </div>

      <!-- Tab Navigation -->
      <div class="tab-nav">
        <router-link to="/settings" class="tab-btn">📊 Monitor</router-link>
        <router-link to="/subdomains" class="tab-btn">🌐 Subdomains</router-link>
        <router-link to="/logs" class="tab-btn">📋 Logs</router-link>
      </div>

      <!-- Slot for page content -->
      <slot></slot>

      <!-- Footer -->
      <div class="footer" style="display: flex; justify-content: space-between; align-items: center;">
        <span>speedwholes v0.1.0</span>
        <button @click="handleLogout" class="logout-btn">🚪 Logout</button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { useAuth } from '../composables/useAuth'

const { logout } = useAuth()

const handleLogout = () => {
  logout()
}
</script>

<style scoped>
.admin-layout {
  padding: 2rem;
}

.page {
  max-width: 900px;
  margin: 0 auto;
}

/* Auth Bar */
.auth-bar {
  background: rgba(18, 18, 28, 0.95);
  border: 1px solid #2a2a3a;
  border-radius: 12px;
  padding: 1rem 1.5rem;
  margin-bottom: 1.5rem;
  display: flex;
  align-items: center;
  gap: 1rem;
}

.auth-bar.auth-ok {
  border-color: rgba(127, 219, 202, 0.3);
}

.auth-icon {
  font-size: 1.25rem;
}

.auth-status {
  font-size: 0.8rem;
  padding: 0.35rem 0.75rem;
  border-radius: 6px;
  white-space: nowrap;
}

.auth-status.authenticated {
  background: rgba(127, 219, 202, 0.15);
  color: #7fdbca;
  cursor: pointer;
  transition: all 0.2s ease;
}

.auth-status.authenticated:hover {
  background: rgba(127, 219, 202, 0.25);
}

/* Auth dropdown */
.auth-dropdown {
  position: relative;
  display: inline-block;
}

.auth-dropdown-menu {
  position: absolute;
  top: calc(100% + 8px);
  right: 0;
  background: rgba(18, 18, 28, 0.98);
  border: 1px solid #2a2a3a;
  border-radius: 8px;
  padding: 0.5rem;
  min-width: 140px;
  opacity: 0;
  visibility: hidden;
  transform: translateY(-8px);
  transition: all 0.2s ease;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
  z-index: 1000;
}

.auth-dropdown:hover .auth-dropdown-menu {
  opacity: 1;
  visibility: visible;
  transform: translateY(0);
}

.auth-dropdown-menu .logout-btn {
  width: 100%;
  display: block;
  text-align: left;
  font-size: 0.75rem;
}

/* Tab Navigation */
.tab-nav {
  display: flex;
  gap: 0;
  margin-bottom: 1.5rem;
  background: rgba(18, 18, 28, 0.7);
  border-radius: 12px;
  padding: 0.35rem;
  border: 1px solid #2a2a3a;
}

.tab-btn {
  flex: 1;
  padding: 0.9rem 1.5rem;
  border: none;
  background: transparent;
  color: #666;
  font-family: inherit;
  font-size: 0.9rem;
  font-weight: 600;
  cursor: pointer;
  border-radius: 8px;
  transition: all 0.25s ease;
  text-transform: uppercase;
  letter-spacing: 0.08em;
  text-decoration: none;
  text-align: center;
}

.tab-btn:hover {
  color: #888;
  background: rgba(255,255,255,0.03);
}

.logout-btn {
  background: rgba(239, 68, 68, 0.1);
  border: 1px solid rgba(239, 68, 68, 0.3);
  color: #f87171;
  padding: 0.4rem 0.75rem;
  border-radius: 6px;
  font-family: inherit;
  font-size: 0.7rem;
  cursor: pointer;
  transition: all 0.2s ease;
}

.logout-btn:hover {
  background: rgba(239, 68, 68, 0.2);
  border-color: rgba(239, 68, 68, 0.5);
}

.footer {
  margin-top: 2rem;
  color: #3a3a4a;
  font-size: 0.7rem;
  text-align: center;
}
</style>
