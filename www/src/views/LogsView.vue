<template>
  <AdminLayout>
    <div class="logs-page">
      <div class="page-header">
        <h1>📋 Logs</h1>
        <div class="header-actions">
          <button 
            class="refresh-btn" 
            @click="loadLogs" 
            :disabled="loading"
          >
            <span v-if="loading" class="spinner"></span>
            {{ loading ? 'Loading...' : 'Refresh' }}
          </button>
        </div>
      </div>

      <!-- Search/Filter -->
      <div class="search-bar">
        <input 
          type="text" 
          v-model="searchQuery" 
          placeholder="Filter logs... (searches message, target, level)"
          class="search-input"
        />
        <div class="log-stats">
          <span class="stat">{{ filteredLogs.length }} / {{ logs.length }} entries</span>
          <span v-if="hasMore" class="stat has-more">More logs available</span>
        </div>
      </div>

      <!-- Level Filter -->
      <div class="level-filters">
        <button 
          v-for="level in levels" 
          :key="level.name"
          class="level-btn"
          :class="{ active: activeLevels.has(level.name), [level.class]: true }"
          @click="toggleLevel(level.name)"
        >
          {{ level.name }}
          <span class="count">{{ levelCounts[level.name] || 0 }}</span>
        </button>
      </div>

      <!-- Error message -->
      <div v-if="error" class="error-banner">
        {{ error }}
      </div>

      <!-- Empty state -->
      <div v-else-if="!loading && logs.length === 0" class="empty-state">
        <div class="empty-icon">📭</div>
        <div class="empty-text">No logs found</div>
        <div class="empty-hint">Logs will appear here once the server starts processing requests.</div>
      </div>

      <!-- Log entries -->
      <div v-else class="log-container" ref="logContainer">
        <div 
          v-for="(log, index) in filteredLogs" 
          :key="index" 
          class="log-entry"
          :class="getLevelClass(log)"
        >
          <div class="log-header">
            <span class="log-timestamp">{{ formatTimestamp(log.timestamp) }}</span>
            <span class="log-level" :class="getLevelClass(log)">{{ getLevel(log) }}</span>
            <span class="log-target">{{ log.target || 'unknown' }}</span>
          </div>
          <div class="log-message">{{ getMessage(log) }}</div>
          <div v-if="hasExtraFields(log)" class="log-fields">
            <span v-for="(value, key) in getExtraFields(log)" :key="key" class="field">
              <span class="field-key">{{ key }}:</span>
              <span class="field-value">{{ formatValue(value) }}</span>
            </span>
          </div>
        </div>
      </div>

      <!-- Load more -->
      <div v-if="hasMore && !loading" class="load-more">
        <button class="load-more-btn" @click="loadMore">
          Load older logs
        </button>
      </div>
    </div>
  </AdminLayout>
</template>

<script setup>
import { ref, computed, onMounted } from 'vue'
import AdminLayout from '../components/AdminLayout.vue'
import { useApi } from '../composables/useApi'

const { authFetch } = useApi()

// State
const logs = ref([])
const loading = ref(false)
const error = ref(null)
const hasMore = ref(false)
const offset = ref(0)
const searchQuery = ref('')
const logContainer = ref(null)

// Level filters
const levels = [
  { name: 'ERROR', class: 'level-error' },
  { name: 'WARN', class: 'level-warn' },
  { name: 'INFO', class: 'level-info' },
  { name: 'DEBUG', class: 'level-debug' },
  { name: 'TRACE', class: 'level-trace' },
]
const activeLevels = ref(new Set(['ERROR', 'WARN', 'INFO', 'DEBUG', 'TRACE']))

const toggleLevel = (level) => {
  if (activeLevels.value.has(level)) {
    activeLevels.value.delete(level)
  } else {
    activeLevels.value.add(level)
  }
  // Trigger reactivity
  activeLevels.value = new Set(activeLevels.value)
}

// Computed
const levelCounts = computed(() => {
  const counts = {}
  for (const log of logs.value) {
    const level = getLevel(log)
    counts[level] = (counts[level] || 0) + 1
  }
  return counts
})

const filteredLogs = computed(() => {
  let result = logs.value

  // Filter by level
  result = result.filter(log => activeLevels.value.has(getLevel(log)))

  // Filter by search query
  if (searchQuery.value.trim()) {
    const query = searchQuery.value.toLowerCase()
    result = result.filter(log => {
      const message = getMessage(log).toLowerCase()
      const target = (log.target || '').toLowerCase()
      const level = getLevel(log).toLowerCase()
      const fieldsStr = JSON.stringify(log.fields || {}).toLowerCase()
      return message.includes(query) || target.includes(query) || level.includes(query) || fieldsStr.includes(query)
    })
  }

  return result
})

// Helpers
const getLevel = (log) => {
  return log.level || 'INFO'
}

