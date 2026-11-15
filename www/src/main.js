import { createApp } from 'vue'
import App from './App.vue'
import router from './router'
import './assets/styles.css'
import { useConfig } from './composables/useConfig'

// Load config before mounting the app
const { loadConfig } = useConfig()

loadConfig().then(() => {
  const app = createApp(App)
  app.use(router)
  app.mount('#app')
})
