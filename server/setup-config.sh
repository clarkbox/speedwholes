#!/bin/bash
set -e

# Speedwholes Configuration Setup Script
# Interactive helper to configure speedwholes.config.json and www/src/config.json

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Configuration - detect installed vs development location
if [ -n "$CONFIG_DIR" ]; then
    # Explicit override via environment
    SERVER_CONFIG="${CONFIG_DIR}/speedwholes.config.json"
    WWW_CONFIG="${CONFIG_DIR}/www/config.json"
    EXAMPLE_CONFIG="${CONFIG_DIR}/examples/speedwholes.config.example.json"
    SERVICE_FILE="${CONFIG_DIR}/examples/speedwholes.service"
    SSL_SCRIPT="${CONFIG_DIR}/scripts/setup-ssl.sh"
elif [ -f "/etc/speedwholes/speedwholes.config.json" ]; then
    # Installed location
    CONFIG_DIR="/etc/speedwholes"
    SERVER_CONFIG="/etc/speedwholes/speedwholes.config.json"
    WWW_CONFIG="/etc/speedwholes/www/config.json"
    EXAMPLE_CONFIG="/etc/speedwholes/examples/speedwholes.config.example.json"
    SERVICE_FILE="/etc/speedwholes/examples/speedwholes.service"
    SSL_SCRIPT="/etc/speedwholes/scripts/setup-ssl.sh"
else
    # Development fallback - script directory
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    CONFIG_DIR="$SCRIPT_DIR"
    SERVER_CONFIG="${SCRIPT_DIR}/speedwholes.config.json"
    WWW_CONFIG="${SCRIPT_DIR}/../www/public/config.json"
    EXAMPLE_CONFIG="${SCRIPT_DIR}/speedwholes.config.example.json"
    SERVICE_FILE="${SCRIPT_DIR}/../systemd-examples/speedwholes.service"
    SSL_SCRIPT="${SCRIPT_DIR}/setup-ssl.sh"
fi

print_banner() {
    echo -e "${CYAN}"
    echo "╔════════════════════════════════════════════╗"
    echo "║     Speedwholes Configuration Setup        ║"
    echo "╚════════════════════════════════════════════╝"
    echo -e "${NC}"
}

print_success() {
    echo -e "${GREEN}✓ $1${NC}"
}

print_warning() {
    echo -e "${YELLOW}⚠ $1${NC}"
}

print_error() {
    echo -e "${RED}✗ $1${NC}"
}

print_info() {
    echo -e "${BLUE}ℹ $1${NC}"
}

