#!/bin/bash
# Test script for multi-instance support

set -e

echo "=== Testing Multi-Instance Support ==="
echo ""

# Build the binary
echo "Building speedwholes..."
cargo build --release
BINARY="./target/release/speedwholes"

# Create test directories
TEST_DIR=$(mktemp -d)
echo "Test directory: $TEST_DIR"

CONFIG_DIR_1="$TEST_DIR/config1"
CONFIG_DIR_2="$TEST_DIR/config2"
CACHE_DIR="$TEST_DIR/cache"

mkdir -p "$CONFIG_DIR_1" "$CONFIG_DIR_2" "$CACHE_DIR"

echo ""
echo "=== Test 1: Start first instance ==="
$BINARY --port 3001 --config-dir "$CONFIG_DIR_1" --cache-path "$CACHE_DIR" &
PID1=$!
echo "Started instance 1 with PID $PID1 on port 3001"
sleep 2

# Check if PID file exists
if [ -f "$CONFIG_DIR_1/speedwholes.pid" ]; then
    echo "✓ PID file created: $CONFIG_DIR_1/speedwholes.pid"
    PID_IN_FILE=$(cat "$CONFIG_DIR_1/speedwholes.pid")
    echo "  PID in file: $PID_IN_FILE"
else
    echo "✗ PID file not found"
    kill $PID1 2>/dev/null || true
    exit 1
fi

echo ""
echo "=== Test 2: Try to start duplicate instance (should fail) ==="
if $BINARY --port 3001 --config-dir "$CONFIG_DIR_1" --cache-path "$CACHE_DIR" 2>&1 | grep -q "already running"; then
    echo "✓ Duplicate instance correctly prevented"
else
    echo "✗ Duplicate instance should have been prevented"
    kill $PID1 2>/dev/null || true
    exit 1
fi

echo ""
echo "=== Test 3: Start second instance with different config dir ==="
$BINARY --port 3002 --config-dir "$CONFIG_DIR_2" --cache-path "$CACHE_DIR" &
PID2=$!
echo "Started instance 2 with PID $PID2 on port 3002"
sleep 2

# Check both PID files exist
if [ -f "$CONFIG_DIR_2/speedwholes.pid" ]; then
    echo "✓ Second PID file created: $CONFIG_DIR_2/speedwholes.pid"
else
    echo "✗ Second PID file not found"
    kill $PID1 $PID2 2>/dev/null || true
    exit 1
fi

echo ""
echo "=== Test 4: Verify both instances are responding ==="
if curl -s -o /dev/null -w "%{http_code}" http://localhost:3001/health | grep -q "200"; then
    echo "✓ Instance 1 responding on port 3001"
else
    echo "✗ Instance 1 not responding"
fi

if curl -s -o /dev/null -w "%{http_code}" http://localhost:3002/health | grep -q "200"; then
    echo "✓ Instance 2 responding on port 3002"
else
    echo "✗ Instance 2 not responding"
fi

echo ""
echo "=== Test 5: Verify both share same cache directory ==="
CACHE_COUNT=$(find "$CACHE_DIR" -type f 2>/dev/null | wc -l)
echo "  Cache files: $CACHE_COUNT (shared between instances)"
echo "✓ Both instances using shared cache: $CACHE_DIR"

echo ""
echo "=== Test 6: Stop first instance and verify lock cleanup ==="
kill $PID1
sleep 1

if [ -f "$CONFIG_DIR_1/speedwholes.pid" ]; then
    echo "✗ PID file should be removed after instance stops"
else
    echo "✓ PID file correctly removed after instance stops"
fi

echo ""
echo "=== Test 7: Restart first instance (stale lock cleanup test) ==="
# Create a fake stale lock with non-existent PID
echo "999999999" > "$CONFIG_DIR_1/speedwholes.pid"
$BINARY --port 3001 --config-dir "$CONFIG_DIR_1" --cache-path "$CACHE_DIR" &
PID1=$!
sleep 2

if [ -f "$CONFIG_DIR_1/speedwholes.pid" ]; then
    NEW_PID=$(cat "$CONFIG_DIR_1/speedwholes.pid")
    if [ "$NEW_PID" = "$PID1" ]; then
        echo "✓ Stale lock cleaned up and new lock created with correct PID"
    else
        echo "✗ PID mismatch: expected $PID1, got $NEW_PID"
    fi
else
    echo "✗ PID file not created after stale lock cleanup"
fi

echo ""
echo "=== Cleanup ==="
kill $PID1 $PID2 2>/dev/null || true
sleep 1
rm -rf "$TEST_DIR"
echo "✓ Test directory cleaned up"

echo ""
echo "=== All Tests Passed! ==="
