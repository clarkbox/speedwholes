<template>
  <AdminLayout>
    <div class="container">
      <div class="header">
        <div style="display: flex; align-items: center; gap: 0.75rem;">
          <router-link to="/subdomains" class="back-btn">← Back</router-link>
          <span class="emoji-logo">🌐</span>
          <h1>{{ isNew ? 'New Subdomain' : 'Edit Subdomain' }}</h1>
        </div>
      </div>
      <div class="subtitle">
        {{ isNew ? 'Create a new subdomain configuration' : `Editing configuration for ${originalName}` }}
      </div>

      <div v-if="loading" class="loading-state">
        Loading subdomain configuration...
      </div>

      <div v-else-if="notFound" class="error-state">
        <div class="error-icon">❌</div>
        <div class="error-message">Subdomain "{{ $route.params.name }}" not found</div>
        <router-link to="/subdomains" class="btn-back">← Back to list</router-link>
      </div>

      <form v-else @submit.prevent="saveSubdomain" class="subdomain-form">
        <div class="form-section">
          <div class="form-fields">
            <div class="form-field full-width">
              <label><span class="field-name">name</span> <span class="field-hint">(subdomain identifier)</span></label>
              <input 
                type="text" 
                v-model="subdomain.name" 
                placeholder="DEVLOCAL, cdn, images..."
                :disabled="!isNew"
                :class="{ readonly: !isNew }"
              >
              <div class="field-note" v-if="!isNew">Name cannot be changed after creation</div>
            </div>
            <div class="form-field">
              <label><span class="field-name">max_width</span> <span class="field-hint">(optional)</span></label>
              <input type="number" v-model.number="subdomain.max_width" min="1" placeholder="use global">
            </div>
            <div class="form-field">
              <label><span class="field-name">max_height</span> <span class="field-hint">(optional)</span></label>
              <input type="number" v-model.number="subdomain.max_height" min="1" placeholder="use global">
            </div>
            <div class="form-field full-width">
              <label><span class="field-name">source_template</span> <span class="field-hint">(URL template for /p/ parts mode)</span></label>
              <input type="text" v-model="subdomain.source_template" placeholder="https://cdn.example.com/{1}/{2}.{0}?token={3}">
              <div class="field-note">
                Use placeholders {0}, {1}, {2}... that map to /p/part0/part1/part2/... in request URLs
              </div>
            </div>
          </div>
        </div>

        <!-- Source Allowlist -->
        <div class="form-section">
          <div class="section-header" style="color: #7fdbca;">
            🛡️ Source Allowlist
          </div>
          <div class="form-field full-width">
            <label>
              <span class="field-name">allowed_sources</span> 
              <span class="field-hint">(one per line, leave empty to use global)</span>
            </label>
            <textarea 
              v-model="allowedSourcesText" 
              placeholder="Leave empty to use global patterns&#10;cdn.example.com&#10;*.storage.example.com"
              rows="4"
            ></textarea>
            <div class="field-note" v-if="parsedAllowedSources.length > 0">
              {{ parsedAllowedSources.length }} custom pattern(s) configured
            </div>
            <div class="field-note" v-else>
              Using global patterns
            </div>
          </div>
        </div>

        <!-- Signing Configuration -->
        <div class="form-section">
          <div class="section-header">
            🔐 URL Signing
          </div>
          <div class="form-fields">
            <div class="form-field full-width">
              <label><span class="field-name">signing_key</span> <span class="field-hint">(HMAC secret)</span></label>
              <div class="password-field">
                <input 
                  :type="showKey ? 'text' : 'password'" 
                  v-model="subdomain.signing_key" 
                  placeholder="Leave empty to disable signing"
                >
                <button class="password-toggle" type="button" @click="showKey = !showKey">
                  {{ showKey ? '🙈' : '👁️' }}
                </button>
              </div>
            </div>
            <div class="form-field">
              <div class="checkbox-field">
                <input type="checkbox" id="signing-required" v-model="subdomain.signing_required">
                <label for="signing-required">
                  <span class="field-name">signing_required</span>
                </label>
              </div>
            </div>
          </div>
        </div>

        <!-- Templates (only shown for existing subdomains) -->
        <div class="form-section" v-if="!isNew">
          <div class="section-header" style="color: #ffcb6b;">
            📋 Templates
            <span class="section-hint">Use /t/name/ in URLs to apply preset params</span>
          </div>
          
          <!-- Template List -->
          <div class="template-list" v-if="templates.length > 0">
            <div 
              v-for="(tmpl, idx) in templates" 
              :key="idx" 
              class="template-item"
              :class="{ editing: editingTemplateIndex === idx }"
            >
              <div class="template-content">
                <div class="template-row">
                  <div class="template-info">
                    <span class="template-name">{{ tmpl.name }}</span>
                    <span v-if="subdomain.default_template === tmpl.name" class="default-badge">DEFAULT</span>
                    <span class="template-params">
                      <code v-if="tmpl.width || tmpl.height">{{ tmpl.width || '∞' }}×{{ tmpl.height || '∞' }}</code>
                      <code v-if="tmpl.fit">{{ tmpl.fit }}</code>
                      <code v-if="tmpl.quality">q{{ tmpl.quality }}</code>
                      <code v-if="tmpl.format">{{ tmpl.format }}</code>
                      <code v-if="tmpl.source_template" class="has-template" title="Has custom source template">🔗 custom</code>
                    </span>
                  </div>
                  <div class="template-actions">
                    <button type="button" class="btn-template-edit" @click="editTemplate(idx)">Edit</button>
                    <button type="button" class="btn-template-delete" @click="deleteTemplate(idx)">×</button>
                  </div>
                </div>
                <div v-if="tmpl.source_template" class="template-source-url">
                  {{ tmpl.source_template }}
                </div>
                <div class="template-example-url">
                  <span class="example-label">Example:</span>
                  <a :href="`${subdomainUrl}/resize/t/${tmpl.name}/u/${currentOrigin}/android-chrome-512x512.png`" target="_blank" rel="noopener">
                    {{ subdomainUrl }}/resize/t/{{ tmpl.name }}/u/{{ currentOrigin }}/android-chrome-512x512.png
                  </a>
                </div>
              </div>
            </div>
          </div>
          <div v-else class="template-empty">
            No templates configured
          </div>

          <!-- Template Editor -->
          <div v-if="showTemplateEditor" class="template-editor">
            <div class="template-editor-header">
              {{ editingTemplateIndex !== null ? 'Edit Template' : 'New Template' }}
            </div>
            <div class="template-editor-fields">
              <div class="form-field">                
                <input 
                  type="text" 
                  v-model="editingTemplate.name" 
                  placeholder="thumbnail, hero, og-image..."
                  @keydown.enter.prevent="saveTemplate"
                >
              </div>
              <div class="form-field">
                <label><span class="field-name">width</span></label>
                <input type="number" v-model.number="editingTemplate.width" min="1" placeholder="optional">
              </div>
              <div class="form-field">
                <label><span class="field-name">height</span></label>
                <input type="number" v-model.number="editingTemplate.height" min="1" placeholder="optional">
              </div>
              <div class="form-field">
                <label><span class="field-name">fit</span></label>
                <select v-model="editingTemplate.fit">
                  <option value="">default (cover)</option>
                  <option value="cover">cover</option>
                  <option value="contain">contain</option>
                  <option value="fill">fill</option>
                </select>
              </div>
              <div class="form-field">
                <label><span class="field-name">quality</span> <span class="field-hint">(1-100)</span></label>
                <input type="number" v-model.number="editingTemplate.quality" min="1" max="100" placeholder="default 85">
              </div>
              <div class="form-field">
                <label><span class="field-name">format</span></label>
                <select v-model="editingTemplate.format">
                  <option value="">auto (from source)</option>
                  <option value="jpg">jpg</option>
                  <option value="png">png</option>
                  <option value="webp">webp</option>
                  <option value="gif">gif</option>
                </select>
              </div>
              <div class="form-field full-width">
                <label><span class="field-name">source_template</span> <span class="field-hint">(optional, overrides subdomain template for /p/ mode)</span></label>
                <input 
                  type="text" 
                  v-model="editingTemplate.source_template" 
                  :placeholder="subdomain.source_template || 'https://cdn.example.com/{1}/{2}.{0}'"
                >
                <div class="field-note" v-if="!editingTemplate.source_template && subdomain.source_template">
                  Using subdomain template: {{ subdomain.source_template }}
                </div>
                <div class="field-note" v-else-if="!editingTemplate.source_template">
                  No template set (requires /u/ URL mode)
                </div>
              </div>
              <div class="form-field full-width" v-if="editingTemplateIndex !== null && editingTemplate.name">
                <div class="checkbox-field make-default-checkbox">
                  <input 
                    type="checkbox" 
                    id="make-default" 
                    :checked="subdomain.default_template === editingTemplate.name"
                    @change="setDefaultTemplate(editingTemplate.name)"
                  >
                  <label for="make-default">
                    <span class="field-name">MAKE DEFAULT</span>
                    <span class="field-hint">&nbsp;(used when template not found or not specified)</span>
                  </label>
                </div>
              </div>
            </div>
            <!-- Live Example URL Preview -->
            <div class="template-editor-preview" v-if="editingTemplate.name">
              <span class="preview-label">Example URL:</span>
              <a 
                :href="`${subdomainUrl}/resize/t/${editingTemplate.name}/u/${currentOrigin}/android-chrome-512x512.png`" 
                target="_blank" 
                rel="noopener"
              >
                {{ subdomainUrl }}/resize/t/{{ editingTemplate.name }}/u/{{ currentOrigin }}/android-chrome-512x512.png
              </a>
            </div>

            <div class="template-editor-actions">
              <button type="button" class="btn-template-cancel" @click="cancelTemplateEdit">Cancel</button>
              <button type="button" class="btn-template-save" @click="saveTemplate">
                {{ editingTemplateIndex !== null ? 'Update' : 'Add' }}
              </button>
            </div>
          </div>

          <!-- Add Template Button -->
          <div v-if="!showTemplateEditor" class="template-buttons">
            <button 
              type="button" 
              class="btn-add-template" 
              @click="startNewTemplate"
            >
              + Add Template
            </button>
            <button 
              type="button" 
              class="btn-import-template" 
              @click="showImportModal = true"
            >
              📥 Import JSON
            </button>
          </div>
        </div>

        <!-- Import Templates Modal -->
        <div v-if="showImportModal" class="import-modal-overlay" @click.self="closeImportModal">
          <div class="import-modal">
            <div class="import-modal-header">
              <span class="import-modal-title">📥 Import Templates from JSON</span>
              <button type="button" class="import-modal-close" @click="closeImportModal">×</button>
            </div>
            <div class="import-modal-body">
              <div class="import-format-hint">
                Paste JSON with this structure:
                <pre class="import-format-example">{
  "templates": {
    "xs": { "w": 100, "h": 10000, "q": 70, "fit": "contain" },
    "max": { "w": 3840, "h": 3840, "q": 90 }
  }
}</pre>
                <div class="import-format-legend">
                  <code>w</code> → width, <code>h</code> → height, <code>q</code> → quality, <code>fit</code> → fit mode
                </div>
                <div class="import-format-note">
                  ⚠️ Both width and height are required for fit modes
                </div>
              </div>
              <button type="button" class="btn-add-defaults" @click="insertDefaultTemplates">
                ✨ Add Default Templates
              </button>
              <textarea 
                v-model="importJsonText" 
                placeholder="Paste your template JSON here..."
                class="import-textarea"
                rows="8"
              ></textarea>
              <div class="import-options">
                <label class="import-checkbox">
                  <input type="checkbox" v-model="importOverwrite">
                  <span>Overwrite existing templates</span>
                </label>
              </div>
              <div v-if="importError" class="import-error">{{ importError }}</div>
              <div v-if="importPreview.length > 0" class="import-preview">
                <div class="import-preview-header">Preview ({{ importPreview.length }} template{{ importPreview.length !== 1 ? 's' : '' }}):</div>
                <div class="import-preview-list">
                  <div v-for="tmpl in importPreview" :key="tmpl.name" class="import-preview-item">
                    <span class="import-preview-name">{{ tmpl.name }}</span>
                    <span class="import-preview-params">
                      <code v-if="tmpl.width || tmpl.height">{{ tmpl.width || '∞' }}×{{ tmpl.height || '∞' }}</code>
                      <code v-if="tmpl.fit">{{ tmpl.fit }}</code>
                      <code v-if="tmpl.quality">q{{ tmpl.quality }}</code>
                    </span>
                  </div>
                </div>
              </div>
            </div>
            <div class="import-modal-actions">
              <button type="button" class="btn-import-cancel" @click="closeImportModal">Cancel</button>
              <button 
                type="button" 
                class="btn-import-confirm" 
                @click="confirmImport"
                :disabled="importPreview.length === 0 || importingTemplates"
              >
                {{ importingTemplates ? 'Importing...' : `Import ${importPreview.length} Template${importPreview.length !== 1 ? 's' : ''}` }}
              </button>
            </div>
          </div>
        </div>

        <div v-if="statusMessage" class="status" :class="statusType === 'success' ? 'status-success' : 'status-error'">
          {{ statusMessage }}
        </div>

        <div class="form-actions">
          <router-link to="/subdomains" class="btn-cancel">Cancel</router-link>
          <button v-if="!isNew" type="button" class="btn-delete" @click="deleteSubdomain">Delete</button>
          <button type="submit" class="btn-save" :disabled="saving">
            {{ saving ? 'Saving...' : 'Save' }}
          </button>
        </div>
      </form>
    </div>
  </AdminLayout>