# Check for required tools
check_dependencies() {
    local missing=()
    
    if ! command -v jq &> /dev/null; then
        missing+=("jq")
    fi
    
    if [ ${#missing[@]} -ne 0 ]; then
        print_error "Missing required tools: ${missing[*]}"
        echo ""
        echo "Install them with:"
        echo "  Ubuntu/Debian: sudo apt install ${missing[*]}"
        echo "  RHEL/CentOS:   sudo yum install ${missing[*]}"
        echo "  macOS:         brew install ${missing[*]}"
        exit 1
    fi
}

# Generate a random admin token
generate_token() {
    if command -v openssl &> /dev/null; then
        openssl rand -hex 32
    else
        # Fallback using /dev/urandom
        head -c 32 /dev/urandom | xxd -p | tr -d '\n'
    fi
}

# Read current value from config if it exists
get_current_value() {
    local key="$1"
    local default="$2"
    
    if [ -f "$SERVER_CONFIG" ]; then
        local value
        value=$(jq -r ".$key // empty" "$SERVER_CONFIG" 2>/dev/null)
        if [ -n "$value" ] && [ "$value" != "null" ]; then
            echo "$value"
            return
        fi
    fi
    echo "$default"
}

# Prompt user for input with default value
prompt_value() {
    local prompt="$1"
    local default="$2"
    local value
    
    if [ -n "$default" ]; then
        echo -en "${BLUE}$prompt${NC} [${GREEN}$default${NC}]: " >&2
    else
        echo -en "${BLUE}$prompt${NC}: " >&2
    fi
    
    read -r value
    if [ -z "$value" ]; then
        echo "$default"
    else
        echo "$value"
    fi
}

# Prompt for yes/no
prompt_yn() {
    local prompt="$1"
    local default="$2"
    local response
    
    if [ "$default" = "y" ]; then
        echo -en "${BLUE}$prompt${NC} [${GREEN}Y${NC}/n]: " >&2
    else
        echo -en "${BLUE}$prompt${NC} [y/${GREEN}N${NC}]: " >&2
    fi
    
    read -r response
    response=${response:-$default}
    [[ "$response" =~ ^[Yy] ]]
}

# Try to extract hostname from existing API URL
get_hostname_from_url() {
    local url="$1"
    echo "$url" | sed -E 's|^https?://||' | sed -E 's|:[0-9]+.*||' | sed -E 's|/.*||'
}

# Main setup function
main() {
    print_banner
    check_dependencies
    
    echo "This script will help you configure Speedwholes."
    echo ""
    
    # Check if config already exists
    if [ -f "$SERVER_CONFIG" ]; then
        print_info "Existing configuration found: $SERVER_CONFIG"
        echo "Current values will be shown as defaults."
        echo ""
    fi
    
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
    
    # Cache path
    current_cache=$(get_current_value "cache_path" "/var/cache/speedwholes")
    CACHE_PATH=$(prompt_value "Cache directory path" "$current_cache")
    
    # Server hostname
    echo ""
    print_info "The external hostname or IP address where this server is accessible"
    print_info "Used by the admin UI to connect to the API"
    
    # Try to get current hostname from www config API URL
    current_hostname=""
    if [ -f "$WWW_CONFIG" ]; then
        current_api=$(jq -r '.apiUrl // empty' "$WWW_CONFIG" 2>/dev/null)
        if [ -n "$current_api" ]; then
            current_hostname=$(get_hostname_from_url "$current_api")
        fi
    fi
    if [ -z "$current_hostname" ]; then
        current_hostname="localhost"
    fi
    
    SERVER_HOSTNAME=$(prompt_value "Server hostname" "$current_hostname")
    
    # Server port (the speedwholes backend)
    echo ""
    print_info "Port for the Speedwholes image server (HTTPS)"
    current_port=$(get_current_value "port" "443")
    SERVER_PORT=$(prompt_value "Server port" "$current_port")
    
    # Web port (nginx serving the admin UI)
    echo ""
    print_info "Port where the admin web UI will be served (via nginx)"
    print_info "This is used for CORS - allowing the UI to call the API"
    WEB_PORT=$(prompt_value "Web UI port" "8080")
    
    # Source allowlist. allow_all_sources is written as false.
    echo ""
    print_info "Sources are denied unless they match an allowlist pattern."
    print_info "Enter comma-separated patterns (for example images.example.com, cdn.example.com/*)."
    print_info "An empty list denies every source for tenants that do not set their own list."
    ALLOWED_SOURCES_RAW=$(prompt_value "Allowed source patterns" "images.example.com")
    ALLOWED_SOURCES_JSON=$(printf '%s' "$ALLOWED_SOURCES_RAW" | jq -R 'split(",") | map(gsub("^\\s+|\\s+$"; "")) | map(select(length > 0))')

    # Admin token
    current_token=$(get_current_value "admin_token" "")
    echo ""
    print_info "Admin token authenticates requests to the admin API"
    if [ -n "$current_token" ] && [ "$current_token" != "CHANGE-ME-generate-with-openssl-rand-hex-32" ]; then
        print_info "Current token: ${current_token:0:8}... (truncated)"
        if prompt_yn "Generate new admin token?" "n"; then
            ADMIN_TOKEN=$(generate_token)
            print_success "Generated new token"
        else
            ADMIN_TOKEN="$current_token"
        fi
    else
        if prompt_yn "Generate random admin token?" "y"; then
            ADMIN_TOKEN=$(generate_token)
            print_success "Generated token: ${ADMIN_TOKEN:0:16}..."
        else
            ADMIN_TOKEN=$(prompt_value "Enter admin token" "")
        fi
    fi
    
    # Derive computed values
    API_URL="https://${SERVER_HOSTNAME}:${SERVER_PORT}"
    
    echo ""
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${CYAN}  Summary${NC}"
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
    echo "  Cache path:     $CACHE_PATH"
    echo "  Server:         https://${SERVER_HOSTNAME}:${SERVER_PORT}"
    echo "  Web UI:         http(s)://${SERVER_HOSTNAME}:${WEB_PORT}"
    echo "  Admin token:    ${ADMIN_TOKEN:0:16}..."
    echo "  Allow all:      false"
    echo "  Allowed sources: $ALLOWED_SOURCES_JSON"
    echo "  API URL:        $API_URL"
    echo ""
    
    if ! prompt_yn "Apply these settings?" "y"; then
        print_warning "Configuration cancelled"
        exit 0
    fi
    
    echo ""
    
    # Create cache directory if needed
    if [ ! -d "$CACHE_PATH" ]; then
        print_info "Creating cache directory: $CACHE_PATH"
        mkdir -p "$CACHE_PATH"
        if id -u speedwholes &>/dev/null; then
            chown speedwholes:speedwholes "$CACHE_PATH"
        fi
        print_success "Created cache directory"
    fi
    
    # Write server config
    if [ -f "$SERVER_CONFIG" ]; then
        cp "$SERVER_CONFIG" "${SERVER_CONFIG}.bak"
        print_success "Backed up existing config to ${SERVER_CONFIG}.bak"
        
        # Update existing config
        local temp_file
        temp_file=$(mktemp)
        jq --argjson port "$SERVER_PORT" \
           --arg cache "$CACHE_PATH" \
           --arg token "$ADMIN_TOKEN" \
           --argjson sources "$ALLOWED_SOURCES_JSON" \
           '.port = $port | .cache_path = $cache | .admin_token = $token | .allow_all_sources = false | .allowed_sources = $sources | del(.cors_origin)' \
           "$SERVER_CONFIG" > "$temp_file"
        mv "$temp_file" "$SERVER_CONFIG"
    else
        # Create new config from example or scratch
        if [ -f "$EXAMPLE_CONFIG" ]; then
            cp "$EXAMPLE_CONFIG" "$SERVER_CONFIG"
            local temp_file
            temp_file=$(mktemp)
            jq --argjson port "$SERVER_PORT" \
               --arg cache "$CACHE_PATH" \
               --arg token "$ADMIN_TOKEN" \
               --argjson sources "$ALLOWED_SOURCES_JSON" \
               '.port = $port | .cache_path = $cache | .admin_token = $token | .allow_all_sources = false | .allowed_sources = $sources | del(.cors_origin)' \
               "$SERVER_CONFIG" > "$temp_file"
            mv "$temp_file" "$SERVER_CONFIG"
        else
            # Create minimal config
            cat > "$SERVER_CONFIG" << EOF
{
  "port": $SERVER_PORT,
  "cache_path": "$CACHE_PATH",
  "max_width": 4096,
  "max_height": 4096,
  "timeout": 30,
  "max_source_pixels": 100000000,
  "allowed_sources": $ALLOWED_SOURCES_JSON,
  "allow_all_sources": false,
  "admin_token": "$ADMIN_TOKEN"
}
EOF
        fi
    fi
    print_success "Wrote server config: $SERVER_CONFIG"
    print_info "  → port: $SERVER_PORT, cache: $CACHE_PATH"
    
    # Write frontend config
    if [ -f "$WWW_CONFIG" ]; then
        cp "$WWW_CONFIG" "${WWW_CONFIG}.bak"
        print_success "Backed up existing frontend config"
    fi
    
    cat > "$WWW_CONFIG" << EOF
{
  "apiUrl": "$API_URL"
}
EOF
    print_success "Wrote frontend config: $WWW_CONFIG"
    print_info "  → apiUrl: $API_URL"
    
    echo ""
    print_success "Configuration complete!"
    
    # Offer to install systemd service
    echo ""
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${CYAN}  Systemd Service${NC}"
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
    
    if [ -f "/etc/systemd/system/speedwholes.service" ]; then
        print_info "Systemd service already installed"
        if prompt_yn "Restart the service now?" "y"; then
            systemctl restart speedwholes
            print_success "Service restarted"
        fi
    elif [ -f "$SERVICE_FILE" ]; then
        print_info "Install systemd service to run Speedwholes automatically on boot"
        if prompt_yn "Install and enable systemd service?" "y"; then
            cp "$SERVICE_FILE" /etc/systemd/system/speedwholes.service
            systemctl daemon-reload
            systemctl enable speedwholes
            print_success "Service installed and enabled"
            
            if prompt_yn "Start the service now?" "y"; then
                systemctl start speedwholes
                print_success "Service started"
            fi
        else
            echo ""
            print_info "You can install the service later with:"
            echo "  sudo cp $SERVICE_FILE /etc/systemd/system/"
            echo "  sudo systemctl daemon-reload"
            echo "  sudo systemctl enable --now speedwholes"
        fi
    else
        print_warning "Service file not found at: $SERVICE_FILE"
    fi
    
    # Offer to run SSL setup
    echo ""
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${CYAN}  SSL Setup${NC}"
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
    print_info "SSL/TLS is required for HTTPS"
    
    if [ -f "$SSL_SCRIPT" ]; then
        if prompt_yn "Would you like to configure SSL now?" "y"; then
            echo ""
            exec "$SSL_SCRIPT"
        else
            echo ""
            print_info "You can run SSL setup later with:"
            echo "  $SSL_SCRIPT"
        fi
    else
        print_warning "SSL setup script not found at: $SSL_SCRIPT"
    fi
    
    echo ""
    print_success "Setup complete!"
}

main "$@"
