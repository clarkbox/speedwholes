#!/bin/bash
set -e

# Speedwholes Deployment Script - Deploys both server and www
# Usage: ./deploy.sh [--keyfile key.pem] [--service server|www|all] <host> [user]
#
# host is required. user defaults to $USER. A private key is used only when
# --keyfile is given (ssh-agent and ~/.ssh/config are used otherwise).

usage() {
    echo "Usage: ./deploy.sh [--keyfile key.pem] [--service server|www|all] <host> [user]"
    echo ""
    echo "  host              Required. SSH host to deploy to."
    echo "  user              SSH user (default: \$USER)."
    echo "  --keyfile FILE    SSH private key. Omit to use the default ssh agent/config."
    echo "  --service NAME    server, www, or all (default: all)."
    exit 2
}

# Default values
DEFAULT_SERVICE="all"

# Service deployment paths
SERVER_BINARY="server/target/release/speedwholes"
SERVER_DEST="/usr/local/bin/speedwholes"

# Frontend deployment paths
WWW_DIR="www"
WWW_DIST="www/dist"
WWW_DEST="/etc/speedwholes/www"

# Parse arguments
KEYFILE=""
SERVICE="$DEFAULT_SERVICE"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --keyfile)
            if [ "$#" -lt 2 ]; then
                usage
            fi
            KEYFILE="$2"
            shift 2
            ;;
        --service)
            if [ "$#" -lt 2 ]; then
                usage
            fi
            SERVICE="$2"
            shift 2
            ;;
        --help|-h)
            usage
            ;;
        -*)
            echo "Unknown option: $1" >&2
            usage
            ;;
        *)
            break
            ;;
    esac
done

if [ "$#" -lt 1 ] || [ -z "${1:-}" ]; then
    usage
fi

HOST="$1"
USER="${2:-$USER}"

# Build SSH options
SSH_OPTS=""
if [ -n "$KEYFILE" ]; then
    if [ ! -f "$KEYFILE" ]; then
        echo "Error: Key file not found: $KEYFILE"
        exit 1
    fi
    SSH_OPTS="-i $KEYFILE"
fi

echo "========================================"
echo "  Speedwholes Deployment Script"
echo "========================================"
echo ""
echo "Target:  ${USER}@${HOST}"
echo "Service: ${SERVICE}"
if [ -n "$KEYFILE" ]; then
    echo "Key:     $KEYFILE"
fi
echo ""

# Validate service option
if [ "$SERVICE" != "all" ] && [ "$SERVICE" != "server" ] && [ "$SERVICE" != "www" ]; then
    echo "Error: Invalid service '$SERVICE'. Must be 'server', 'www', or 'all'"
    exit 1
fi

# Check for required system dependencies (when building server)
check_server_deps() {
    echo "[CHECK] Verifying system dependencies for server build..."
    local missing=0

    # The turbojpeg crate can either link to a system libjpeg-turbo (via pkg-config)
    # or build libjpeg-turbo from source (via cmake + nasm for SIMD).
    local has_system_lib=false
    local has_build_tools=false

    # Option A: system libjpeg-turbo
    if pkg-config --exists libturbojpeg 2>/dev/null || pkg-config --exists libjpeg 2>/dev/null; then
        has_system_lib=true
        echo "        libjpeg-turbo: found (system library via pkg-config)"
    elif [ -f /usr/include/turbojpeg.h ] || [ -f /usr/local/include/turbojpeg.h ]; then
        has_system_lib=true
        echo "        libjpeg-turbo: found (system header detected)"
    fi

    # Option B: cmake + nasm (build from source)
    if command -v cmake &>/dev/null && command -v nasm &>/dev/null; then
        has_build_tools=true
        echo "        cmake: found ($(cmake --version | head -1))"
        echo "        nasm: found ($(nasm -v 2>&1 | head -1))"
    fi

    if [ "$has_system_lib" = false ] && [ "$has_build_tools" = false ]; then
        echo ""
        echo "Error: No libjpeg-turbo build path available."
        echo "       The turbojpeg crate needs one of the following:"
        echo ""
        echo "       Option A - Install system library (faster compile):"
        echo "         RHEL/CentOS: dnf install libjpeg-turbo-devel"
        echo "         Ubuntu/Debian: apt install libturbojpeg0-dev"
        echo ""
        echo "       Option B - Install build tools (builds from source with SIMD):"
        echo "         RHEL/CentOS: dnf install cmake nasm gcc"
        echo "         Ubuntu/Debian: apt install cmake nasm build-essential"
        missing=1
    fi

    if [ "$missing" -ne 0 ]; then
        echo ""
        echo "Aborting deployment due to missing system dependencies."
        exit 1
    fi
    echo ""
}

