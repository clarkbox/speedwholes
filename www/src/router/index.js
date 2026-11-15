import { createRouter, createWebHistory } from 'vue-router'
import LoginView from '../views/LoginView.vue'
import SettingsView from '../views/SettingsView.vue'
import SubdomainListView from '../views/SubdomainListView.vue'
import SubdomainEditView from '../views/SubdomainEditView.vue'
import LogsView from '../views/LogsView.vue'

const routes = [
  {
    path: '/',
    redirect: '/login'
  },
  {
    path: '/login',
    name: 'login',
    component: LoginView
  },
  {
    path: '/settings',
    name: 'settings',
    component: SettingsView,
    meta: { requiresAuth: true }
  },
  {
    path: '/subdomains',
    name: 'subdomains',
    component: SubdomainListView,
    meta: { requiresAuth: true }
  },
  {
    path: '/subdomains/:name',
    name: 'subdomain-edit',
    component: SubdomainEditView,
    meta: { requiresAuth: true }
  },
  {
    path: '/logs',
    name: 'logs',
    component: LogsView,
    meta: { requiresAuth: true }
  }
]

const router = createRouter({
  history: createWebHistory(),
  routes
})

// The session token itself lives in localStorage. This flag only remembers
// that login succeeded in this tab; the API still checks the bearer token.
function isAuthenticated() {
  return sessionStorage.getItem('authenticated') === 'true'
}

// Auth guard
router.beforeEach((to, from, next) => {
  if (to.meta.requiresAuth && !isAuthenticated()) {
    // Redirect to login if auth is required but user is not authenticated
    next('/login')
  } else {
    next()
  }
})

export default router
