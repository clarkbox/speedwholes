#!/bin/bash
# Speedwholes Installation Script
# Run as root: sudo ./install.sh [--upgrade] [--tls-domain DOMAIN]
#
# Safe for upgrades - preserves:
#   - speedwholes.config.json (your configuration)
#   - www/config.json (frontend API URL)
#   - subdomainConfig/*.json (your subdomain configs)
#   - certs/ (your SSL certificates)
#   - cache directory

# Grant the service user read access to one Let's Encrypt lineage.
# Does not change the mode of any private key.
setup_tls_acl() {
    local DOMAIN="${1:-}"
    local LE_DIR="${LE_DIR:-/etc/letsencrypt}"
    local SERVICE_USER="${SERVICE_USER:-speedwholes}"

    if [ -z "$DOMAIN" ]; then
        echo "Pass --tls-domain DOMAIN to grant ${SERVICE_USER} read access to Let's Encrypt files."
        echo "Private key modes are left unchanged."
        echo "Example: sudo ./install.sh --tls-domain example.com"
        return 0
    fi
    if ! command -v setfacl >/dev/null 2>&1; then
        echo "setfacl is required. Install it with: sudo apt install acl"
        return 1
    fi

    setfacl -m "u:${SERVICE_USER}:x" "${LE_DIR}/live" "${LE_DIR}/archive"
    setfacl -R -m "u:${SERVICE_USER}:rX" "${LE_DIR}/live/${DOMAIN}" "${LE_DIR}/archive/${DOMAIN}"
    setfacl -d -m "u:${SERVICE_USER}:r" "${LE_DIR}/archive/${DOMAIN}"
    echo "[+] Granted ${SERVICE_USER} read access to ${DOMAIN} (key mode unchanged)"
}

main() {
    set -e

    if [ "$EUID" -ne 0 ]; then
        echo "Please run as root: sudo ./install.sh"
        exit 1
    fi

    local tls_domain="${TLS_DOMAIN:-}"
    while [ $# -gt 0 ]; do
        case "$1" in
            --tls-domain)
                tls_domain="${2:-}"
                shift 2
                ;;
            --upgrade)
                shift
                ;;
            -h|--help)
                echo "Usage: sudo ./install.sh [--upgrade] [--tls-domain DOMAIN]"
                exit 0
                ;;
            *)
                echo "Unknown argument: $1"
                exit 2
                ;;
        esac
    done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Detect fresh install vs upgrade
if [ -f /usr/local/bin/speedwholes ]; then
    UPGRADE=true
    echo "========================================"
    echo "  Upgrading Speedwholes"
    echo "========================================"
else
    UPGRADE=false
    echo "========================================"
    echo "  Installing Speedwholes"
    echo "========================================"
fi
echo ""

# Create user if doesn't exist
if ! id -u speedwholes &>/dev/null; then
    useradd -r -s /bin/false speedwholes
    echo "[+] Created speedwholes user"
fi

# Install binary
echo "[+] Installing binary: /usr/local/bin/speedwholes"
install -m 755 "${SCRIPT_DIR}/bin/speedwholes" /usr/local/bin/

# Create base directory
mkdir -p /etc/speedwholes

# Update application files (these are safe to replace)
echo "[+] Updating application files:"
echo "    - /etc/speedwholes/www/ (frontend)"
echo "    - /etc/speedwholes/scripts/"
echo "    - /etc/speedwholes/docs/"
echo "    - /etc/speedwholes/examples/"

# Backup www/config.json if it exists (contains user's API URL)
WWW_CONFIG_BACKUP=""
if [ -f /etc/speedwholes/www/config.json ]; then
    WWW_CONFIG_BACKUP=$(mktemp)
    cp /etc/speedwholes/www/config.json "$WWW_CONFIG_BACKUP"
fi

rm -rf /etc/speedwholes/www
rm -rf /etc/speedwholes/scripts
rm -rf /etc/speedwholes/docs
rm -rf /etc/speedwholes/examples

