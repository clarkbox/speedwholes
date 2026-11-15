#!/bin/bash
set -e

# Speedwholes SSL Setup Script
# Configures TLS certificates for HTTPS (self-signed or Let's Encrypt)

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
    CONFIG_FILE="${CONFIG_DIR}/speedwholes.config.json"
    DEFAULT_CERT_DIR="${CONFIG_DIR}/certs"
elif [ -f "/etc/speedwholes/speedwholes.config.json" ]; then
    # Installed location
    CONFIG_DIR="/etc/speedwholes"
    CONFIG_FILE="/etc/speedwholes/speedwholes.config.json"
    DEFAULT_CERT_DIR="/etc/speedwholes/certs"
else
    # Development fallback - script directory
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    CONFIG_DIR="$SCRIPT_DIR"
    CONFIG_FILE="${SCRIPT_DIR}/speedwholes.config.json"
    DEFAULT_CERT_DIR="${SCRIPT_DIR}/certs"
fi

print_banner() {
    echo -e "${CYAN}"
    echo "╔════════════════════════════════════════════╗"
    echo "║       Speedwholes SSL Setup Script         ║"
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

# Check if config file exists
check_config() {
    if [ ! -f "$CONFIG_FILE" ]; then
        print_error "Configuration file not found: $CONFIG_FILE"
        echo "Please ensure speedwholes.config.json exists before running this script."
        exit 1
    fi
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

# Update config file with certificate paths
update_config() {
    local cert_path="$1"
    local key_path="$2"
    local port="$3"
    
    print_info "Updating configuration file..."
    
    # Create backup
    cp "$CONFIG_FILE" "${CONFIG_FILE}.bak"
    print_success "Created backup: ${CONFIG_FILE}.bak"
    
    # Update config with jq
    local temp_file=$(mktemp)
    jq --arg cert "$cert_path" --arg key "$key_path" \
       '.tls_cert = $cert | .tls_key = $key' \
       "$CONFIG_FILE" > "$temp_file"
    
    # Optionally update port
    if [ -n "$port" ]; then
        jq --argjson port "$port" '.port = $port' "$temp_file" > "${temp_file}.2"
        mv "${temp_file}.2" "$temp_file"
    fi
    
    mv "$temp_file" "$CONFIG_FILE"
    print_success "Updated $CONFIG_FILE"
}

# Setup self-signed certificate
setup_self_signed() {
    echo ""
    echo -e "${CYAN}═══════════════════════════════════════════════${NC}"
    echo -e "${CYAN}  Self-Signed Certificate Setup${NC}"
    echo -e "${CYAN}═══════════════════════════════════════════════${NC}"
    echo ""
    
    # Check for openssl
    if ! command -v openssl &> /dev/null; then
        print_error "OpenSSL is required but not installed."
        echo "Install it with: sudo apt install openssl"
        exit 1
    fi
    
    # Get domain/hostname
    read -p "Enter domain or hostname [localhost]: " domain
    domain=${domain:-localhost}
    
    # Get additional IPs/domains for SAN
    echo ""
    echo "Enter additional Subject Alternative Names (SANs)."
    echo "These can be IP addresses or domain names."
    read -p "Additional SANs (comma-separated, or press Enter to skip): " extra_sans
    
    # Get certificate directory
    read -p "Certificate directory [${DEFAULT_CERT_DIR}]: " cert_dir
    cert_dir=${cert_dir:-$DEFAULT_CERT_DIR}
    
    # Get validity period
    read -p "Certificate validity in days [365]: " validity_days
    validity_days=${validity_days:-365}
    
    # Create certificate directory
    mkdir -p "$cert_dir"
    
    # Build SAN string
    local san_string="DNS:${domain}"
    if [[ "$domain" == "localhost" ]] || [[ "$domain" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        san_string="${san_string},IP:127.0.0.1"
    fi
    
    if [ -n "$extra_sans" ]; then
        IFS=',' read -ra SANS <<< "$extra_sans"
        for san in "${SANS[@]}"; do
            san=$(echo "$san" | xargs)  # Trim whitespace
            if [[ "$san" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
                san_string="${san_string},IP:${san}"
            else
                san_string="${san_string},DNS:${san}"
            fi
        done
    fi
    
    local cert_path="${cert_dir}/cert.pem"
    local key_path="${cert_dir}/key.pem"
    
    echo ""
    print_info "Generating self-signed certificate..."
    echo "  Domain: $domain"
    echo "  SANs: $san_string"
    echo "  Validity: $validity_days days"
    echo ""
    
    # Generate certificate
    openssl req -x509 -newkey rsa:4096 \
        -keyout "$key_path" \
        -out "$cert_path" \
        -days "$validity_days" \
        -nodes \
        -subj "/CN=${domain}" \
        -addext "subjectAltName=${san_string}" \
        2>/dev/null
    
    # Set secure permissions
    chmod 600 "$key_path"
    chmod 644 "$cert_path"
    
    print_success "Certificate generated!"
    echo "  Certificate: $cert_path"
    echo "  Private Key: $key_path"
    
    # Update config (don't change port - that's set in main config)
    update_config "$cert_path" "$key_path" ""
    
    echo ""
    echo -e "${GREEN}═══════════════════════════════════════════════${NC}"
    echo -e "${GREEN}  Self-Signed SSL Setup Complete!${NC}"
    echo -e "${GREEN}═══════════════════════════════════════════════${NC}"
    echo ""
    print_warning "Browsers will show a security warning for self-signed certificates."
    echo "For locally-trusted certificates, consider using mkcert:"
    echo "  https://github.com/FiloSottile/mkcert"
    echo ""
    echo "Restart speedwholes to apply changes:"
    echo "  systemctl restart speedwholes"
    echo ""
}

# Normalize a domain for Let's Encrypt directory path
# Wildcard certs (*.example.com) are stored in directory "example.com" (without asterisk)
normalize_letsencrypt_domain() {
    local domain="$1"
    # Remove leading "*." if present (wildcard cert)
    echo "$domain" | sed 's/^\*\.//'
}

# Setup Let's Encrypt certificate
setup_letsencrypt() {
    echo ""
    echo -e "${CYAN}═══════════════════════════════════════════════${NC}"
    echo -e "${CYAN}  Let's Encrypt Certificate Setup${NC}"
    echo -e "${CYAN}═══════════════════════════════════════════════${NC}"
    echo ""
    
    # Check for certbot
    if ! command -v certbot &> /dev/null; then
        print_error "Certbot is required but not installed."
        echo ""
        echo "Install certbot:"
        echo "  Ubuntu/Debian: sudo apt install certbot"
        echo "  RHEL/CentOS:   sudo yum install certbot"
        echo "  macOS:         brew install certbot"
        echo ""
        read -p "Would you like to continue anyway (for configuration only)? [y/N]: " continue_anyway
        if [[ ! "$continue_anyway" =~ ^[Yy]$ ]]; then
            exit 1
        fi
    fi
    
    # Check if running as root
    if [ "$EUID" -ne 0 ]; then
        print_warning "Let's Encrypt setup typically requires root privileges."
        echo "You may need to run this script with sudo for certbot commands."
        echo ""
    fi
    
    # Get domain
    echo "Enter your domain (e.g., api.example.com or *.example.com for wildcard)"
    read -p "Domain: " domain
    if [ -z "$domain" ]; then
        print_error "Domain is required for Let's Encrypt."
        exit 1
    fi
    
    # Normalize the domain for the certificate path
    # Wildcard certs like *.example.com are stored in /etc/letsencrypt/live/example.com/
    local cert_domain=$(normalize_letsencrypt_domain "$domain")
    
    # Get email
    read -p "Enter email for certificate notifications: " email
    if [ -z "$email" ]; then
        print_warning "Email is recommended for expiration notices."
        read -p "Continue without email? [y/N]: " continue_no_email
        if [[ ! "$continue_no_email" =~ ^[Yy]$ ]]; then
            exit 1
        fi
    fi
    
    # Choose verification method
    echo ""
    echo "Choose verification method:"
    echo "  1) Standalone (certbot runs temporary web server on port 80)"
    echo "  2) Webroot (use existing web server's document root)"
    echo "  3) DNS (manual DNS TXT record verification)"
    echo "  4) Skip certificate generation (just configure paths)"
    echo ""
    read -p "Select method [1-4]: " method
    
    # Use normalized domain for certificate paths (handles wildcards)
    local cert_path="/etc/letsencrypt/live/${cert_domain}/fullchain.pem"
    local key_path="/etc/letsencrypt/live/${cert_domain}/privkey.pem"
    
    if [[ "$domain" == \** ]]; then
        print_info "Wildcard certificate detected"
        print_info "Certificate will be stored in: /etc/letsencrypt/live/${cert_domain}/"
        echo ""
    fi
    
    case "$method" in
        1)
            # Standalone mode
            print_info "Using standalone mode..."
            print_warning "Ensure port 80 is available (stop any web servers)."
            echo ""
            read -p "Ready to proceed? [y/N]: " ready
            if [[ ! "$ready" =~ ^[Yy]$ ]]; then
                exit 0
            fi
            
            local certbot_cmd="certbot certonly --standalone -d ${domain}"
            if [ -n "$email" ]; then
                certbot_cmd="${certbot_cmd} --email ${email} --agree-tos --no-eff-email"
            else
                certbot_cmd="${certbot_cmd} --register-unsafely-without-email --agree-tos"
            fi
            
            echo ""
            print_info "Running: $certbot_cmd"
            echo ""
            
            if [ "$EUID" -eq 0 ]; then
                eval "$certbot_cmd"
            else
                sudo $certbot_cmd
            fi
            ;;
        2)
            # Webroot mode
            read -p "Enter webroot path (e.g., /var/www/html): " webroot
            if [ -z "$webroot" ]; then
                print_error "Webroot path is required."
                exit 1
            fi
            
            local certbot_cmd="certbot certonly --webroot -w ${webroot} -d ${domain}"
            if [ -n "$email" ]; then
                certbot_cmd="${certbot_cmd} --email ${email} --agree-tos --no-eff-email"
            else
                certbot_cmd="${certbot_cmd} --register-unsafely-without-email --agree-tos"
            fi
            
            echo ""
            print_info "Running: $certbot_cmd"
            echo ""
            
            if [ "$EUID" -eq 0 ]; then
                eval "$certbot_cmd"
            else
                sudo $certbot_cmd
            fi
            ;;
        3)
            # DNS mode
            local certbot_cmd="certbot certonly --manual --preferred-challenges dns -d ${domain}"
            if [ -n "$email" ]; then
                certbot_cmd="${certbot_cmd} --email ${email} --agree-tos --no-eff-email"
            else
                certbot_cmd="${certbot_cmd} --register-unsafely-without-email --agree-tos"
            fi
            
            echo ""
            print_info "Running: $certbot_cmd"
            print_warning "You will be prompted to create a DNS TXT record."
            echo ""
            
            if [ "$EUID" -eq 0 ]; then
                eval "$certbot_cmd"
            else
                sudo $certbot_cmd
            fi
            ;;
        4)
            # Skip generation, just configure
            print_info "Skipping certificate generation."
            echo ""
            echo "Assuming certificates will be at:"
            echo "  Certificate: $cert_path"
            echo "  Private Key: $key_path"
            ;;
        *)
            print_error "Invalid option."
            exit 1
            ;;
    esac
    
    # Update config (don't change port - that's set in main config)
    update_config "$cert_path" "$key_path" ""
    
    # Setup renewal hook
    echo ""
    read -p "Create auto-renewal hook for speedwholes restart? [Y/n]: " setup_hook
    if [[ ! "$setup_hook" =~ ^[Nn]$ ]]; then
        local hook_dir="/etc/letsencrypt/renewal-hooks/post"
        local hook_file="${hook_dir}/speedwholes.sh"
        
        if [ "$EUID" -eq 0 ]; then
            mkdir -p "$hook_dir"
            cat > "$hook_file" << 'EOF'
#!/bin/bash
# Restart speedwholes after certificate renewal
systemctl restart speedwholes 2>/dev/null || true
EOF
            chmod +x "$hook_file"
            print_success "Created renewal hook: $hook_file"
        else
            echo ""
            print_warning "Run these commands as root to create the renewal hook:"
            echo "  sudo mkdir -p $hook_dir"
            echo "  sudo tee $hook_file << 'EOF'"
            echo "#!/bin/bash"
            echo "systemctl restart speedwholes 2>/dev/null || true"
            echo "EOF"
            echo "  sudo chmod +x $hook_file"
        fi
    fi
    
    # Setup certificate permissions automatically
    echo ""
    print_info "Setting up certificate permissions..."
    
    if id -u speedwholes &>/dev/null; then
        # Check if setfacl is available (preferred method)
        if command -v setfacl &>/dev/null; then
            echo ""
            read -p "Grant speedwholes user read access to certificates using ACLs? [Y/n]: " setup_acls
            if [[ ! "$setup_acls" =~ ^[Nn]$ ]]; then
                if [ "$EUID" -eq 0 ]; then
                    # Set ACLs on live and archive directories
                    setfacl -R -m u:speedwholes:rX /etc/letsencrypt/live/ 2>/dev/null || true
                    setfacl -R -m u:speedwholes:rX /etc/letsencrypt/archive/ 2>/dev/null || true
                    # Also set default ACLs for future renewals
                    setfacl -R -d -m u:speedwholes:rX /etc/letsencrypt/live/ 2>/dev/null || true
                    setfacl -R -d -m u:speedwholes:rX /etc/letsencrypt/archive/ 2>/dev/null || true
                    print_success "ACLs configured for speedwholes user"
                else
                    print_warning "Run these commands as root to set up certificate permissions:"
                    echo "  sudo setfacl -R -m u:speedwholes:rX /etc/letsencrypt/live/"
                    echo "  sudo setfacl -R -m u:speedwholes:rX /etc/letsencrypt/archive/"
                    echo "  sudo setfacl -R -d -m u:speedwholes:rX /etc/letsencrypt/live/"
                    echo "  sudo setfacl -R -d -m u:speedwholes:rX /etc/letsencrypt/archive/"
                fi
            fi
        else
            # Fallback: set group permissions
            echo ""
            print_warning "setfacl not available. Using group permissions instead."
            echo "The speedwholes user needs read access to the certificates."
            echo ""
            read -p "Set group permissions for speedwholes? [Y/n]: " setup_perms
            if [[ ! "$setup_perms" =~ ^[Nn]$ ]]; then
                if [ "$EUID" -eq 0 ]; then
                    chmod 750 "/etc/letsencrypt/live/${cert_domain}" 2>/dev/null || true
                    chmod 750 "/etc/letsencrypt/archive/${cert_domain}" 2>/dev/null || true
                    chgrp speedwholes "/etc/letsencrypt/live/${cert_domain}" 2>/dev/null || true
                    chgrp speedwholes "/etc/letsencrypt/archive/${cert_domain}" 2>/dev/null || true
                    # Also need to allow traversal of parent directories
                    chmod o+x /etc/letsencrypt/live /etc/letsencrypt/archive 2>/dev/null || true
                    print_success "Group permissions configured"
                else
                    print_warning "Run these commands as root:"
                    echo "  sudo chmod 750 /etc/letsencrypt/live/${cert_domain}"
                    echo "  sudo chmod 750 /etc/letsencrypt/archive/${cert_domain}"
                    echo "  sudo chgrp speedwholes /etc/letsencrypt/live/${cert_domain}"
                    echo "  sudo chgrp speedwholes /etc/letsencrypt/archive/${cert_domain}"
                fi
            fi
        fi
    else
        print_info "speedwholes user not found - skipping permission setup"
        print_info "If running as a different user, ensure that user can read the certificates"
    fi
    echo ""
    
    echo ""
    echo -e "${GREEN}═══════════════════════════════════════════════${NC}"
    echo -e "${GREEN}  Let's Encrypt SSL Setup Complete!${NC}"
    echo -e "${GREEN}═══════════════════════════════════════════════${NC}"
    echo ""
    echo "Restart speedwholes to apply changes:"
    echo "  systemctl restart speedwholes"
    echo ""
    echo "Test certificate renewal:"
    echo "  sudo certbot renew --dry-run"
    echo ""
}

# List existing Let's Encrypt certificates
list_letsencrypt_certs() {
    if [ -d /etc/letsencrypt/live ]; then
        local certs=()
        for dir in /etc/letsencrypt/live/*/; do
            if [ -f "${dir}fullchain.pem" ]; then
                local name=$(basename "$dir")
                # Skip the README
                if [ "$name" != "README" ]; then
                    certs+=("$name")
                fi
            fi
        done
        
        if [ ${#certs[@]} -gt 0 ]; then
            echo ""
            echo -e "${CYAN}Existing Let's Encrypt certificates found:${NC}"
            local i=1
            for cert in "${certs[@]}"; do
                local cert_file="/etc/letsencrypt/live/${cert}/fullchain.pem"
                local expiry=$(openssl x509 -in "$cert_file" -noout -enddate 2>/dev/null | cut -d= -f2)
                echo "  $i) $cert (expires: $expiry)"
                ((i++))
            done
            echo ""
            return 0
        fi
    fi
    return 1
}

# Configure existing Let's Encrypt certificate
setup_existing_letsencrypt() {
    echo ""
    echo -e "${CYAN}═══════════════════════════════════════════════${NC}"
    echo -e "${CYAN}  Configure Existing Let's Encrypt Certificate${NC}"
    echo -e "${CYAN}═══════════════════════════════════════════════${NC}"
    
    if ! list_letsencrypt_certs; then
        print_error "No Let's Encrypt certificates found in /etc/letsencrypt/live/"
        return 1
    fi
    
    # Get available certs
    local certs=()
    for dir in /etc/letsencrypt/live/*/; do
        if [ -f "${dir}fullchain.pem" ]; then
            local name=$(basename "$dir")
            if [ "$name" != "README" ]; then
                certs+=("$name")
            fi
        fi
    done
    
    read -p "Select certificate number (or enter domain name): " selection
    
    local cert_domain=""
    if [[ "$selection" =~ ^[0-9]+$ ]] && [ "$selection" -ge 1 ] && [ "$selection" -le ${#certs[@]} ]; then
        cert_domain="${certs[$((selection-1))]}"
    else
        cert_domain="$selection"
    fi
    
    local cert_path="/etc/letsencrypt/live/${cert_domain}/fullchain.pem"
    local key_path="/etc/letsencrypt/live/${cert_domain}/privkey.pem"
    
    if [ ! -f "$cert_path" ]; then
        print_error "Certificate not found: $cert_path"
        return 1
    fi
    
    print_success "Selected certificate: $cert_domain"
    
    # Update config (don't change port - that's set in main config)
    update_config "$cert_path" "$key_path" ""
    
    # Setup certificate permissions
    echo ""
    print_info "Setting up certificate permissions..."
    
    if id -u speedwholes &>/dev/null && command -v setfacl &>/dev/null; then
        read -p "Grant speedwholes user read access to certificates using ACLs? [Y/n]: " setup_acls
        if [[ ! "$setup_acls" =~ ^[Nn]$ ]]; then
            if [ "$EUID" -eq 0 ]; then
                setfacl -R -m u:speedwholes:rX /etc/letsencrypt/live/ 2>/dev/null || true
                setfacl -R -m u:speedwholes:rX /etc/letsencrypt/archive/ 2>/dev/null || true
                setfacl -R -d -m u:speedwholes:rX /etc/letsencrypt/live/ 2>/dev/null || true
                setfacl -R -d -m u:speedwholes:rX /etc/letsencrypt/archive/ 2>/dev/null || true
                print_success "ACLs configured for speedwholes user"
            else
                print_warning "Run as root to configure ACLs, or manually run:"
                echo "  sudo setfacl -R -m u:speedwholes:rX /etc/letsencrypt/live/"
                echo "  sudo setfacl -R -m u:speedwholes:rX /etc/letsencrypt/archive/"
            fi
        fi
    fi
    
    echo ""
    echo -e "${GREEN}═══════════════════════════════════════════════${NC}"
    echo -e "${GREEN}  Certificate Configuration Complete!${NC}"
    echo -e "${GREEN}═══════════════════════════════════════════════${NC}"
    echo ""
    echo "Restart speedwholes to apply changes:"
    echo "  systemctl restart speedwholes"
    echo ""
}

# Show current TLS configuration
show_current_config() {
    echo ""
    echo -e "${CYAN}Current TLS Configuration:${NC}"
    echo ""
    
    local tls_cert=$(jq -r '.tls_cert // "not set"' "$CONFIG_FILE")
    local tls_key=$(jq -r '.tls_key // "not set"' "$CONFIG_FILE")
    local port=$(jq -r '.port // 3000' "$CONFIG_FILE")
    
    echo "  Port:        $port"
    echo "  Certificate: $tls_cert"
    echo "  Private Key: $tls_key"
    
    if [ "$tls_cert" != "not set" ] && [ -f "$tls_cert" ]; then
        echo ""
        echo -e "${CYAN}Certificate Info:${NC}"
        openssl x509 -in "$tls_cert" -noout -subject -dates 2>/dev/null | sed 's/^/  /'
    elif [ "$tls_cert" != "not set" ]; then
        print_warning "Certificate file not found: $tls_cert"
        # Check if path contains asterisk (common mistake)
        if [[ "$tls_cert" == *"*"* ]]; then
            print_error "Path contains literal asterisk (*) - this won't work!"
            local fixed_path=$(echo "$tls_cert" | sed 's/\*\.//')
            print_info "Try: $fixed_path"
        fi
    fi
    echo ""
}

# Main menu
main() {
    print_banner
    check_config
    check_dependencies
    show_current_config
    
    # Check for existing Let's Encrypt certs
    local has_existing_certs=false
    if [ -d /etc/letsencrypt/live ] && [ "$(ls -A /etc/letsencrypt/live 2>/dev/null)" ]; then
        has_existing_certs=true
        list_letsencrypt_certs
    fi
    
    echo "Choose SSL setup option:"
    echo ""
    echo "  1) Self-signed certificate (development/testing)"
    echo "  2) Let's Encrypt - request new certificate (production)"
    if [ "$has_existing_certs" = true ]; then
        echo "  3) Let's Encrypt - use existing certificate (recommended)"
    else
        echo "  3) Let's Encrypt - use existing certificate (none found)"
    fi
    echo "  4) Show current configuration"
    echo "  5) Exit"
    echo ""
    read -p "Select option [1-5]: " choice
    
    case "$choice" in
        1)
            setup_self_signed
            ;;
        2)
            setup_letsencrypt
            ;;
        3)
            if [ "$has_existing_certs" = true ]; then
                setup_existing_letsencrypt
            else
                print_error "No existing Let's Encrypt certificates found."
                print_info "Use option 2 to request a new certificate."
            fi
            ;;
        4)
            show_current_config
            ;;
        5)
            echo "Goodbye!"
            exit 0
            ;;
        *)
            print_error "Invalid option. Please select 1-5."
            exit 1
            ;;
    esac
}

# Run main function
main "$@"
