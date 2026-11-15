<template>
  <AdminLayout>
    <div class="monitor-page">
      <div class="page-header">
        <h1>⚡ Monitor</h1>
        <div class="status-badge" :class="{ connected: sseConnected }">
          <span class="dot"></span>
          {{ sseConnected ? 'Live' : 'Connecting...' }}
        </div>
      </div>

      <table class="stats-table">
        <thead>
          <tr>
            <th>Metric</th>
            <th class="num">Value</th>
            <th class="num">Rate</th>
          </tr>
        </thead>
        <tbody>
          <tr class="section-header">
            <td colspan="3">Requests</td>
          </tr>
          <tr>
            <td>Total Resize Requests</td>
            <td class="num highlight-blue">{{ stats.image_requests.toLocaleString() }}</td>
            <td class="num muted">—</td>
          </tr>

          <tr class="section-header">
            <td colspan="3">Resize Cache</td>
          </tr>
          <tr>
            <td>Cache Hits</td>
            <td class="num highlight-green">{{ stats.cache_hits.toLocaleString() }}</td>
            <td class="num" rowspan="2">
              <span class="rate-badge" :class="rateClass(resizeCacheHitRate)">{{ resizeCacheHitRate }}%</span>
            </td>
          </tr>
          <tr>
            <td>Cache Misses</td>
            <td class="num highlight-amber">{{ stats.cache_misses.toLocaleString() }}</td>
          </tr>

          <tr class="section-header">
            <td colspan="3">Source Cache</td>
          </tr>
          <tr>
            <td>Source Cache Hits</td>
            <td class="num highlight-purple">{{ stats.source_cache_hits.toLocaleString() }}</td>
            <td class="num" rowspan="2">
              <span class="rate-badge" :class="rateClass(sourceCacheHitRate)">{{ sourceCacheHitRate }}%</span>
            </td>
          </tr>
          <tr>
            <td>Network Fetches</td>
            <td class="num highlight-orange">{{ stats.source_cache_misses.toLocaleString() }}</td>
          </tr>

          <template v-if="stats.signature_cache_hits > 0 || stats.signature_cache_misses > 0">
            <tr class="section-header">
              <td colspan="3">Signature Cache</td>
            </tr>
            <tr>
              <td>Signature Cache Hits</td>
              <td class="num highlight-purple">{{ stats.signature_cache_hits.toLocaleString() }}</td>
              <td class="num" rowspan="2">
                <span class="rate-badge" :class="rateClass(signatureCacheHitRate)">{{ signatureCacheHitRate }}%</span>
              </td>
            </tr>
            <tr>
              <td>Signature Verifications</td>
              <td class="num highlight-pink">{{ stats.signature_cache_misses.toLocaleString() }}</td>
            </tr>
          </template>
        </tbody>
      </table>

      <div class="info-row">
        <span class="label">Subdomain</span>
        <code class="value">{{ currentSubdomain || '—' }}</code>
      </div>

      <div class="example-url">
        <span class="label">Example URL</span>
        <a :href="resizeServerUrl + '/resize/w/300/h/200/fit/cover/' + currentOrigin + '/android-chrome-512x512.png'" target="_blank">
          {{ resizeServerUrl }}/resize/w/300/h/200/fit/cover/{{ currentOrigin }}/android-chrome-512x512.png
        </a>
      </div>

      <!-- Disk Usage Section -->
      <div class="disk-usage-section">
        <div class="section-header-row">
          <h2>Disk Usage</h2>
          <button 
            class="check-disk-btn" 
            @click="loadDiskStats" 
            :disabled="diskStatsLoading"
          >
            <span v-if="diskStatsLoading" class="spinner"></span>
            {{ diskStatsLoading ? 'Checking...' : 'Check Disk' }}
          </button>
        </div>

        <div v-if="diskStatsError" class="disk-error">
          {{ diskStatsError }}
        </div>

        <table v-if="diskStats && Object.keys(diskStats.subdomains).length > 0" class="disk-table">
          <thead>
            <tr>
              <th>Subdomain</th>
              <th class="num">Resize Cache</th>
              <th class="num">Source Cache</th>
              <th class="num">Total</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="(stats, name) in diskStats.subdomains" :key="name">
              <td>
                <code class="subdomain-name">{{ name }}</code>
              </td>
              <td class="num">{{ formatKB(stats.resize_kb) }}</td>
              <td class="num">{{ formatKB(stats.source_kb) }}</td>
              <td class="num highlight-blue">{{ formatKB(stats.resize_kb + stats.source_kb) }}</td>
            </tr>
          </tbody>
          <tfoot>
            <tr class="total-row">
              <td><strong>Total</strong></td>
              <td class="num">{{ formatKB(totalResizeKB) }}</td>
              <td class="num">{{ formatKB(totalSourceKB) }}</td>
              <td class="num highlight-green"><strong>{{ formatKB(totalResizeKB + totalSourceKB) }}</strong></td>
            </tr>
          </tfoot>
        </table>

        <div v-else-if="diskStats" class="disk-empty">
          No cache data found. Cache folders will appear after processing some images.
        </div>
      </div>

      <!-- Resize Times Section -->
      <div class="resize-times-section">
        <div class="section-header-row">
          <h2>⏱️ Resize Times</h2>
          <button 
            class="check-times-btn" 
            @click="loadResizeTimes" 
            :disabled="resizeTimesLoading"
          >
            <span v-if="resizeTimesLoading" class="spinner"></span>
            {{ resizeTimesLoading ? 'Loading...' : 'Check Times' }}
          </button>
        </div>

        <div v-if="resizeTimesError" class="times-error">
          {{ resizeTimesError }}
        </div>

        <table v-if="resizeTimes && hasResizeTimeData" class="times-table">
          <thead>
            <tr>
              <th>Subdomain</th>
              <th>Template</th>
              <th class="num">Avg Time</th>
              <th class="num">Count</th>
            </tr>
          </thead>
          <tbody>
            <template v-for="(subData, subName) in resizeTimes.subdomains" :key="subName">
              <tr v-for="(tmplData, tmplName, tmplIdx) in subData.templates" :key="`${subName}-${tmplName}`">
                <td>
                  <code v-if="tmplIdx === 0" class="subdomain-name">{{ subName }}</code>
                </td>
                <td>
                  <code class="template-name">{{ tmplName }}</code>
                </td>
                <td class="num">
                  <span class="time-value" :class="timeClass(tmplData.avg_ms)">{{ formatMs(tmplData.avg_ms) }}</span>
                </td>
                <td class="num muted">{{ tmplData.count.toLocaleString() }}</td>
              </tr>
            </template>
          </tbody>
        </table>

        <div v-else-if="resizeTimes" class="times-empty">
          No resize timing data yet. Stats appear after images are processed.
        </div>
      </div>
    </div>
  </AdminLayout>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue'
