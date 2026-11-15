# Instance Locking & Multi-Instance Support

## Overview

Speedwholes now supports running multiple isolated instances simultaneously, each with its own configuration directory and port. Instance locking prevents accidentally running duplicate instances with the same configuration.

## Key Features

### 1. Instance Locking
- **PID-based locking**: Each config directory can only have one running instance
- **Automatic stale lock cleanup**: If a process dies, the lock is automatically removed on next startup
- **Clear error messages**: Shows the PID of the already-running instance if duplicate detected

### 2. Configurable Config Directory
- **New `--config-dir` flag**: Specify where config files and PID lock are stored
- **Default**: Current working directory (`.`)

### 3. Safe Cache Sharing
- Multiple instances can safely share the same cache directory
- Uses atomic write operations (write-to-temp, then rename)
- Detects and logs when cache is shared with other instances

## Files in Config Directory

Each config directory contains:
- `speedwholes.config.json` - Runtime configuration
- `subdomainConfig/` - Directory containing subdomain config files
- `speedwholes.pid` - PID lock file (auto-managed)

## Usage Examples

### Single Instance (Original Behavior)
```bash
speedwholes --port 3000
```

### Multiple Isolated Instances
```bash
# Production instance
speedwholes \
  --port 3000 \
  --config-dir /var/lib/speedwholes/prod \
  --cache-path /var/cache/speedwholes

# Staging instance (sharing cache)
speedwholes \
  --port 3001 \
  --config-dir /var/lib/speedwholes/staging \
  --cache-path /var/cache/speedwholes

# Development instance (separate cache)
speedwholes \
  --port 3002 \
  --config-dir ./dev-config \
  --cache-path ./dev-cache
```

## Error Handling

### Duplicate Instance Detected
```
ERROR Another instance is already running with PID 12345 (lock file: "/var/lib/speedwholes/prod/speedwholes.pid")
```
**Resolution**: 
- Stop the existing instance, or
- Use a different `--config-dir`

### Stale Lock Cleanup
```
WARN Removing stale lock file from dead process 12345 (lock file: "/var/lib/speedwholes/prod/speedwholes.pid")
INFO Acquired instance lock (pid: 67890)
```
This happens automatically - no action needed.

## Implementation Details

### Lock Module (`src/lock.rs`)
- `InstanceLock::acquire()` - Acquires PID lock for config directory
- `InstanceLock::release()` - Releases lock (also called in Drop)
- `is_process_alive()` - Checks if PID is still running (Unix-specific)

### Process Check
- **Unix/Linux**: Uses `kill(pid, SIGCONT)` via nix crate
- Checks without sending actual signal
- Handles permission errors (EPERM = process exists but no permission)

### Automatic Cleanup
- Lock released on graceful shutdown (SIGTERM/SIGINT)
- Lock released on process termination (Drop trait)
- Lock released on panic (Drop trait runs before exit)

## Cache Safety

### Why Shared Cache is Safe
The cache implementation uses atomic operations:
1. Write data to temporary file (`{cache_key}.tmp`)
2. Sync file to disk
3. Atomically rename to final name (OS-level atomic operation)

This means:
- **No partial writes**: Readers always see complete files or nothing
- **No corruption**: Rename is atomic at filesystem level
- **Cache miss on conflict**: If two instances write same file, last writer wins
- **Eventual consistency**: Subsequent requests work correctly

### Cache Sharing Detection
On startup, speedwholes checks if the cache directory contains existing files and logs:
```
INFO Cache directory contains 1234 existing files - may be shared with other instances (this is safe)
```

## Testing

Run the lock module tests:
```bash
cargo test --bin speedwholes lock::
```

Tests verify:
- Lock acquisition works
- Duplicate prevention works
- Auto-release on Drop works
- Process alive detection works

## Migration Guide

### Existing Deployments
No changes needed! Default behavior is unchanged:
- Config dir defaults to current directory (`.`)
- PID lock created in same directory as before

### New Multi-Instance Setups
1. Create separate config directories for each instance
2. Start each instance with different `--port` and `--config-dir`
3. Optionally share `--cache-path` between instances for efficiency

### Docker/Kubernetes
```dockerfile
# Example Dockerfile
FROM rust:latest
WORKDIR /app
COPY . .
RUN cargo build --release

VOLUME ["/data/config", "/data/cache"]
CMD ["./target/release/speedwholes", "--config-dir", "/data/config", "--cache-path", "/data/cache", "--port", "3000"]
```

```yaml
# Example Kubernetes deployment with multiple replicas
apiVersion: apps/v1
kind: Deployment
metadata:
  name: speedwholes
spec:
  replicas: 3
  template:
    spec:
      containers:
      - name: speedwholes
        image: speedwholes:latest
        args:
        - "--config-dir"
        - "/data/config/$(POD_NAME)"  # Unique per pod
        - "--cache-path"
        - "/data/cache"  # Shared via PVC
        - "--port"
        - "3000"
        volumeMounts:
        - name: config
          mountPath: /data/config
        - name: cache
          mountPath: /data/cache
      volumes:
      - name: config
        persistentVolumeClaim:
          claimName: speedwholes-config
      - name: cache
        persistentVolumeClaim:
          claimName: speedwholes-cache
```

## Troubleshooting

### Port Already in Use
```
ERROR Port 3000 is already in use. Is another instance running?
```
**Solution**: Use different `--port` for each instance

### Permission Denied on Lock File
```
ERROR I/O error: Permission denied (os error 13)
```
**Solution**: Ensure write permissions to config directory

### Lock File Won't Release
If the lock file persists after killing a process:
```bash
# Check if process is still running
ps aux | grep speedwholes

# Manually remove stale lock if process is dead
rm /path/to/config-dir/speedwholes.pid
```

## Benefits

1. **Development**: Run multiple test instances without conflicts
2. **Staging**: Run staging alongside production with different configs
3. **A/B Testing**: Run multiple configs on different ports
4. **Resource Efficiency**: Share cache between instances to save disk space
5. **Safety**: Prevent accidental duplicate instances from corrupting config
