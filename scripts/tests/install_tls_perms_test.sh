#!/bin/bash
# Proves setup_tls_acl grants one lineage read access and does not chmod keys.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=../install.sh
source "${ROOT}/scripts/install.sh"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

export LE_DIR="$tmp"
mkdir -p "${LE_DIR}/live/ex.com" "${LE_DIR}/archive/ex.com"
mkdir -p "${LE_DIR}/live/other.com" "${LE_DIR}/archive/other.com"
printf 'key\n' > "${LE_DIR}/live/ex.com/privkey.pem"
printf 'key\n' > "${LE_DIR}/archive/ex.com/privkey1.pem"
printf 'key\n' > "${LE_DIR}/live/other.com/privkey.pem"
printf 'key\n' > "${LE_DIR}/archive/other.com/privkey1.pem"
chmod 600 \
    "${LE_DIR}/live/ex.com/privkey.pem" \
    "${LE_DIR}/archive/ex.com/privkey1.pem" \
    "${LE_DIR}/live/other.com/privkey.pem" \
    "${LE_DIR}/archive/other.com/privkey1.pem"

SERVICE_USER=nobody setup_tls_acl ex.com

# setfacl stores the ACL mask in the group permission bits, so an effective
# named-user read shows up as mode 640. The script never chmods the key.
# World access (the last digit) must stay 0. Untouched lineages stay 600.
for key in "${LE_DIR}/live/ex.com/privkey.pem" "${LE_DIR}/archive/ex.com/privkey1.pem"; do
    mode="$(stat -c %a "$key")"
    case "$mode" in
        600|640) ;;
        *)
            echo "expected mode 600 or 640 for $key, got $mode"
            exit 1
            ;;
    esac
    if [ "${mode: -1}" != "0" ]; then
        echo "key is world-accessible: $key mode $mode"
        exit 1
    fi
done
other_mode="$(stat -c %a "${LE_DIR}/archive/other.com/privkey1.pem")"
if [ "$other_mode" != "600" ]; then
    echo "expected untouched key to stay 600, got $other_mode"
    exit 1
fi

if ! getfacl -p "${LE_DIR}/archive/ex.com/privkey1.pem" | grep -q 'user:nobody:r'; then
    echo "ex.com private key is missing user:nobody:r"
    getfacl -p "${LE_DIR}/archive/ex.com/privkey1.pem"
    exit 1
fi

if getfacl -p "${LE_DIR}/archive/other.com/privkey1.pem" | grep -q 'user:nobody:'; then
    echo "other.com private key must not grant nobody"
    exit 1
fi

echo "install_tls_perms_test: ok"