import AdminLayout from '../components/AdminLayout.vue'
import { useApi } from '../composables/useApi'
import { useSubdomain } from '../composables/useSubdomain'
import { useConfig } from '../composables/useConfig'

const { authFetch } = useApi()
const { currentSubdomain, currentOrigin } = useSubdomain()
const { config } = useConfig()

// The resize server URL (same as API)
const resizeServerUrl = computed(() => config.value?.apiUrl || '')

// Stats
const stats = ref({
  image_requests: 0,
  cache_hits: 0,
  cache_misses: 0,
  source_cache_hits: 0,
  source_cache_misses: 0,
  signature_cache_hits: 0,
  signature_cache_misses: 0
})

const sourceCacheHitRate = computed(() => {
  const total = stats.value.source_cache_hits + stats.value.source_cache_misses
  if (total === 0) return 0
  return Math.round((stats.value.source_cache_hits / total) * 100)
})

const resizeCacheHitRate = computed(() => {
  const total = stats.value.cache_hits + stats.value.cache_misses
  if (total === 0) return 0
  return Math.round((stats.value.cache_hits / total) * 100)
})

const signatureCacheHitRate = computed(() => {
  const total = stats.value.signature_cache_hits + stats.value.signature_cache_misses
  if (total === 0) return 0
  return Math.round((stats.value.signature_cache_hits / total) * 100)
})