</template>

<script setup>
import { ref, computed, onMounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import AdminLayout from '../components/AdminLayout.vue'
import { useApi } from '../composables/useApi'
import { useSubdomain } from '../composables/useSubdomain'
import { useConfig } from '../composables/useConfig'

const route = useRoute()
const router = useRouter()
const { authFetch } = useApi()
const { currentOrigin } = useSubdomain()
const { config } = useConfig()

const apiUrl = computed(() => config.value?.apiUrl || '')

// Construct the URL for the subdomain being edited (e.g., images.example.com)
// Uses baseDomain from config if available, otherwise falls back to apiUrl hostname
const subdomainUrl = computed(() => {
  if (!subdomain.value.name) {
    return apiUrl.value
  }
  try {
    // If baseDomain is configured, use it (e.g., "example.com")
    if (config.value?.baseDomain) {
      const url = new URL(apiUrl.value)
      return `${url.protocol}//${subdomain.value.name}.${config.value.baseDomain}`
    }
    // Fallback: prepend subdomain to apiUrl hostname
    const url = new URL(apiUrl.value)
    url.hostname = `${subdomain.value.name}.${url.hostname}`
    return url.origin
  } catch (e) {
    return apiUrl.value
  }
})

const isNew = computed(() => route.params.name === 'new')
const originalName = ref('')
const loading = ref(true)
const notFound = ref(false)
const saving = ref(false)
const savingTemplate = ref(false)
const showKey = ref(false)
const showImportModal = ref(false)
const importJsonText = ref('')
const importError = ref('')
const importingTemplates = ref(false)
const importOverwrite = ref(false)
const statusMessage = ref(null)
const statusType = ref(null)

const subdomain = ref({
  name: '',
  max_width: null,
  max_height: null,
  source_template: null,
  signing_key: null,
  signing_required: false,
  default_template: null
})

const allowedSourcesText = ref('')

const parsedAllowedSources = computed(() => {
  if (!allowedSourcesText.value) return []
  return allowedSourcesText.value.split('\n').map(p => p.trim()).filter(p => p.length > 0)
})

// Templates
const templates = ref([])
const editingTemplate = ref({
  name: '',
  width: null,
  height: null,
  fit: '',
  quality: null,
  format: '',
  source_template: ''
})
const editingTemplateIndex = ref(null)

const showTemplateEditor = computed(() => {
  return editingTemplateIndex.value !== null || editingTemplate.value._isNew
})

// Import templates preview
const importPreview = computed(() => {
  if (!importJsonText.value.trim()) {
    importError.value = ''
    return []
  }
  
  try {
    const parsed = JSON.parse(importJsonText.value)
    
    if (!parsed.templates || typeof parsed.templates !== 'object') {
      importError.value = 'JSON must have a "templates" object'
      return []
    }
    
    const results = []
    for (const [name, params] of Object.entries(parsed.templates)) {
      if (typeof params !== 'object' || params === null) continue
      
      results.push({
        name,
        width: params.w || params.width || null,
        height: params.h || params.height || null,
        quality: params.q || params.quality || null,
        fit: params.fit || null,
        format: params.format || params.f || null,
        source_template: params.source_template || params.src || null
      })
    }
    
    if (results.length === 0) {
      importError.value = 'No valid templates found in JSON'
      return []
    }
    
    importError.value = ''
    return results
  } catch (e) {
    importError.value = `Invalid JSON: ${e.message}`
    return []
  }
})

const closeImportModal = () => {
  showImportModal.value = false
  importJsonText.value = ''
  importError.value = ''
  importOverwrite.value = false
}

const insertDefaultTemplates = () => {
  importJsonText.value = `{
  "templates": {
    "xs": { "w": 100, "h": 100, "q": 70, "fit": "contain" },
    "xsm": { "w": 100, "h": 100, "q": 70, "fit": "contain" },
    "xsh": { "w": 100, "h": 100, "q": 80, "fit": "contain" },
    
    "sm": { "w": 300, "h":  300, "q": 80, "fit": "contain" },
    "smm": { "w": 300, "h": 300, "q": 70, "fit": "contain" },
    "smh": { "w": 300, "h": 300, "q": 80, "fit": "contain" },
    
    "md": { "w": 600, "h":  600, "q": 80, "fit": "contain" },
    "mdm": { "w": 600, "h":  600, "q": 70, "fit": "contain" },
    "mdh": { "w": 600, "h":  600, "q": 80, "fit": "contain" },
    
    "lg": { "w": 1200, "h":  1200, "q": 80, "fit": "contain" },
    "lgm": { "w": 1200, "h":  1200, "q": 70, "fit": "contain" },
    "lgh": { "w": 1200, "h":  1200, "q": 80, "fit": "contain" },
    
    "xl": { "w": 1920, "h":  1920, "q": 85, "fit": "contain" },
    "xlm": { "w": 1920, "h":  1920, "q": 75, "fit": "contain" },
    "xlh": { "w": 1920, "h":  1920, "q": 80, "fit": "contain" },
    
    "max": { "w": 4096, "h": 4096, "q": 90, "fit": "contain" }
  }
}`
}

const confirmImport = async () => {
  if (importPreview.value.length === 0) return
  
  importingTemplates.value = true
  let successCount = 0
  let errors = []
  
  for (const tmpl of importPreview.value) {
    // Check if template name already exists
    const existingIdx = templates.value.findIndex(t => t.name === tmpl.name)
    if (existingIdx !== -1 && !importOverwrite.value) {
      errors.push(`"${tmpl.name}" already exists (skipped)`)
      continue
    }
    
    const cleaned = {
      name: tmpl.name,
      width: tmpl.width || null,
      height: tmpl.height || null,
      fit: tmpl.fit || null,
      quality: tmpl.quality || null,
      format: tmpl.format || null,
      source_template: tmpl.source_template || null
    }
    
    try {
      const res = await authFetch(
        `/config007/subdomain/${encodeURIComponent(subdomain.value.name)}/template/${encodeURIComponent(cleaned.name)}`,
        {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(cleaned)
        }
      )
      
      if (res.ok) {
        if (existingIdx !== -1) {
          // Update existing template
          templates.value[existingIdx] = cleaned
        } else {
          // Add new template
          templates.value.push(cleaned)
        }
        successCount++
      } else {
        const error = await res.json()
        errors.push(`"${tmpl.name}": ${error.error || 'Failed'}`)
      }
    } catch (e) {
      errors.push(`"${tmpl.name}": Network error`)
    }
  }
  
  importingTemplates.value = false
  closeImportModal()
  
  if (successCount > 0) {
    statusMessage.value = `Imported ${successCount} template${successCount !== 1 ? 's' : ''}` + 
      (errors.length > 0 ? ` (${errors.length} skipped/failed)` : '')
    statusType.value = 'success'
    setTimeout(() => { statusMessage.value = null }, 3000)
  } else if (errors.length > 0) {
    statusMessage.value = errors.join(', ')
    statusType.value = 'error'
  }
}

const startNewTemplate = () => {
  editingTemplate.value = {
    name: '',
    width: null,
    height: null,
    fit: '',
    quality: null,
    format: '',
    source_template: '',
    _isNew: true
  }
  editingTemplateIndex.value = null
}

const editTemplate = (idx) => {
  const tmpl = templates.value[idx]
  editingTemplate.value = {
    name: tmpl.name || '',
    width: tmpl.width || null,
    height: tmpl.height || null,
    fit: tmpl.fit || '',
    quality: tmpl.quality || null,
    format: tmpl.format || '',
    source_template: tmpl.source_template || ''
  }
  editingTemplateIndex.value = idx
}

// Delete template via API
const deleteTemplate = async (idx) => {
  const templateName = templates.value[idx].name
  if (!confirm(`Delete template "${templateName}"?`)) {
    return
  }
  
  try {
    const res = await authFetch(
      `/config007/subdomain/${encodeURIComponent(subdomain.value.name)}/template/${encodeURIComponent(templateName)}`,
      { method: 'DELETE' }
    )
    
    if (res.ok) {
      templates.value.splice(idx, 1)
      if (editingTemplateIndex.value === idx) {
        cancelTemplateEdit()
      } else if (editingTemplateIndex.value !== null && editingTemplateIndex.value > idx) {
        editingTemplateIndex.value--
      }
      statusMessage.value = 'Template deleted'
      statusType.value = 'success'
      setTimeout(() => { statusMessage.value = null }, 1500)
    } else {
      const error = await res.json()
      alert(`Failed to delete: ${error.error || 'Unknown error'}`)
    }
  } catch (e) {
    alert(`Network error: ${e.message}`)
  }
}

// Save template via API (PUT)
const saveTemplate = async () => {
  if (!editingTemplate.value.name || editingTemplate.value.name.trim() === '') {
    return
  }
  
  // Check for duplicate names (except when editing same index)
  const existingIdx = templates.value.findIndex(t => t.name === editingTemplate.value.name.trim())
  if (existingIdx !== -1 && existingIdx !== editingTemplateIndex.value) {
    alert(`Template "${editingTemplate.value.name}" already exists`)
    return
  }
  
  const cleaned = {
    name: editingTemplate.value.name.trim(),
    width: editingTemplate.value.width || null,
    height: editingTemplate.value.height || null,
    fit: editingTemplate.value.fit || null,
    quality: editingTemplate.value.quality || null,
    format: editingTemplate.value.format || null,
    source_template: editingTemplate.value.source_template || null
  }
  
  savingTemplate.value = true
  
  try {
    const res = await authFetch(
      `/config007/subdomain/${encodeURIComponent(subdomain.value.name)}/template/${encodeURIComponent(cleaned.name)}`,
      {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(cleaned)
      }
    )
    
    if (res.ok) {
      // Update local state
      if (editingTemplateIndex.value !== null) {
        templates.value[editingTemplateIndex.value] = cleaned
      } else {
        templates.value.push(cleaned)
      }
      cancelTemplateEdit()
      statusMessage.value = 'Template saved'
      statusType.value = 'success'
      setTimeout(() => { statusMessage.value = null }, 1500)
    } else {
      const error = await res.json()
      statusMessage.value = error.error || 'Failed to save template'
      statusType.value = 'error'
    }
  } catch (e) {
    statusMessage.value = 'Network error'
    statusType.value = 'error'
  }
  
  savingTemplate.value = false
}

const cancelTemplateEdit = () => {
  editingTemplate.value = {
    name: '',
    width: null,
    height: null,
    fit: '',
    quality: null,
    format: '',
    source_template: ''
  }
  editingTemplateIndex.value = null
}

// Set or clear the default template
const setDefaultTemplate = async (templateName) => {
  const isAlreadyDefault = subdomain.value.default_template === templateName
  const newDefault = isAlreadyDefault ? null : templateName
  
  try {
    const payload = {}
    if (newDefault) {
      payload.default_template = newDefault
    } else {
      payload.clear_default_template = true
    }
    
    const res = await authFetch(
      `/config007/subdomain/${encodeURIComponent(subdomain.value.name)}/params`,
      {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload)
      }
    )
    
    if (res.ok) {
      subdomain.value.default_template = newDefault
      statusMessage.value = newDefault ? `"${newDefault}" is now the default template` : 'Default template cleared'
      statusType.value = 'success'
      setTimeout(() => { statusMessage.value = null }, 2000)
    } else {
      const error = await res.json()
      statusMessage.value = error.error || 'Failed to set default template'
      statusType.value = 'error'
    }
  } catch (e) {
    statusMessage.value = 'Network error'
    statusType.value = 'error'
  }
}