# Function to build a service
build_service() {
    local service_name=$1
    local service_dir=$2
    local binary_path=$3
    
    echo "[BUILD] Building ${service_name}..."
    cd "$service_dir"
    cargo build --release
    cd ..
    
    if [ ! -f "$binary_path" ]; then
        echo "Error: Build failed - binary not found at $binary_path"
        exit 1
    fi
    
    local size=$(du -h "$binary_path" | cut -f1)
    echo "        Built: $binary_path ($size)"
}

# Function to deploy a service
deploy_service() {
    local service_name=$1
    local binary_path=$2
    local dest_path=$3
    local systemd_service=$4
    
    echo ""
    echo "[DEPLOY] Deploying ${service_name} to ${USER}@${HOST}:${dest_path}..."
    
    if [ -z "$KEYFILE" ]; then
        echo "         (You may be prompted for password)"
    fi
    
    scp $SSH_OPTS "$binary_path" "${USER}@${HOST}:${dest_path}"
    
    echo "[RESTART] Restarting ${systemd_service} service..."
    ssh $SSH_OPTS "${USER}@${HOST}" "systemctl restart ${systemd_service} 2>/dev/null || echo 'Note: No systemd service found for ${systemd_service}'"
}

# Function to build frontend
build_www() {
    echo "[BUILD] Building Frontend..."
    cd "$WWW_DIR"
    npm install
    npm run build
    cd ..
    
    if [ ! -d "$WWW_DIST" ]; then
        echo "Error: Build failed - dist directory not found at $WWW_DIST"
        exit 1
    fi
    echo "        Built: $WWW_DIST"
}

# Function to deploy frontend
deploy_www() {
    echo ""
    echo "[DEPLOY] Deploying Frontend to ${USER}@${HOST}:${WWW_DEST}..."
    
    if [ -z "$KEYFILE" ]; then
        echo "         (You may be prompted for password)"
    fi
    
    # Sync the dist folder, excluding config files to preserve server-specific settings
    rsync -avz --delete \
        --exclude='config.json' \
        --exclude='*.config.json' \
        -e "ssh $SSH_OPTS" "$WWW_DIST/" "${USER}@${HOST}:${WWW_DEST}/"
    
    # Reload nginx if present
    ssh $SSH_OPTS "${USER}@${HOST}" "systemctl reload nginx 2>/dev/null || true"
}

# Build and deploy server
if [ "$SERVICE" = "all" ] || [ "$SERVICE" = "server" ]; then
    echo "========================================"
    echo "  Backend Server"
    echo "========================================"
    check_server_deps
    build_service "Backend Server" "server" "$SERVER_BINARY"
    deploy_service "Backend Server" "$SERVER_BINARY" "$SERVER_DEST" "speedwholes"
fi

# Build and deploy www
if [ "$SERVICE" = "all" ] || [ "$SERVICE" = "www" ]; then
    echo "========================================"
    echo "  Frontend (www)"
    echo "========================================"
    build_www
    deploy_www
fi

echo ""
echo "========================================"
echo "  Deployment Complete!"
echo "========================================"
echo ""

# Show verification commands
echo "Verify backend:"
echo "  ssh $SSH_OPTS ${USER}@${HOST} 'curl -s http://localhost:3000/health'"

echo ""
echo "View logs:"
echo "  ssh $SSH_OPTS ${USER}@${HOST} 'journalctl -u speedwholes -f'"

echo ""
echo "Verify frontend:"
echo "  Open https://admin.yourdomain.com in browser"

echo ""
