# Production Deployment Guide

This guide covers deploying Speedwholes in production environments.

## Prerequisites

- Built binaries (see [Build](#building-binaries))
- A Linux server with root access
- Domain name (for HTTPS)

### Build Dependencies

The JPEG codec builds libjpeg-turbo from source. Install a C compiler, CMake, and NASM. A system libjpeg package is not used.

```bash
# RHEL / CentOS / Fedora
sudo dnf install cmake nasm gcc

# Ubuntu / Debian
sudo apt install cmake nasm build-essential
```

## Building Binaries

```bash
# Backend
cd server && cargo build --release
# Binary: server/target/release/speedwholes

# Frontend
cd www && npm install && npm run build
# Output: www/dist/
```

## Directory Structure

Recommended production layout:

```
/usr/local/bin/
├── speedwholes              # Backend binary

/etc/speedwholes/
├── speedwholes.config.json   # Main configuration
└── subdomainConfig/         # Per-subdomain configs

/var/cache/speedwholes/      # Image cache (can grow large)

/var/www/speedwholes/
└── dist/                    # Frontend static files
```

## Installation

### 1. Create User and Directories

```bash
# Create dedicated user (no login shell)
sudo useradd -r -s /bin/false speedwholes

# Create directories
sudo mkdir -p /etc/speedwholes/subdomainConfig
sudo mkdir -p /var/cache/speedwholes
sudo mkdir -p /var/www/speedwholes

# Set ownership
sudo chown -R speedwholes:speedwholes /etc/speedwholes
sudo chown -R speedwholes:speedwholes /var/cache/speedwholes
```

### 2. Install Binaries

```bash
# Copy backend binary
sudo cp server/target/release/speedwholes /usr/local/bin/
sudo chmod 755 /usr/local/bin/speedwholes

# Copy frontend
sudo cp -r www/dist /var/www/speedwholes/
```

### 3. Configure Backend

Create `/etc/speedwholes/speedwholes.config.json`:

```json
{
  "port": 3000,
  "cache_path": "/var/cache/speedwholes",
  "max_width": 4096,
  "max_height": 4096,
  "timeout": 30,
  "max_source_pixels": 100000000,
  "allowed_sources": ["images.example.com"],
  "allow_all_sources": false,
  "admin_token": "generate-a-secure-random-token"
}
```

Secure the config file:

```bash
sudo chmod 600 /etc/speedwholes/speedwholes.config.json
sudo chown speedwholes:speedwholes /etc/speedwholes/speedwholes.config.json
```

**Tip:** Generate a secure admin token:
```bash
openssl rand -hex 32
```

### 4. Configure Frontend

Edit `/var/www/speedwholes/dist/config.json`:

```json
{
  "apiUrl": "https://api.yourdomain.com"
}
```

## Running the Server

Basic command:

```bash
speedwholes --config-dir /etc/speedwholes --cache-path /var/cache/speedwholes
```

For process management, see:
- [Systemd setup](../systemd-examples/README.md) - recommended for Linux servers
- Or use your preferred process manager (supervisor, pm2, docker, etc.)

## HTTPS/TLS Setup

Speedwholes has native TLS support via rustls. For detailed TLS configuration including certificate setup, see [TLS/HTTPS Configuration](../server/README.md#tlshttps-configuration) in the server documentation.

### Quick Setup with Let's Encrypt

1. **Obtain certificate:**
   ```bash
   sudo apt install certbot
   sudo certbot certonly --standalone -d api.yourdomain.com
   ```

2. **Update config** (`/etc/speedwholes/speedwholes.config.json`):
   ```json
   {
     "port": 443,
     "tls_cert": "/etc/letsencrypt/live/api.yourdomain.com/fullchain.pem",
     "tls_key": "/etc/letsencrypt/live/api.yourdomain.com/privkey.pem"
   }
   ```

3. **Grant certificate access** without making private keys world-readable. Leave `privkey*.pem` at mode `600` and add an ACL for the service user:

   ```bash
   sudo ./scripts/install.sh --upgrade --tls-domain api.yourdomain.com
   ```

   That runs `setfacl` for that lineage only. If `setfacl` is missing: `sudo apt install acl`.

   Alternative: keep the key root-only and hand it to the service with systemd `LoadCredential=`. The credential is exposed at `$CREDENTIALS_DIRECTORY/tls.key`; set `tls_key` to that path. Do not copy the key into a world-readable file.

   ```ini
   [Service]
   LoadCredential=tls.key:/etc/letsencrypt/live/api.yourdomain.com/privkey.pem
   ```

4. **Allow binding to port 443** (if not running as root):
   ```bash
   sudo setcap 'cap_net_bind_service=+ep' /usr/local/bin/speedwholes
   ```

## nginx for the admin UI

The image server terminates its own TLS and does not listen on HTTP. nginx is how you serve the Vue admin build from `/var/www/speedwholes/dist`. Do not proxy `/resize` to `http://localhost:3000`; nothing is listening there.

```nginx
# /etc/nginx/sites-available/speedwholes

server {
    listen 443 ssl;
    server_name admin.example.com;

    ssl_certificate /etc/letsencrypt/live/admin.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/admin.example.com/privkey.pem;

    root /var/www/speedwholes/dist;
    index index.html;

    # SPA fallback
    location / {
        try_files $uri $uri/ /index.html;
    }

    # Cache static assets
    location ~* \.(js|css|png|jpg|jpeg|gif|ico|svg)$ {
        expires 1y;
        add_header Cache-Control "public, immutable";
    }
}
```

Enable the site:
```bash
sudo ln -s /etc/nginx/sites-available/speedwholes /etc/nginx/sites-enabled/
sudo nginx -t && sudo systemctl reload nginx
```

## Health Checks

```bash
# Backend health
curl http://localhost:3000/health

# With HTTPS
curl https://api.yourdomain.com/health
```

Expected response: `200 OK`

## Monitoring

### Log Output

The backend logs to stdout/stderr. Configure your process manager to capture logs.

Log levels via `RUST_LOG` environment variable:
```bash
RUST_LOG=info speedwholes     # Default
RUST_LOG=debug speedwholes    # Verbose
RUST_LOG=error speedwholes    # Errors only
```

### Statistics Endpoint

The admin API provides runtime statistics:

```bash
curl -sk -H "Authorization: Bearer ${ADMIN_TOKEN}" \
  https://images.example.com/config007/stats
```

Returns a `StatsSnapshot`:
```json
{
  "image_requests": 0,
  "cache_hits": 0,
  "cache_misses": 0,
  "source_cache_hits": 0,
  "source_cache_misses": 0,
  "signature_cache_hits": 0,
  "signature_cache_misses": 0,
  "last_subdomain": null
}
```

### Disk Usage

Monitor cache directory size:
```bash
du -sh /var/cache/speedwholes
```

## Troubleshooting

### Port Already in Use

```bash
sudo lsof -i :3000
# Kill the process using the port, or use a different port
```

### Permission Denied

Check file ownership:
```bash
ls -la /etc/speedwholes/
ls -la /var/cache/speedwholes/
```

Ensure speedwholes user owns these directories.

### TLS Certificate Errors

1. Check certificate paths exist and are readable:
   ```bash
   sudo -u speedwholes cat /etc/letsencrypt/live/api.yourdomain.com/fullchain.pem
   ```

2. Verify certificate validity:
   ```bash
   openssl x509 -in /etc/letsencrypt/live/api.yourdomain.com/fullchain.pem -text -noout
   ```

### Cache Issues

Clear the cache if needed:
```bash
sudo rm -rf /var/cache/speedwholes/*
```

## Security Checklist

- [ ] Use a strong, randomly generated `admin_token`
- [ ] Configure `allowed_sources` instead of `allow_all_sources: true`
- [ ] Use signing keys for URL tampering prevention
- [ ] Use HTTPS in production
- [ ] Restrict config file permissions (`chmod 600`)
- [ ] Run as non-root user
- [ ] Keep certificates up to date (auto-renewal)
