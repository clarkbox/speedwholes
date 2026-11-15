# Systemd Service Files

Example systemd service files for running Speedwholes on Linux servers.

For general production deployment (directories, configuration, TLS, reverse proxy), see **[docs/deployment.md](../docs/deployment.md)**.

## Prerequisites

Complete the setup steps in [docs/deployment.md](../docs/deployment.md) first:
- Create user and directories
- Install binaries
- Create configuration file

## Install Service Files

```bash
# Copy service files
sudo cp systemd-examples/speedwholes.service /etc/systemd/system/

# Reload systemd
sudo systemctl daemon-reload
```

## Manage Services

```bash
# Enable to start on boot
sudo systemctl enable speedwholes

# Start/stop/restart
sudo systemctl start speedwholes
sudo systemctl stop speedwholes
sudo systemctl restart speedwholes

# Check status
sudo systemctl status speedwholes
```

## View Logs

```bash
# Follow logs in real-time
sudo journalctl -u speedwholes -f

# Last 100 lines
sudo journalctl -u speedwholes -n 100

# Since last boot
sudo journalctl -u speedwholes -b

# Filter by time
sudo journalctl -u speedwholes --since "1 hour ago"
```

## Customize Service

To override settings without editing the original service file:

```bash
sudo systemctl edit speedwholes
```

This opens an editor for `/etc/systemd/system/speedwholes.service.d/override.conf`.

### Example: Change Port

```ini
[Service]
ExecStart=
ExecStart=/usr/local/bin/speedwholes --port 3001 --config-dir /etc/speedwholes --cache-path /var/cache/speedwholes
```

### Example: Enable HTTPS on Port 443

```ini
[Service]
# Allow binding to privileged port without root
AmbientCapabilities=CAP_NET_BIND_SERVICE

# Grant read access to Let's Encrypt certificates
ReadOnlyPaths=/etc/letsencrypt

# Alternative to an ACL: expose the private key only inside the service
# LoadCredential=tls.key:/etc/letsencrypt/live/example.com/privkey.pem
# Then set tls_key to ${CREDENTIALS_DIRECTORY}/tls.key
```

### Example: Set Log Level

```ini
[Service]
Environment="RUST_LOG=debug"
```

After editing, apply changes:

```bash
sudo systemctl daemon-reload
sudo systemctl restart speedwholes
```

## Certificate Renewal Hook

If using Let's Encrypt with native TLS, set up auto-restart on renewal:

```bash
sudo tee /etc/letsencrypt/renewal-hooks/post/speedwholes.sh << 'EOF'
#!/bin/bash
systemctl restart speedwholes
EOF
sudo chmod +x /etc/letsencrypt/renewal-hooks/post/speedwholes.sh
```

## Troubleshooting

### Service Won't Start

Check logs for errors:
```bash
sudo journalctl -u speedwholes -n 50 --no-pager
```

Verify the binary runs manually:
```bash
sudo -u speedwholes /usr/local/bin/speedwholes --config-dir /etc/speedwholes --cache-path /var/cache/speedwholes
```

### Check Service Configuration

```bash
# Show effective service configuration
sudo systemctl cat speedwholes

# Verify service file syntax
sudo systemd-analyze verify speedwholes.service
```
