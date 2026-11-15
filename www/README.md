# Speedwholes Frontend

Modern Vue 3 + Vite SPA for the Speedwholes admin interface.

## Quick Start

### Development

```bash
npm install
npm run dev
```

Visit http://localhost:8081

The frontend reads its API origin from `public/config.json` (`apiUrl`). The image server is HTTPS, including in local development (`https://localhost:3000`).

### Production Build

```bash
npm run build
```

This creates optimized static files in `dist/` ready to be served by nginx or any static host.

## Project Structure

```
www/                       # Vue 3 + Vite SPA
├── src/
│   ├── views/            # Page components
│   ├── components/       # Shared components
│   ├── composables/      # Vue composables
│   ├── router/           # Route definitions
│   └── assets/           # Global CSS
├── public/               # Static assets (favicons)
├── dist/                 # Build output (gitignored)
├── index.html            # HTML entry point
├── package.json          # Dependencies
└── vite.config.js        # Vite configuration
```

## Routes

All routes use client-side routing (Vue Router):

- `/` → Redirects to `/login`
- `/login` → Authentication page
- `/settings` → Runtime configuration and live stats
- `/subdomains` → Per-subdomain configuration

## Configuration

### API URL Configuration

The frontend loads its configuration from `public/config.json`:

```json
{
  "apiUrl": "http://localhost:8089"
}
```

**How it works:**
1. On startup, the app fetches `/config.json`
2. Sets `window.SPEEDWHOLES_API_URL` to the configured `apiUrl`
3. Falls back to `http://localhost:8089` if config file is not found
4. All API calls use this URL via the `useApi` composable

**To change the backend URL:**
- Edit `public/config.json` and set `apiUrl` to your backend server address
- Example: `"apiUrl": "http://localhost:3000"` or `"apiUrl": "https://api.example.com"`

## Technology Stack

- **Vue 3** - Progressive JavaScript framework (Composition API)
- **Vue Router 4** - Client-side routing
- **Vite 5** - Fast build tool and dev server
- **Custom CSS** - Dark theme, no UI framework

## Production Deployment

### Build the Frontend

```bash
npm install
npm run build
```

### Serve with Nginx

Example nginx configuration:

```nginx
server {
    listen 80;
    server_name admin.yourdomain.com;
    root /var/www/speedwholes/dist;
    index index.html;

    # SPA fallback - serve index.html for all routes
    location / {
        try_files $uri $uri/ /index.html;
    }

    # Proxy API requests to backend
    location /config007/ {
        proxy_pass http://localhost:3000;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    }

    # Cache static assets
    location ~* \.(js|css|png|jpg|jpeg|gif|ico|svg)$ {
        expires 1y;
        add_header Cache-Control "public, immutable";
    }
}
```

### Deploy Structure

```
/var/www/speedwholes/
└── dist/
    ├── index.html
    └── assets/
        ├── *.js
        └── *.css
```

## API Integration

The Vue SPA communicates with the Speedwholes backend via:

- **Base URL**: Configured at runtime
- **API Endpoints**: `/config007/*` on the backend server
- **Authentication**: the session token is stored in `localStorage` and sent as `Authorization: Bearer`

## Features

### Settings Page
- Live statistics (requests, cache hits/misses)
- Runtime configuration
- Source URL allowlist/security
- Auto-refresh stats every 5 seconds

### Subdomain Page
- Per-subdomain configuration
- URL signing (HMAC secrets)
- Custom max dimensions per subdomain
- Source URL/template patterns

### Login
- Token-based authentication
- Session persistence
- Auto-redirect on expiry

## Development Tips

### Hot Module Reloading

Vite provides instant hot module reloading during development. Changes to `.vue` files will update immediately without full page reload.

### Debugging

- Vue DevTools: Install browser extension for component inspection
- Console logs: Check browser console for API errors
- Network tab: Monitor API requests to backend

## Troubleshooting

### Changes Not Reflecting

Make sure you're running the dev server:
```bash
npm run dev
```

### API Calls Failing

1. Verify backend is running on port 3000
2. Check browser console for CORS errors
3. Confirm backend API URL is correct

### Build Errors

Clear and reinstall dependencies:
```bash
rm -rf node_modules package-lock.json
npm install
```

## License

MIT
