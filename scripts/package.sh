#!/bin/bash
set -e

# Speedwholes Packaging Script
# Creates a distributable tarball ready for deployment
#
# Usage: ./scripts/package.sh [version]
#        version defaults to git tag or datestamp

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_ROOT"

# Determine version
if [ -n "$1" ]; then
    VERSION="$1"
elif git describe --tags --exact-match 2>/dev/null; then
    VERSION="$(git describe --tags --exact-match)"
else
    VERSION="$(date +%Y%m%d)"
fi

BUILD_DIR="${PROJECT_ROOT}/build"
PACKAGE_NAME="speedwholes-${VERSION}"
PACKAGE_DIR="${BUILD_DIR}/${PACKAGE_NAME}"

echo "========================================"
echo "  Speedwholes Packaging Script"
echo "  Version: ${VERSION}"
echo "========================================"
echo ""

# Clean previous build
rm -rf "$BUILD_DIR"
mkdir -p "$PACKAGE_DIR"

# 1. Build Rust binary
echo "[1/4] Building Rust binary (release)..."
cd "${PROJECT_ROOT}/server"
cargo build --release
BINARY="${PROJECT_ROOT}/server/target/release/speedwholes"
if [ ! -f "$BINARY" ]; then
    echo "Error: Binary not found at $BINARY"
    exit 1
fi
echo "      Built: $BINARY ($(du -h "$BINARY" | cut -f1))"

# 2. Build Vue frontend
echo ""
echo "[2/4] Building Vue frontend..."
cd "${PROJECT_ROOT}/www"
npm ci --silent
npm run build
if [ ! -d "dist" ]; then
    echo "Error: Frontend build failed - dist/ not found"
    exit 1
fi
echo "      Built: www/dist/"

# 3. Assemble package structure
echo ""
echo "[3/4] Assembling package..."
cd "$PROJECT_ROOT"

# Binary goes in bin/
mkdir -p "${PACKAGE_DIR}/bin"
cp "$BINARY" "${PACKAGE_DIR}/bin/"

# License files travel with the tarball
cp "${PROJECT_ROOT}/LICENSE" "${PACKAGE_DIR}/LICENSE"
cp "${PROJECT_ROOT}/THIRD_PARTY_NOTICES.md" "${PACKAGE_DIR}/THIRD_PARTY_NOTICES.md"

# Everything else goes in etc/speedwholes/
ETC_DIR="${PACKAGE_DIR}/etc/speedwholes"
mkdir -p "${ETC_DIR}"

# Scripts
mkdir -p "${ETC_DIR}/scripts"
cp "${PROJECT_ROOT}/server/setup-ssl.sh" "${ETC_DIR}/scripts/"
cp "${PROJECT_ROOT}/server/setup-config.sh" "${ETC_DIR}/scripts/"
chmod +x "${ETC_DIR}/scripts/"*.sh

# Documentation
mkdir -p "${ETC_DIR}/docs"
cp "${PROJECT_ROOT}/docs/"*.md "${ETC_DIR}/docs/" 2>/dev/null || true
cp "${PROJECT_ROOT}/server/README.md" "${ETC_DIR}/docs/server.md"
cp "${PROJECT_ROOT}/www/README.md" "${ETC_DIR}/docs/www.md"
cp "${PROJECT_ROOT}/README.md" "${ETC_DIR}/docs/README.md" 2>/dev/null || true

# Example configs
mkdir -p "${ETC_DIR}/examples"
cp "${PROJECT_ROOT}/server/speedwholes.config.example.json" "${ETC_DIR}/examples/"
cp "${PROJECT_ROOT}/systemd-examples/speedwholes.service" "${ETC_DIR}/examples/"

# Frontend dist
mkdir -p "${ETC_DIR}/www"
cp -r "${PROJECT_ROOT}/www/dist/"* "${ETC_DIR}/www/"

# Empty subdomainConfig directory
mkdir -p "${ETC_DIR}/subdomainConfig"

# Install script
cp "${PROJECT_ROOT}/scripts/install.sh" "${PACKAGE_DIR}/install.sh"
chmod +x "${PACKAGE_DIR}/install.sh"

# 4. Create tarball
echo ""
echo "[4/4] Creating tarball..."
cd "$BUILD_DIR"
tar -czf "${PACKAGE_NAME}.tar.gz" "$PACKAGE_NAME"
rm -rf "$PACKAGE_DIR"

TARBALL="${BUILD_DIR}/${PACKAGE_NAME}.tar.gz"
echo ""
echo "========================================"
echo "  Package created!"
echo "========================================"
echo ""
echo "  ${TARBALL}"
echo "  Size: $(du -h "$TARBALL" | cut -f1)"
echo ""
echo "Deploy:"
echo "  scp ${TARBALL} yourserver:/tmp/"
echo "  ssh yourserver 'cd /tmp && tar xzf ${PACKAGE_NAME}.tar.gz && cd ${PACKAGE_NAME} && sudo ./install.sh'"
echo ""