const getMessage = (log) => {
  if (log.fields?.message) return log.fields.message
  if (log.message) return log.message
  if (log.raw) return log.raw
  return JSON.stringify(log)
}

const getLevelClass = (log) => {
  const level = getLevel(log).toUpperCase()
  switch (level) {
    case 'ERROR': return 'level-error'
    case 'WARN': return 'level-warn'
    case 'INFO': return 'level-info'
    case 'DEBUG': return 'level-debug'
    case 'TRACE': return 'level-trace'
    default: return 'level-info'
  }
}

const formatTimestamp = (ts) => {
  if (!ts) return '—'
  try {
    const date = new Date(ts)
    return date.toLocaleTimeString('en-US', { 
      hour12: false, 
      hour: '2-digit', 
      minute: '2-digit', 
      second: '2-digit',
      fractionalSecondDigits: 3 
    })
  } catch {
    return ts
  }
}

const hasExtraFields = (log) => {
  const fields = log.fields || {}
  const extraKeys = Object.keys(fields).filter(k => k !== 'message')
  return extraKeys.length > 0
}

const getExtraFields = (log) => {
  const fields = { ...log.fields }
  delete fields.message
  return fields
}

const formatValue = (value) => {
  if (typeof value === 'object') return JSON.stringify(value)
  return String(value)
}

// API calls
const loadLogs = async () => {
  loading.value = true
  error.value = null
  offset.value = 0
  
  try {
    const res = await authFetch('/config007/logs?lines=1000&offset=0')
    if (res.ok) {
      const data = await res.json()
      // Reverse to show newest first
      logs.value = data.entries.reverse()
      hasMore.value = data.has_more
    } else {
      error.value = 'Failed to load logs'
    }
  } catch (e) {
    error.value = 'Error connecting to server'
  } finally {
    loading.value = false
  }
}

const loadMore = async () => {
  loading.value = true
  offset.value += 1000
  
  try {
    const res = await authFetch(`/config007/logs?lines=1000&offset=${offset.value}`)
    if (res.ok) {
      const data = await res.json()
      // Append older logs (reversed) to the end
      logs.value = [...logs.value, ...data.entries.reverse()]
      hasMore.value = data.has_more
    }
  } catch (e) {
    // Ignore
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  loadLogs()
})
</script>

<style scoped>
.logs-page {
  background: rgba(18, 18, 28, 0.95);
  border: 1px solid #2a2a3a;
  border-radius: 12px;
  padding: 1.5rem;
}

.page-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 1.25rem;
  padding-bottom: 1rem;
  border-bottom: 1px solid #1f1f2f;
}

.page-header h1 {
  font-size: 1.1rem;
  font-weight: 600;
  color: #e0e0e0;
  margin: 0;
}

.header-actions {
  display: flex;
  gap: 0.5rem;
}

.refresh-btn {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  padding: 0.4rem 0.8rem;
  font-size: 0.75rem;
  font-weight: 500;
  color: #82aaff;
  background: rgba(130, 170, 255, 0.1);
  border: 1px solid rgba(130, 170, 255, 0.3);
  border-radius: 6px;
  cursor: pointer;
  transition: all 0.15s ease;
}

.refresh-btn:hover:not(:disabled) {
  background: rgba(130, 170, 255, 0.2);
  border-color: rgba(130, 170, 255, 0.5);
}