const loadSubdomain = async () => {
  if (isNew.value) {
    loading.value = false
    return
  }

  loading.value = true
  notFound.value = false
  
  try {
    const res = await authFetch('/config007/subdomains')
    if (res.ok) {
      const data = await res.json()
      const found = data.find(s => s.name === route.params.name)
      if (found) {
        subdomain.value = {
          name: found.name,
          max_width: found.max_width || null,
          max_height: found.max_height || null,
          source_template: found.source_template || null,
          signing_key: found.signing_key || null,
          signing_required: found.signing_required || false,
          default_template: found.default_template || null
        }
        allowedSourcesText.value = (found.allowed_sources || []).join('\n')
        templates.value = (found.templates || []).map(t => ({
          name: t.name || '',
          width: t.width || null,
          height: t.height || null,
          fit: t.fit || null,
          quality: t.quality || null,
          format: t.format || null,
          source_template: t.source_template || null
        }))
        originalName.value = found.name
      } else {
        notFound.value = true
      }
    }
  } catch (e) {
    notFound.value = true
  }
  
  loading.value = false
}

const saveSubdomain = async () => {
  if (!subdomain.value.name || subdomain.value.name.trim() === '') {
    statusMessage.value = 'Subdomain name is required'
    statusType.value = 'error'
    setTimeout(() => { statusMessage.value = null }, 3000)
    return
  }
  
  saving.value = true
  statusMessage.value = null
  
  try {
    const allowedSources = parsedAllowedSources.value.length > 0 ? parsedAllowedSources.value : null
    
    if (isNew.value) {
      // For NEW subdomains: create with params only (no templates)
      const cleanedSubdomain = {
        name: subdomain.value.name,
        max_width: subdomain.value.max_width || null,
        max_height: subdomain.value.max_height || null,
        source_template: subdomain.value.source_template || null,
        signing_key: subdomain.value.signing_key || null,
        signing_required: subdomain.value.signing_required || false,
        allowed_sources: allowedSources,
        templates: null
      }
      
      const res = await authFetch(`/config007/subdomain/${encodeURIComponent(subdomain.value.name)}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(cleanedSubdomain)
      })
      
      if (res.ok) {
        statusMessage.value = 'Subdomain created! You can now add templates.'
        statusType.value = 'success'
        // Redirect to edit the new subdomain (where templates can be added)
        setTimeout(() => {
          router.push(`/subdomains/${encodeURIComponent(subdomain.value.name)}`)
        }, 1000)
      } else {
        const error = await res.json()
        statusMessage.value = error.error || 'Failed to create'
        statusType.value = 'error'
      }
    } else {
      // For EXISTING subdomains: use PATCH to update just params (templates saved separately)
      const payload = {}
      
      if (subdomain.value.max_width) payload.max_width = subdomain.value.max_width
      if (subdomain.value.max_height) payload.max_height = subdomain.value.max_height
      if (subdomain.value.source_template) {
        payload.source_template = subdomain.value.source_template
      } else {
        payload.clear_source_template = true
      }
      if (subdomain.value.signing_key) {
        payload.signing_key = subdomain.value.signing_key
      } else {
        payload.clear_signing_key = true
      }
      payload.signing_required = subdomain.value.signing_required || false
      
      if (allowedSources && allowedSources.length > 0) {
        payload.allowed_sources = allowedSources
      } else {
        payload.clear_allowed_sources = true
      }
      
      const res = await authFetch(`/config007/subdomain/${encodeURIComponent(subdomain.value.name)}/params`, {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload)
      })
      
      if (res.ok) {
        statusMessage.value = 'Settings saved!'
        statusType.value = 'success'
        setTimeout(() => { statusMessage.value = null }, 3000)
      } else {
        const error = await res.json()
        statusMessage.value = error.error || 'Failed to save'
        statusType.value = 'error'
      }
    }
  } catch (e) {
    statusMessage.value = 'Network error'
    statusType.value = 'error'
  }
  
  saving.value = false
}

const deleteSubdomain = async () => {
  if (!confirm(`Delete subdomain "${subdomain.value.name}"?`)) {
    return
  }
  
  try {
    const res = await authFetch(`/config007/subdomain/${encodeURIComponent(subdomain.value.name)}`, {
      method: 'DELETE'
    })
    if (!res.ok) {
      const error = await res.json()
      alert(`Failed to delete: ${error.error || 'Unknown error'}`)
      return
    }
    router.push('/subdomains')
  } catch (e) {
    alert(`Network error: ${e.message}`)
  }
}

// Watch for route changes (in case someone navigates from edit to new or vice versa)
watch(() => route.params.name, () => {
  if (route.params.name === 'new') {
    subdomain.value = {
      name: '',
      max_width: null,
      max_height: null,
      source_template: null,
      signing_key: null,
      signing_required: false,
      default_template: null
    }
    allowedSourcesText.value = ''
    templates.value = []
    cancelTemplateEdit()
    originalName.value = ''
    loading.value = false
    notFound.value = false
  } else {
    loadSubdomain()
  }
})

onMounted(() => {
  loadSubdomain()
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
  gap: 0.75rem;
  margin-bottom: 0.5rem;
}

.back-btn {
  color: #82aaff;
  text-decoration: none;
  font-size: 0.85rem;
  padding: 0.4rem 0.75rem;
  border-radius: 6px;
  background: rgba(130, 170, 255, 0.1);
  transition: all 0.2s ease;
}

.back-btn:hover {
  background: rgba(130, 170, 255, 0.2);
}

.emoji-logo {
  font-size: 1.75rem;
}

.subtitle {
  color: #666;
  font-size: 0.8rem;
  margin-bottom: 2rem;
  padding-bottom: 1.5rem;
  border-bottom: 1px solid #1f1f2f;
}

.loading-state,
.error-state {
  text-align: center;
  padding: 3rem;
  color: #666;
}

.error-icon {
  font-size: 2.5rem;
  margin-bottom: 1rem;
}

.error-message {
  color: #ff6b9d;
  font-size: 1rem;
  margin-bottom: 1.5rem;
}

.btn-back {
  color: #82aaff;
  text-decoration: none;
  padding: 0.5rem 1rem;
  border-radius: 6px;
  background: rgba(130, 170, 255, 0.1);
  transition: all 0.2s ease;
}

.btn-back:hover {
  background: rgba(130, 170, 255, 0.2);
}

.subdomain-form {
  display: flex;
  flex-direction: column;
  gap: 1.5rem;
}

.form-section {
  background: #0f0f18;
  border: 1px solid #2a2a3a;
  border-radius: 12px;
  padding: 1.25rem;
}

.section-header {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-bottom: 1rem;
  color: #ff6b9d;
  font-size: 0.8rem;
  font-weight: 500;
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.form-fields {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
  gap: 1rem;
}

.form-field {
  display: flex;
  flex-direction: column;
}

.form-field.full-width {
  grid-column: 1 / -1;
}

.form-field label {
  font-size: 0.75rem;
  margin-bottom: 0.4rem;
  color: #888;
}

.field-name {
  color: #c792ea;
  font-weight: 500;
}

.field-hint {
  color: #555;
}

.field-note {
  color: #555;
  font-size: 0.7rem;
  margin-top: 0.35rem;
}

.form-field input,
.form-field textarea {
  width: 100%;
  padding: 0.6rem 0.8rem;
  font-size: 0.85rem;
  background: #0a0a0f;
  border: 1px solid #2a2a3a;
  border-radius: 6px;
  color: #e0e0e0;
  font-family: inherit;
  transition: all 0.2s ease;
}

.form-field input:focus,
.form-field textarea:focus {
  border-color: #c792ea;
  outline: none;
  box-shadow: 0 0 0 2px rgba(199, 146, 234, 0.1);
}

.form-field input.readonly {
  background: #15151f;
  color: #888;
  cursor: not-allowed;
}

.form-field input::placeholder,
.form-field textarea::placeholder {
  color: #444;
}

.form-field textarea {
  resize: vertical;
  min-height: 80px;
}

.password-field {
  position: relative;
  display: flex;
  align-items: center;
}

.password-field input {
  flex: 1;
  padding-right: 2.5rem;
}

.password-toggle {
  position: absolute;
  right: 0.5rem;
  background: none;
  border: none;
  color: #666;
  cursor: pointer;
  padding: 0.25rem;
  font-size: 0.9rem;
}

.password-toggle:hover {
  color: #888;
}

.checkbox-field {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-top: 0.5rem;
}

.checkbox-field input[type="checkbox"] {
  width: auto;
  accent-color: #ff6b9d;
}

.checkbox-field label {
  margin: 0;
  color: #888;
  font-size: 0.85rem;
}

.status {
  padding: 0.75rem 1rem;
  border-radius: 8px;
  font-size: 0.85rem;
  text-align: center;
}

.status-success {
  background: rgba(127, 219, 202, 0.1);
  color: #7fdbca;
  border: 1px solid rgba(127, 219, 202, 0.2);
}

.status-error {
  background: rgba(255, 107, 157, 0.1);
  color: #ff6b9d;
  border: 1px solid rgba(255, 107, 157, 0.2);
}

.form-actions {
  display: flex;
  gap: 0.75rem;
  justify-content: flex-end;
  padding-top: 1rem;
  border-top: 1px solid #1f1f2f;
}

.btn-cancel {
  background: transparent;
  border: 1px solid #2a2a3a;
  color: #888;
  padding: 0.75rem 1.25rem;
  border-radius: 8px;
  font-family: inherit;
  font-size: 0.85rem;
  cursor: pointer;
  text-decoration: none;
  transition: all 0.2s ease;
}

.btn-cancel:hover {
  border-color: #3a3a4a;
  color: #aaa;
  background: rgba(255,255,255,0.03);
}

.btn-delete {
  background: transparent;
  border: 1px solid #ff6b9d33;
  color: #ff6b9d;
  padding: 0.75rem 1.25rem;
  border-radius: 8px;
  font-family: inherit;
  font-size: 0.85rem;
  cursor: pointer;
  transition: all 0.2s ease;
}

.btn-delete:hover {
  background: rgba(255, 107, 157, 0.1);
  border-color: #ff6b9d;
}

.btn-save {
  background: linear-gradient(135deg, #7fdbca 0%, #64b5a0 100%);
  border: none;
  color: #0a0a0f;
  padding: 0.75rem 1.5rem;
  border-radius: 8px;
  font-family: inherit;
  font-size: 0.85rem;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.2s ease;
}

.btn-save:hover {
  transform: translateY(-1px);
  box-shadow: 0 4px 12px rgba(127, 219, 202, 0.3);
}

.btn-save:disabled {
  opacity: 0.5;
  cursor: not-allowed;
  transform: none;
  box-shadow: none;
}

/* Templates Section */
.section-hint {
  margin-left: auto;
  font-size: 0.65rem;
  font-weight: 400;
  color: #555;
  text-transform: none;
  letter-spacing: 0;
}

.template-list {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  margin-bottom: 1rem;
}

.template-item {
  background: #0a0a0f;
  border: 1px solid #2a2a3a;
  border-radius: 8px;
  padding: 0.6rem 0.85rem;
  transition: all 0.2s ease;
}

.template-item:hover {
  border-color: #3a3a4a;
}

.template-item.editing {
  border-color: #ffcb6b;
  background: rgba(255, 203, 107, 0.05);
}

.template-content {
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
}

.template-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.template-info {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  flex: 1;
  min-width: 0;
}

.template-name {
  color: #ffcb6b;
  font-weight: 600;
  font-size: 0.85rem;
}

.default-badge {
  background: linear-gradient(135deg, #7fdbca 0%, #64b5a0 100%);
  color: #0a0a0f;
  font-size: 0.6rem;
  font-weight: 700;
  padding: 0.15rem 0.4rem;
  border-radius: 4px;
  letter-spacing: 0.05em;
  text-transform: uppercase;
}

.make-default-checkbox {
  margin-top: 0.75rem;
  padding: 0.6rem 0.75rem;
  background: rgba(127, 219, 202, 0.05);
  border: 1px solid rgba(127, 219, 202, 0.2);
  border-radius: 6px;
}

.make-default-checkbox label {
  display: flex;
  align-items: center;
  cursor: pointer;
}

.make-default-checkbox input[type="checkbox"] {
  accent-color: #7fdbca;
}

.template-params {
  display: flex;
  flex-wrap: wrap;
  gap: 0.4rem;
}

.template-params code {
  font-size: 0.7rem;
  color: #82aaff;
  background: rgba(130, 170, 255, 0.1);
  padding: 0.15rem 0.4rem;
  border-radius: 4px;
}

.template-actions {
  display: flex;
  gap: 0.4rem;
  flex-shrink: 0;
}

.template-example-url {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding-top: 0.35rem;
  border-top: 1px solid #1a1a26;
  margin-top: 0.25rem;
}

.template-example-url .example-label {
  color: #555;
  font-size: 0.65rem;
  text-transform: uppercase;
  letter-spacing: 0.03em;
  flex-shrink: 0;
}

.template-example-url a {
  font-size: 0.68rem;
  color: #82aaff;
  opacity: 0.7;
  word-break: break-all;
  text-decoration: none;
  transition: opacity 0.2s ease;
}

.template-example-url a:hover {
  opacity: 1;
  text-decoration: underline;
}

.template-params code.has-template {
  color: #c792ea;
  background: rgba(199, 146, 234, 0.1);
}

.template-source-url {
  font-size: 0.65rem;
  color: #546e7a;
  margin-top: 0.25rem;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.btn-template-edit {
  background: transparent;
  border: 1px solid rgba(130, 170, 255, 0.3);
  color: #82aaff;
  padding: 0.3rem 0.6rem;
  border-radius: 4px;
  font-family: inherit;
  font-size: 0.7rem;
  cursor: pointer;
  transition: all 0.2s ease;
}

.btn-template-edit:hover {
  background: rgba(130, 170, 255, 0.1);
  border-color: #82aaff;
}

.btn-template-delete {
  background: transparent;
  border: 1px solid rgba(255, 107, 157, 0.3);
  color: #ff6b9d;
  padding: 0.3rem 0.5rem;
  border-radius: 4px;
  font-family: inherit;
  font-size: 0.8rem;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.2s ease;
}

.btn-template-delete:hover {
  background: rgba(255, 107, 157, 0.1);
  border-color: #ff6b9d;
}

.template-empty {
  color: #555;
  font-size: 0.8rem;
  text-align: center;
  padding: 1rem;
  background: #0a0a0f;
  border: 1px dashed #2a2a3a;
  border-radius: 8px;
  margin-bottom: 1rem;
}

.template-editor {
  background: rgba(255, 203, 107, 0.03);
  border: 1px solid rgba(255, 203, 107, 0.2);
  border-radius: 10px;
  padding: 1rem;
  margin-bottom: 1rem;
}

.template-editor-header {
  color: #ffcb6b;
  font-size: 0.75rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  margin-bottom: 0.85rem;
}

.template-editor-fields {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(120px, 1fr));
  gap: 0.75rem;
}

.template-editor-fields .form-field {
  margin: 0;
}

.template-editor-fields .form-field label {
  font-size: 0.7rem;
}

.template-editor-fields .form-field input,
.template-editor-fields .form-field select {
  padding: 0.5rem 0.65rem;
  font-size: 0.8rem;
}

.template-editor-fields select {
  width: 100%;
  padding: 0.5rem 0.65rem;
  font-size: 0.8rem;
  background: #0a0a0f;
  border: 1px solid #2a2a3a;
  border-radius: 6px;
  color: #e0e0e0;
  font-family: inherit;
  cursor: pointer;
  transition: all 0.2s ease;
}

.template-editor-fields select:focus {
  border-color: #ffcb6b;
  outline: none;
  box-shadow: 0 0 0 2px rgba(255, 203, 107, 0.1);
}

.template-editor-preview {
  grid-column: 1 / -1;
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-top: 0.5rem;
  padding: 0.6rem 0.75rem;
  background: rgba(130, 170, 255, 0.05);
  border: 1px solid rgba(130, 170, 255, 0.15);
  border-radius: 6px;
}

.template-editor-preview .preview-label {
  color: #666;
  font-size: 0.65rem;
  text-transform: uppercase;
  letter-spacing: 0.03em;
  flex-shrink: 0;
}

.template-editor-preview a {
  font-size: 0.7rem;
  color: #82aaff;
  word-break: break-all;
  text-decoration: none;
  transition: opacity 0.2s ease;
}

.template-editor-preview a:hover {
  text-decoration: underline;
}

.template-editor-actions {
  display: flex;
  gap: 0.5rem;
  justify-content: flex-end;
  margin-top: 1rem;
  padding-top: 0.85rem;
  border-top: 1px solid rgba(255, 203, 107, 0.1);
}

.btn-template-cancel {
  background: transparent;
  border: 1px solid #2a2a3a;
  color: #888;
  padding: 0.5rem 0.85rem;
  border-radius: 6px;
  font-family: inherit;
  font-size: 0.75rem;
  cursor: pointer;
  transition: all 0.2s ease;
}

.btn-template-cancel:hover {
  border-color: #3a3a4a;
  color: #aaa;
}

.btn-template-save {
  background: linear-gradient(135deg, #ffcb6b 0%, #e0a800 100%);
  border: none;
  color: #0a0a0f;
  padding: 0.5rem 1rem;
  border-radius: 6px;
  font-family: inherit;
  font-size: 0.75rem;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.2s ease;
}

.btn-template-save:hover {
  transform: translateY(-1px);
  box-shadow: 0 3px 10px rgba(255, 203, 107, 0.3);
}

.btn-add-template {
  background: transparent;
  border: 1px dashed rgba(255, 203, 107, 0.4);
  color: #ffcb6b;
  padding: 0.65rem 1rem;
  border-radius: 8px;
  font-family: inherit;
  font-size: 0.8rem;
  cursor: pointer;
  width: 100%;
  transition: all 0.2s ease;
}

.btn-add-template:hover {
  background: rgba(255, 203, 107, 0.05);
  border-color: #ffcb6b;
}

/* Template buttons container */
.template-buttons {
  display: flex;
  gap: 0.75rem;
}

.template-buttons .btn-add-template {
  flex: 1;
}

.btn-import-template {
  background: transparent;
  border: 1px dashed rgba(130, 170, 255, 0.4);
  color: #82aaff;
  padding: 0.65rem 1rem;
  border-radius: 8px;
  font-family: inherit;
  font-size: 0.8rem;
  cursor: pointer;
  transition: all 0.2s ease;
  white-space: nowrap;
}

.btn-import-template:hover {
  background: rgba(130, 170, 255, 0.05);
  border-color: #82aaff;
}

/* Import Modal */
.import-modal-overlay {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: rgba(0, 0, 0, 0.75);
  backdrop-filter: blur(4px);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
  padding: 1rem;
}

.import-modal {
  background: #12121c;
  border: 1px solid #2a2a3a;
  border-radius: 16px;
  width: 100%;
  max-width: 560px;
  max-height: 90vh;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5);
}

.import-modal-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 1rem 1.25rem;
  border-bottom: 1px solid #2a2a3a;
  background: #0f0f18;
}

.import-modal-title {
  color: #82aaff;
  font-size: 0.9rem;
  font-weight: 600;
}

.import-modal-close {
  background: none;
  border: none;
  color: #666;
  font-size: 1.5rem;
  cursor: pointer;
  padding: 0;
  line-height: 1;
  transition: color 0.2s ease;
}

.import-modal-close:hover {
  color: #ff6b9d;
}

.import-modal-body {
  padding: 1.25rem;
  overflow-y: auto;
  flex: 1;
}

.import-format-hint {
  color: #888;
  font-size: 0.75rem;
  margin-bottom: 1rem;
}

.import-format-example {
  background: #0a0a0f;
  border: 1px solid #2a2a3a;
  border-radius: 6px;
  padding: 0.75rem;
  margin: 0.5rem 0;
  font-size: 0.7rem;
  color: #7fdbca;
  overflow-x: auto;
}

.import-format-legend {
  margin-top: 0.5rem;
}

.import-format-legend code {
  color: #c792ea;
  background: rgba(199, 146, 234, 0.1);
  padding: 0.1rem 0.3rem;
  border-radius: 3px;
  font-size: 0.7rem;
}

.import-format-note {
  margin-top: 0.5rem;
  color: #ffcb6b;
  font-size: 0.7rem;
}

.btn-add-defaults {
  width: 100%;
  padding: 0.65rem 1rem;
  margin-bottom: 0.75rem;
  font-family: inherit;
  font-size: 0.8rem;
  font-weight: 500;
  background: linear-gradient(135deg, rgba(199, 146, 234, 0.15) 0%, rgba(130, 170, 255, 0.15) 100%);
  border: 1px dashed rgba(199, 146, 234, 0.5);
  border-radius: 8px;
  color: #c792ea;
  cursor: pointer;
  transition: all 0.2s ease;
}

.btn-add-defaults:hover {
  background: linear-gradient(135deg, rgba(199, 146, 234, 0.25) 0%, rgba(130, 170, 255, 0.25) 100%);
  border-color: #c792ea;
  transform: translateY(-1px);
}

.import-textarea {
  width: 100%;
  padding: 0.75rem;
  font-size: 0.8rem;
  font-family: 'JetBrains Mono', 'Fira Code', monospace;
  background: #0a0a0f;
  border: 1px solid #2a2a3a;
  border-radius: 8px;
  color: #e0e0e0;
  resize: vertical;
  min-height: 120px;
  transition: border-color 0.2s ease;
}

.import-textarea:focus {
  border-color: #82aaff;
  outline: none;
  box-shadow: 0 0 0 2px rgba(130, 170, 255, 0.1);
}

.import-textarea::placeholder {
  color: #444;
}

.import-options {
  margin-bottom: 0.75rem;
}

.import-checkbox {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  cursor: pointer;
  font-size: 0.8rem;
  color: #888;
}

.import-checkbox input[type="checkbox"] {
  width: 16px;
  height: 16px;
  accent-color: #c792ea;
  cursor: pointer;
}

.import-checkbox span {
  user-select: none;
}

.import-checkbox:hover span {
  color: #aaa;
}

.import-error {
  color: #ff6b9d;
  font-size: 0.75rem;
  margin-top: 0.5rem;
  padding: 0.5rem 0.75rem;
  background: rgba(255, 107, 157, 0.1);
  border-radius: 6px;
}

.import-preview {
  margin-top: 1rem;
  padding: 0.75rem;
  background: rgba(127, 219, 202, 0.05);
  border: 1px solid rgba(127, 219, 202, 0.2);
  border-radius: 8px;
}

.import-preview-header {
  color: #7fdbca;
  font-size: 0.75rem;
  font-weight: 600;
  margin-bottom: 0.5rem;
}

.import-preview-list {
  display: flex;
  flex-direction: column;
  gap: 0.35rem;
}

.import-preview-item {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 0.35rem 0.5rem;
  background: rgba(127, 219, 202, 0.05);
  border-radius: 4px;
}

.import-preview-name {
  color: #ffcb6b;
  font-weight: 600;
  font-size: 0.8rem;
}

.import-preview-params {
  display: flex;
  flex-wrap: wrap;
  gap: 0.3rem;
}

.import-preview-params code {
  font-size: 0.65rem;
  color: #82aaff;
  background: rgba(130, 170, 255, 0.1);
  padding: 0.1rem 0.35rem;
  border-radius: 3px;
}

.import-modal-actions {
  display: flex;
  gap: 0.75rem;
  justify-content: flex-end;
  padding: 1rem 1.25rem;
  border-top: 1px solid #2a2a3a;
  background: #0f0f18;
}

.btn-import-cancel {
  background: transparent;
  border: 1px solid #2a2a3a;
  color: #888;
  padding: 0.6rem 1rem;
  border-radius: 6px;
  font-family: inherit;
  font-size: 0.8rem;
  cursor: pointer;
  transition: all 0.2s ease;
}

.btn-import-cancel:hover {
  border-color: #3a3a4a;
  color: #aaa;
}

.btn-import-confirm {
  background: linear-gradient(135deg, #82aaff 0%, #6690e8 100%);
  border: none;
  color: #0a0a0f;
  padding: 0.6rem 1.25rem;
  border-radius: 6px;
  font-family: inherit;
  font-size: 0.8rem;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.2s ease;
}

.btn-import-confirm:hover:not(:disabled) {
  transform: translateY(-1px);
  box-shadow: 0 4px 12px rgba(130, 170, 255, 0.3);
}

.btn-import-confirm:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>