const rateClass = (rate) => {
  if (rate >= 80) return 'good'
  if (rate >= 50) return 'ok'
  return 'low'
}

// Disk Stats
const diskStats = ref(null)
const diskStatsLoading = ref(false)
const diskStatsError = ref(null)

// Resize Times
const resizeTimes = ref(null)
const resizeTimesLoading = ref(false)
const resizeTimesError = ref(null)

const loadDiskStats = async () => {
  diskStatsLoading.value = true
  diskStatsError.value = null
  try {
    const res = await authFetch('/config007/disk-stats')
    if (res.ok) {
      diskStats.value = await res.json()
    } else {
      diskStatsError.value = 'Failed to load disk stats'
    }
  } catch (e) {
    diskStatsError.value = 'Error connecting to server'
  } finally {
    diskStatsLoading.value = false
  }
}

const totalResizeKB = computed(() => {
  if (!diskStats.value) return 0
  return Object.values(diskStats.value.subdomains).reduce((sum, s) => sum + s.resize_kb, 0)
})

const totalSourceKB = computed(() => {
  if (!diskStats.value) return 0
  return Object.values(diskStats.value.subdomains).reduce((sum, s) => sum + s.source_kb, 0)
})

const formatKB = (kb) => {
  if (kb < 1024) return `${kb} KB`
  if (kb < 1024 * 1024) return `${(kb / 1024).toFixed(1)} MB`
  return `${(kb / (1024 * 1024)).toFixed(2)} GB`
}

// Resize Times
const loadResizeTimes = async () => {
  resizeTimesLoading.value = true
  resizeTimesError.value = null
  try {
    const res = await authFetch('/config007/resize-stats')
    if (res.ok) {
      resizeTimes.value = await res.json()
    } else {
      resizeTimesError.value = 'Failed to load resize stats'
    }
  } catch (e) {
    resizeTimesError.value = 'Error connecting to server'
  } finally {
    resizeTimesLoading.value = false
  }
}

const hasResizeTimeData = computed(() => {
  if (!resizeTimes.value?.subdomains) return false
  return Object.keys(resizeTimes.value.subdomains).length > 0
})

const formatMs = (ms) => {
  if (ms < 1) return '<1ms'
  if (ms < 1000) return `${Math.round(ms)}ms`
  return `${(ms / 1000).toFixed(2)}s`
}

const timeClass = (ms) => {
  if (ms < 100) return 'fast'
  if (ms < 500) return 'medium'
  return 'slow'
}

const sseConnected = ref(false)
let pollInterval = null

const connectSSE = () => {
  sseConnected.value = true
  pollInterval = setInterval(async () => {
    try {
      const res = await authFetch('/config007/stats')
      if (res.ok) {
        const data = await res.json()
        stats.value = data
        sseConnected.value = true
      } else {
        sseConnected.value = false
      }
    } catch (e) {
      sseConnected.value = false
    }
  }, 5000)
}

const disconnectSSE = () => {
  if (pollInterval) {
    clearInterval(pollInterval)
    pollInterval = null
  }
  sseConnected.value = false
}

const loadStats = async () => {
  try {
    const res = await authFetch('/config007/stats')
    if (res.ok) {
      const data = await res.json()
      stats.value = data
    }
  } catch (e) {
    // Ignore
  }
}

onMounted(async () => {
  loadStats()
  connectSSE()
})

onUnmounted(() => {
  disconnectSSE()
})
</script>

<style scoped>
.monitor-page {
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

.status-badge {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  font-size: 0.7rem;
  color: #666;
  padding: 0.25rem 0.6rem;
  background: rgba(255, 107, 157, 0.1);
  border-radius: 20px;
}

.status-badge.connected {
  background: rgba(127, 219, 202, 0.1);
  color: #7fdbca;
}

.status-badge .dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: #ff6b9d;
}