cp -r "${SCRIPT_DIR}/etc/speedwholes/scripts" /etc/speedwholes/
cp -r "${SCRIPT_DIR}/etc/speedwholes/docs" /etc/speedwholes/
cp -r "${SCRIPT_DIR}/etc/speedwholes/examples" /etc/speedwholes/
cp -r "${SCRIPT_DIR}/etc/speedwholes/www" /etc/speedwholes/

# Restore www/config.json if we backed it up
if [ -n "$WWW_CONFIG_BACKUP" ] && [ -f "$WWW_CONFIG_BACKUP" ]; then
    cp "$WWW_CONFIG_BACKUP" /etc/speedwholes/www/config.json
    rm -f "$WWW_CONFIG_BACKUP"
fi

# Preserve user data
echo "[=] Preserving user data:"

# subdomainConfig - create dir but never touch contents
if [ -d /etc/speedwholes/subdomainConfig ]; then
    count=$(find /etc/speedwholes/subdomainConfig -name '*.json' 2>/dev/null | wc -l)
    echo "    - subdomainConfig/ (${count} configs)"
else
    mkdir -p /etc/speedwholes/subdomainConfig
    echo "    - subdomainConfig/ (created empty)"
fi

# certs - never touch
if [ -d /etc/speedwholes/certs ]; then
    echo "    - certs/ (preserved)"
fi

# Main config - only create from example if missing
if [ -f /etc/speedwholes/speedwholes.config.json ]; then
    echo "    - speedwholes.config.json (preserved)"
else
    cp "${SCRIPT_DIR}/etc/speedwholes/examples/speedwholes.config.example.json" \
       /etc/speedwholes/speedwholes.config.json
    echo "    - speedwholes.config.json (created from example)"
    echo ""
    echo "    *** EDIT /etc/speedwholes/speedwholes.config.json BEFORE STARTING ***"
fi

# www config - show status (already handled during www copy)
if [ -n "$WWW_CONFIG_BACKUP" ]; then
    echo "    - www/config.json (preserved)"
else
    echo "    - www/config.json (created from example)"
fi

# Set ownership (but preserve file permissions)
chown -R speedwholes:speedwholes /etc/speedwholes

# Create default cache directory if missing
if [ ! -d /var/cache/speedwholes ]; then
    mkdir -p /var/cache/speedwholes
    chown speedwholes:speedwholes /var/cache/speedwholes
    echo "[+] Created /var/cache/speedwholes"
fi

# Let's Encrypt: grant read access with an ACL. Never chmod private keys.
if [ -n "$tls_domain" ]; then
    TLS_DOMAIN="$tls_domain" setup_tls_acl "$tls_domain"
else
    echo ""
    echo "Let's Encrypt keys were not modified."
    echo "Grant the service user read access without widening key modes:"
    echo "  sudo ./install.sh --upgrade --tls-domain example.com"
    echo "Or load the key with systemd LoadCredential= (see docs/deployment.md)."
fi

echo ""
echo "========================================"
if [ "$UPGRADE" = true ]; then
    echo "  Upgrade complete!"
    echo "========================================"
    echo ""
    echo "Restart the service to use new version:"
    echo "  sudo systemctl restart speedwholes"
fi

echo "  AFTER INSTALLATION:"
echo "========================================"
echo ""
echo "Installed:"
echo "  Binary:   /usr/local/bin/speedwholes"
echo "  Config:   /etc/speedwholes/speedwholes.config.json"
echo "  Frontend: /etc/speedwholes/www/"
echo ""
echo "Next steps:"
echo "  1. Run configuration setup (recommended):"
echo "     speedwholes init"
echo ""
echo "     Or manually edit:"
echo "       - /etc/speedwholes/speedwholes.config.json (server)"
echo "       - /etc/speedwholes/www/config.json (frontend API URL)"
echo ""
echo "  2. Install systemd service:"
echo "     sudo cp /etc/speedwholes/examples/speedwholes.service /etc/systemd/system/"
echo "     sudo systemctl daemon-reload"
echo "     sudo systemctl enable --now speedwholes"
echo ""
echo "  3. Point nginx at /etc/speedwholes/www/"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi


