<template>
  <AdminLayout>
    <div class="container">
      <div class="header">
        <div style="display: flex; align-items: center; gap: 0.75rem;">
          <span class="emoji-logo">🌐</span>
          <h1>Subdomain Configuration</h1>
        </div>
      </div>
      <div class="subtitle">Configure per-subdomain settings (max dimensions, source URL)</div>

      <!-- Resize Stats Summary -->
      <div class="stats-summary" v-if="hasAnyStats">
        <div class="stats-header">
          <span class="stats-title">⏱️ Resize Performance</span>
          <button class="btn-reset-stats" @click="resetAllStats" :disabled="resettingStats">
            {{ resettingStats ? 'Resetting...' : 'Reset All Stats' }}
          </button>
        </div>
        <div class="stats-overall">
          <span class="stats-label">Overall avg:</span>
          <code class="stats-value">{{ formatMs(overallAvgMs) }}</code>
          <span class="stats-count">({{ totalResizeCount }} resizes)</span>
        </div>
      </div>

      <div class="subdomain-list">
        <div v-if="loading" class="empty-state">
          Loading subdomains...
        </div>
        
        <div v-else-if="subdomains.length === 0" class="empty-state">
          No subdomain configurations yet
        </div>

        <router-link 
          v-for="subdomain in subdomains" 
          :key="subdomain.name" 
          :to="`/subdomains/${encodeURIComponent(subdomain.name)}`"
          class="subdomain-card"
        >
          <div class="subdomain-info">
            <div class="subdomain-name">{{ subdomain.name || '(unnamed)' }}</div>
            <div class="subdomain-url" v-if="subdomain.source_template">{{ subdomain.source_template }}</div>
            <div class="subdomain-summary">
              <span class="subdomain-summary-item" v-if="subdomain.max_width || subdomain.max_height">
                📐 <code>{{ subdomain.max_width || '∞' }}×{{ subdomain.max_height || '∞' }}</code>
              </span>
              <span class="subdomain-summary-item" v-if="subdomain.signing_required">
                🔐 signed
              </span>
              <span class="subdomain-summary-item" v-if="subdomain.allowed_sources && subdomain.allowed_sources.length">
                🛡️ {{ subdomain.allowed_sources.length }} patterns
              </span>
              <span class="subdomain-summary-item template-badge" v-if="subdomain.templates && subdomain.templates.length">
                📋 {{ subdomain.templates.length }} template{{ subdomain.templates.length !== 1 ? 's' : '' }}
              </span>
              <span class="subdomain-summary-item timing-badge" v-if="getSubdomainStats(subdomain.name)">
                ⏱️ <code>{{ formatMs(getSubdomainAvgMs(subdomain.name)) }}</code>
                <span class="timing-count">({{ getSubdomainCount(subdomain.name) }})</span>
              </span>
            </div>
          </div>
          <div class="subdomain-actions" @click.prevent>
            <button class="btn-cache" @click.stop="clearCache(subdomain.name, 'resize')" :disabled="clearingCache[subdomain.name]" title="Clear resized images cache">
              🗑️ Resize
            </button>
            <button class="btn-cache" @click.stop="clearCache(subdomain.name, 'source')" :disabled="clearingCache[subdomain.name]" title="Clear source images cache">
              🗑️ Source
            </button>
            <button class="btn-edit" @click.stop="editSubdomain(subdomain.name)">Edit</button>
            <button class="btn-delete" @click.stop="deleteSubdomain(subdomain)">Delete</button>
          </div>
        </router-link>
      </div>

      <div class="actions">
        <router-link to="/subdomains/new" class="btn-add">+ Add Subdomain</router-link>
      </div>
    </div>
  </AdminLayout>
</template>