.status-badge.connected .dot {
  background: #7fdbca;
  animation: pulse 2s infinite;
}

/* Table */
.stats-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.85rem;
}

.stats-table th {
  text-align: left;
  padding: 0.5rem 0.75rem;
  color: #555;
  font-weight: 500;
  font-size: 0.7rem;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  border-bottom: 1px solid #1f1f2f;
}

.stats-table th.num {
  text-align: right;
}

.stats-table td {
  padding: 0.5rem 0.75rem;
  color: #aaa;
  border-bottom: 1px solid #1a1a26;
}

.stats-table td.num {
  text-align: right;
  font-variant-numeric: tabular-nums;
  font-weight: 500;
}

.stats-table tr:hover td {
  background: rgba(255, 255, 255, 0.02);
}

.stats-table tr.section-header td {
  padding-top: 1rem;
  padding-bottom: 0.35rem;
  color: #666;
  font-size: 0.65rem;
  text-transform: uppercase;
  letter-spacing: 0.1em;
  font-weight: 600;
  border-bottom: none;
  background: none;
}

.stats-table tr.section-header:hover td {
  background: none;
}

/* Value colors */
.highlight-blue { color: #82aaff; }
.highlight-green { color: #7fdbca; }
.highlight-amber { color: #ffcb6b; }
.highlight-purple { color: #c792ea; }
.highlight-orange { color: #ff9e64; }
.highlight-pink { color: #ff6b9d; }
.muted { color: #444; }

/* Rate badge */
.rate-badge {
  display: inline-block;
  padding: 0.15rem 0.5rem;
  border-radius: 4px;
  font-size: 0.75rem;
  font-weight: 600;
}

.rate-badge.good {
  background: rgba(127, 219, 202, 0.15);
  color: #7fdbca;
}

.rate-badge.ok {
  background: rgba(255, 203, 107, 0.15);
  color: #ffcb6b;
}

.rate-badge.low {
  background: rgba(255, 107, 157, 0.15);
  color: #ff6b9d;
}

/* Info rows */
.info-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.75rem;
  margin-top: 1rem;
  background: #0f0f18;
  border-radius: 6px;
  border: 1px solid #1f1f2f;
}

.info-row .label {
  color: #666;
  font-size: 0.75rem;
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.info-row .value {
  color: #c792ea;
  font-size: 0.85rem;
  background: rgba(199, 146, 234, 0.1);
  padding: 0.2rem 0.5rem;
  border-radius: 4px;
}

.example-url {
  margin-top: 0.75rem;
  padding: 0.75rem;
  background: #0a0a12;
  border-radius: 6px;
  border: 1px solid #1a1a26;
}

.example-url .label {
  display: block;
  color: #555;
  font-size: 0.65rem;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  margin-bottom: 0.4rem;
}

.example-url a {
  color: #82aaff;
  font-size: 0.7rem;
  word-break: break-all;
  text-decoration: none;
  opacity: 0.8;
}

.example-url a:hover {
  opacity: 1;
  text-decoration: underline;
}

@keyframes pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.5; }
}

/* Disk Usage Section */
.disk-usage-section {
  margin-top: 1.5rem;
  padding-top: 1.5rem;
  border-top: 1px solid #1f1f2f;
}

.section-header-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 1rem;
}

.section-header-row h2 {
  font-size: 0.9rem;
  font-weight: 600;
  color: #e0e0e0;
  margin: 0;
}

.check-disk-btn {
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

.check-disk-btn:hover:not(:disabled) {
  background: rgba(130, 170, 255, 0.2);
  border-color: rgba(130, 170, 255, 0.5);
}

.check-disk-btn:disabled {
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

.disk-error {
  padding: 0.75rem;
  background: rgba(255, 107, 157, 0.1);
  border: 1px solid rgba(255, 107, 157, 0.3);
  border-radius: 6px;
  color: #ff6b9d;
  font-size: 0.8rem;
}

.disk-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.8rem;
  margin-top: 0.5rem;
}

.disk-table th {
  text-align: left;
  padding: 0.5rem 0.75rem;
  color: #555;
  font-weight: 500;
  font-size: 0.65rem;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  border-bottom: 1px solid #1f1f2f;
}

.disk-table th.num {
  text-align: right;
}

.disk-table td {
  padding: 0.5rem 0.75rem;
  color: #aaa;
  border-bottom: 1px solid #1a1a26;
}

.disk-table td.num {
  text-align: right;
  font-variant-numeric: tabular-nums;
  font-weight: 500;
}

.disk-table tr:hover td {
  background: rgba(255, 255, 255, 0.02);
}

.disk-table .total-row td {
  border-top: 1px solid #2a2a3a;
  background: rgba(127, 219, 202, 0.05);
}

.disk-table .total-row:hover td {
  background: rgba(127, 219, 202, 0.08);
}

.subdomain-name {
  color: #c792ea;
  background: rgba(199, 146, 234, 0.1);
  padding: 0.15rem 0.4rem;
  border-radius: 4px;
  font-size: 0.75rem;
}

.disk-empty {
  padding: 1rem;
  background: #0f0f18;
  border-radius: 6px;
  border: 1px solid #1f1f2f;
  color: #666;
  font-size: 0.8rem;
  text-align: center;
}

/* Resize Times Section */
.resize-times-section {
  margin-top: 1.5rem;
  padding-top: 1.5rem;
  border-top: 1px solid #1f1f2f;
}

.check-times-btn {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  padding: 0.4rem 0.8rem;
  font-size: 0.75rem;
  font-weight: 500;
  color: #89ddff;
  background: rgba(137, 221, 255, 0.1);
  border: 1px solid rgba(137, 221, 255, 0.3);
  border-radius: 6px;
  cursor: pointer;
  transition: all 0.15s ease;
}

.check-times-btn:hover:not(:disabled) {
  background: rgba(137, 221, 255, 0.2);
  border-color: rgba(137, 221, 255, 0.5);
}

.check-times-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.times-error {
  padding: 0.75rem;
  background: rgba(255, 107, 157, 0.1);
  border: 1px solid rgba(255, 107, 157, 0.3);
  border-radius: 6px;
  color: #ff6b9d;
  font-size: 0.8rem;
}

.times-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.8rem;
  margin-top: 0.5rem;
}

.times-table th {
  text-align: left;
  padding: 0.5rem 0.75rem;
  color: #555;
  font-weight: 500;
  font-size: 0.65rem;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  border-bottom: 1px solid #1f1f2f;
}

.times-table th.num {
  text-align: right;
}

.times-table td {
  padding: 0.4rem 0.75rem;
  color: #aaa;
  border-bottom: 1px solid #1a1a26;
}

.times-table td.num {
  text-align: right;
  font-variant-numeric: tabular-nums;
  font-weight: 500;
}

.times-table tr:hover td {
  background: rgba(255, 255, 255, 0.02);
}

.template-name {
  color: #ffcb6b;
  background: rgba(255, 203, 107, 0.1);
  padding: 0.15rem 0.4rem;
  border-radius: 4px;
  font-size: 0.75rem;
}

.time-value {
  display: inline-block;
  padding: 0.15rem 0.5rem;
  border-radius: 4px;
  font-size: 0.75rem;
  font-weight: 600;
}

.time-value.fast {
  background: rgba(127, 219, 202, 0.15);
  color: #7fdbca;
}

.time-value.medium {
  background: rgba(255, 203, 107, 0.15);
  color: #ffcb6b;
}

.time-value.slow {
  background: rgba(255, 107, 157, 0.15);
  color: #ff6b9d;
}

.times-empty {
  padding: 1rem;
  background: #0f0f18;
  border-radius: 6px;
  border: 1px solid #1f1f2f;
  color: #666;
  font-size: 0.8rem;
  text-align: center;
}
</style>