.refresh-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.spinner {
  width: 12px;
  height: 12px;
  border: 2px solid transparent;
  border-top-color: currentColor;
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

/* Search bar */
.search-bar {
  display: flex;
  align-items: center;
  gap: 1rem;
  margin-bottom: 1rem;
}

.search-input {
  flex: 1;
  padding: 0.6rem 1rem;
  font-size: 0.85rem;
  color: #e0e0e0;
  background: #0a0a12;
  border: 1px solid #2a2a3a;
  border-radius: 8px;
  outline: none;
  transition: border-color 0.15s ease;
}

.search-input:focus {
  border-color: #82aaff;
}

.search-input::placeholder {
  color: #555;
}

.log-stats {
  display: flex;
  gap: 0.75rem;
  font-size: 0.7rem;
  color: #666;
}

.log-stats .has-more {
  color: #ffcb6b;
}

/* Level filters */
.level-filters {
  display: flex;
  gap: 0.5rem;
  margin-bottom: 1rem;
  flex-wrap: wrap;
}

.level-btn {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  padding: 0.3rem 0.6rem;
  font-size: 0.7rem;
  font-weight: 500;
  background: rgba(255, 255, 255, 0.03);
  border: 1px solid #2a2a3a;
  border-radius: 4px;
  cursor: pointer;
  opacity: 0.4;
  transition: all 0.15s ease;
}

.level-btn.active {
  opacity: 1;
}

.level-btn .count {
  padding: 0.1rem 0.35rem;
  font-size: 0.6rem;
  background: rgba(255, 255, 255, 0.1);
  border-radius: 3px;
}

.level-btn.level-error { color: #ff6b6b; border-color: rgba(255, 107, 107, 0.3); }
.level-btn.level-error.active { background: rgba(255, 107, 107, 0.1); }
.level-btn.level-warn { color: #ffcb6b; border-color: rgba(255, 203, 107, 0.3); }
.level-btn.level-warn.active { background: rgba(255, 203, 107, 0.1); }
.level-btn.level-info { color: #7fdbca; border-color: rgba(127, 219, 202, 0.3); }
.level-btn.level-info.active { background: rgba(127, 219, 202, 0.1); }
.level-btn.level-debug { color: #82aaff; border-color: rgba(130, 170, 255, 0.3); }
.level-btn.level-debug.active { background: rgba(130, 170, 255, 0.1); }
.level-btn.level-trace { color: #888; border-color: rgba(136, 136, 136, 0.3); }
.level-btn.level-trace.active { background: rgba(136, 136, 136, 0.1); }

/* Error banner */
.error-banner {
  padding: 0.75rem;
  background: rgba(255, 107, 107, 0.1);
  border: 1px solid rgba(255, 107, 107, 0.3);
  border-radius: 6px;
  color: #ff6b6b;
  font-size: 0.8rem;
  margin-bottom: 1rem;
}

/* Empty state */
.empty-state {
  text-align: center;
  padding: 3rem 1rem;
}

.empty-icon {
  font-size: 2.5rem;
  margin-bottom: 1rem;
}

.empty-text {
  color: #888;
  font-size: 1rem;
  margin-bottom: 0.5rem;
}

.empty-hint {
  color: #555;
  font-size: 0.8rem;
}

/* Log container */
.log-container {
  max-height: 600px;
  overflow-y: auto;
  background: #0a0a12;
  border: 1px solid #1a1a26;
  border-radius: 8px;
}

.log-container::-webkit-scrollbar {
  width: 8px;
}

.log-container::-webkit-scrollbar-track {
  background: #0a0a12;
}

.log-container::-webkit-scrollbar-thumb {
  background: #2a2a3a;
  border-radius: 4px;
}

.log-container::-webkit-scrollbar-thumb:hover {
  background: #3a3a4a;
}

/* Log entry */
.log-entry {
  padding: 0.6rem 0.8rem;
  border-bottom: 1px solid #151520;
  font-family: 'JetBrains Mono', 'Fira Code', monospace;
  font-size: 0.75rem;
}

.log-entry:last-child {
  border-bottom: none;
}

.log-entry:hover {
  background: rgba(255, 255, 255, 0.02);
}

.log-entry.level-error {
  background: rgba(255, 107, 107, 0.03);
  border-left: 3px solid #ff6b6b;
}

.log-entry.level-warn {
  border-left: 3px solid #ffcb6b;
}

.log-header {
  display: flex;
  align-items: center;
  gap: 0.6rem;
  margin-bottom: 0.3rem;
}

.log-timestamp {
  color: #555;
  font-size: 0.7rem;
}

.log-level {
  padding: 0.1rem 0.4rem;
  font-size: 0.6rem;
  font-weight: 600;
  border-radius: 3px;
  text-transform: uppercase;
}

.log-level.level-error { background: rgba(255, 107, 107, 0.2); color: #ff6b6b; }
.log-level.level-warn { background: rgba(255, 203, 107, 0.2); color: #ffcb6b; }
.log-level.level-info { background: rgba(127, 219, 202, 0.2); color: #7fdbca; }
.log-level.level-debug { background: rgba(130, 170, 255, 0.2); color: #82aaff; }
.log-level.level-trace { background: rgba(136, 136, 136, 0.2); color: #888; }

.log-target {
  color: #c792ea;
  font-size: 0.65rem;
}

.log-message {
  color: #d0d0d0;
  word-break: break-word;
  line-height: 1.4;
}

.log-fields {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
  margin-top: 0.4rem;
}

.field {
  display: inline-flex;
  gap: 0.25rem;
  padding: 0.15rem 0.4rem;
  background: rgba(130, 170, 255, 0.08);
  border-radius: 3px;
  font-size: 0.65rem;
}

.field-key {
  color: #82aaff;
}

.field-value {
  color: #aaa;
}

/* Load more */
.load-more {
  text-align: center;
  padding: 1rem;
}

.load-more-btn {
  padding: 0.5rem 1.5rem;
  font-size: 0.8rem;
  color: #82aaff;
  background: rgba(130, 170, 255, 0.1);
  border: 1px solid rgba(130, 170, 255, 0.3);
  border-radius: 6px;
  cursor: pointer;
  transition: all 0.15s ease;
}

.load-more-btn:hover {
  background: rgba(130, 170, 255, 0.2);
}
</style>