<script setup>
import { ref, computed, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import AdminLayout from '../components/AdminLayout.vue'
import { useApi } from '../composables/useApi'

const router = useRouter()
const { authFetch } = useApi()

const subdomains = ref([])
const resizeStats = ref(null)
const loading = ref(true)
const resettingStats = ref(false)
const clearingCache = ref({})

// Load both subdomains and resize stats
const loadData = async () => {
  loading.value = true
  try {
    const [subdomainsRes, statsRes] = await Promise.all([
      authFetch('/config007/subdomains'),
      authFetch('/config007/resize-stats')
    ])
    
    if (subdomainsRes.ok) {
      subdomains.value = await subdomainsRes.json()
    }
    if (statsRes.ok) {
      resizeStats.value = await statsRes.json()
    }
  } catch (e) {
    // Ignore
  }
  loading.value = false
}

// Get stats for a specific subdomain
const getSubdomainStats = (name) => {
  return resizeStats.value?.subdomains?.[name]
}

// Compute average ms for a subdomain (across all its templates)
const getSubdomainAvgMs = (name) => {
  const stats = getSubdomainStats(name)
  if (!stats?.templates) return 0
  
  let totalMs = 0
  let totalCount = 0
  for (const tmpl of Object.values(stats.templates)) {
    totalMs += tmpl.avg_ms * tmpl.count
    totalCount += tmpl.count
  }
  return totalCount > 0 ? totalMs / totalCount : 0
}

// Get total resize count for a subdomain
const getSubdomainCount = (name) => {
  const stats = getSubdomainStats(name)
  if (!stats?.templates) return 0
  
  return Object.values(stats.templates).reduce((sum, t) => sum + t.count, 0)
}

// Check if we have any stats at all
const hasAnyStats = computed(() => {
  const subs = resizeStats.value?.subdomains
  return subs && Object.keys(subs).length > 0
})

// Compute overall average across all subdomains
const overallAvgMs = computed(() => {
  const subs = resizeStats.value?.subdomains
  if (!subs) return 0
  
  let totalMs = 0
  let totalCount = 0
  for (const sub of Object.values(subs)) {
    for (const tmpl of Object.values(sub.templates || {})) {
      totalMs += tmpl.avg_ms * tmpl.count
      totalCount += tmpl.count
    }
  }
  return totalCount > 0 ? totalMs / totalCount : 0
})

// Total resize count across all subdomains
const totalResizeCount = computed(() => {
  const subs = resizeStats.value?.subdomains
  if (!subs) return 0
  
  let total = 0
  for (const sub of Object.values(subs)) {
    for (const tmpl of Object.values(sub.templates || {})) {
      total += tmpl.count
    }
  }
  return total
})

// Format milliseconds for display
const formatMs = (ms) => {
  if (ms < 1) return '<1ms'
  if (ms < 1000) return `${Math.round(ms)}ms`
  return `${(ms / 1000).toFixed(2)}s`
}

// Reset all stats
const resetAllStats = async () => {
  if (!confirm('Reset all resize timing stats?')) return
  
  resettingStats.value = true
  try {
    const res = await authFetch('/config007/resize-stats', { method: 'DELETE' })
    if (res.ok) {
      resizeStats.value = { subdomains: {} }
    } else {
      const error = await res.json()
      alert(`Failed to reset: ${error.error || 'Unknown error'}`)
    }
  } catch (e) {
    alert(`Network error: ${e.message}`)
  }
  resettingStats.value = false
}

// Clear cache for a subdomain
const clearCache = async (subdomainName, cacheType) => {
  const typeLabel = cacheType === 'resize' ? 'resized images' : 'source images'
  if (!confirm(`Clear ${typeLabel} cache for "${subdomainName}"?\n\nThis will delete all cached ${typeLabel} for this subdomain.`)) {
    return
  }
  
  clearingCache.value[subdomainName] = true
  try {
    const res = await authFetch(`/config007/subdomain/${encodeURIComponent(subdomainName)}/cache/${cacheType}`, {
      method: 'DELETE'
    })
    const data = await res.json()
    if (res.ok) {
      const kb = Math.round(data.bytes_freed / 1024)
      alert(`✓ ${cacheType.charAt(0).toUpperCase() + cacheType.slice(1)} cache cleared!\n\nFiles deleted: ${data.files_deleted}\nSpace freed: ${kb} KB`)
    } else {
      alert(`Failed to clear cache: ${data.error || 'Unknown error'}`)
    }
  } catch (e) {
    alert(`Network error: ${e.message}`)
  }
  clearingCache.value[subdomainName] = false
}

const editSubdomain = (name) => {
  router.push(`/subdomains/${encodeURIComponent(name)}`)
}

const deleteSubdomain = async (subdomain) => {
  if (!confirm(`Delete subdomain "${subdomain.name}"?`)) {
    return
  }
  
  try {
    const res = await authFetch(`/config007/subdomain/${encodeURIComponent(subdomain.name)}`, {
      method: 'DELETE'
    })
    if (!res.ok) {
      const error = await res.json()
      alert(`Failed to delete: ${error.error || 'Unknown error'}`)
      return
    }
    // Reload list after deletion
    await loadData()
  } catch (e) {
    alert(`Network error: ${e.message}`)
  }
}

onMounted(() => {
  loadData()
})
</script>

<style scoped>
.container {
  background: rgba(18, 18, 28, 0.95);
  border: 1px solid #2a2a3a;
  border-radius: 16px;
  padding: 2.5rem;
  margin-bottom: 1.5rem;
  box-shadow: 
    0 0 0 1px rgba(255,255,255,0.05),
    0 30px 60px -20px rgba(0, 0, 0, 0.7);
}

.header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.75rem;
  margin-bottom: 0.5rem;
}

.emoji-logo {
  font-size: 1.75rem;
}

.subtitle {
  color: #666;
  font-size: 0.8rem;
  margin-bottom: 1.5rem;
}

.stats-summary {
  background: rgba(137, 221, 255, 0.05);
  border: 1px solid rgba(137, 221, 255, 0.2);
  border-radius: 10px;
  padding: 1rem 1.25rem;
  margin-bottom: 1.5rem;
}

.stats-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 0.5rem;
}

.stats-title {
  color: #89ddff;
  font-weight: 600;
  font-size: 0.85rem;
}

.stats-overall {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: 0.8rem;
}

.stats-label {
  color: #666;
}

.stats-value {
  color: #89ddff;
  background: rgba(137, 221, 255, 0.15);
  padding: 0.15rem 0.4rem;
  border-radius: 4px;
  font-size: 0.85rem;
}

.stats-count {
  color: #546e7a;
  font-size: 0.75rem;
}

.btn-reset-stats {
  background: transparent;
  border: 1px solid rgba(255, 107, 157, 0.3);
  color: #ff6b9d;
  padding: 0.35rem 0.65rem;
  border-radius: 6px;
  font-family: inherit;
  font-size: 0.7rem;
  cursor: pointer;
  transition: all 0.2s ease;
}

.btn-reset-stats:hover:not(:disabled) {
  background: rgba(255, 107, 157, 0.1);
  border-color: #ff6b9d;
}

.btn-reset-stats:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.subdomain-list {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.subdomain-card {
  display: flex;
  justify-content: space-between;
  align-items: center;
  background: #0f0f18;
  border: 1px solid rgba(199, 146, 234, 0.3);
  border-radius: 12px;
  padding: 1rem 1.25rem;
  text-decoration: none;
  transition: all 0.2s ease;
}

.subdomain-card:hover {
  border-color: rgba(199, 146, 234, 0.5);
  background: rgba(199, 146, 234, 0.05);
  transform: translateY(-1px);
}

.subdomain-info {
  flex: 1;
  min-width: 0;
}

.subdomain-name {
  color: #c792ea;
  font-weight: 600;
  font-size: 1rem;
}

.subdomain-url {
  color: #546e7a;
  font-size: 0.75rem;
  margin-top: 0.25rem;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.subdomain-summary {
  display: flex;
  flex-wrap: wrap;
  gap: 0.75rem;
  font-size: 0.75rem;
  color: #666;
  margin-top: 0.5rem;
}

.subdomain-summary-item {
  display: flex;
  align-items: center;
  gap: 0.25rem;
}

.subdomain-summary-item code {
  color: #82aaff;
  background: rgba(130, 170, 255, 0.1);
  padding: 0.1rem 0.3rem;
  border-radius: 3px;
}

.subdomain-summary-item.template-badge {
  color: #ffcb6b;
}

.subdomain-summary-item.timing-badge {
  color: #89ddff;
}

.subdomain-summary-item.timing-badge code {
  color: #89ddff;
  background: rgba(137, 221, 255, 0.1);
}

.subdomain-summary-item .timing-count {
  color: #546e7a;
  font-size: 0.7rem;
}

.subdomain-actions {
  display: flex;
  gap: 0.5rem;
  flex-shrink: 0;
  margin-left: 1rem;
}

.empty-state {
  text-align: center;
  padding: 2rem;
  color: #555;
}

.actions {
  margin-top: 2rem;
  padding-top: 1.5rem;
  border-top: 1px solid #1f1f2f;
  display: flex;
  gap: 0.75rem;
}

.btn-add {
  background: linear-gradient(135deg, #c792ea 0%, #a06cd5 100%);
  border: none;
  color: #0a0a0f;
  padding: 0.75rem 1.25rem;
  border-radius: 8px;
  font-family: inherit;
  font-size: 0.85rem;
  font-weight: 600;
  cursor: pointer;
  text-decoration: none;
  transition: all 0.2s ease;
}

.btn-add:hover {
  transform: translateY(-1px);
  box-shadow: 0 4px 12px rgba(199, 146, 234, 0.3);
}

.btn-edit {
  background: transparent;
  border: 1px solid rgba(130, 170, 255, 0.3);
  color: #82aaff;
  padding: 0.5rem 0.75rem;
  border-radius: 6px;
  font-family: inherit;
  font-size: 0.75rem;
  cursor: pointer;
  transition: all 0.2s ease;
}

.btn-edit:hover {
  background: rgba(130, 170, 255, 0.1);
  border-color: #82aaff;
}

.btn-delete {
  background: transparent;
  border: 1px solid #ff6b9d33;
  color: #ff6b9d;
  padding: 0.5rem 0.75rem;
  border-radius: 6px;
  font-family: inherit;
  font-size: 0.75rem;
  cursor: pointer;
  transition: all 0.2s ease;
}

.btn-delete:hover {
  background: rgba(255, 107, 157, 0.1);
  border-color: #ff6b9d;
}

.btn-cache {
  background: transparent;
  border: 1px solid rgba(255, 203, 107, 0.3);
  color: #ffcb6b;
  padding: 0.5rem 0.6rem;
  border-radius: 6px;
  font-family: inherit;
  font-size: 0.7rem;
  cursor: pointer;
  transition: all 0.2s ease;
}

.btn-cache:hover:not(:disabled) {
  background: rgba(255, 203, 107, 0.1);
  border-color: #ffcb6b;
}

.btn-cache:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>
