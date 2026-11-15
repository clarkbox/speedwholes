#!/bin/bash

# Speedwholes Local Development Starter
# This script starts both the backend server and frontend Vite dev server in tmux panes

# Initialize variables
SESSION_NAME="speedwholes"
runPath=""
logPath=""
port="8081"

# Output usage
echo ""
echo "-------------------------------------------------------------------------------"
echo "Usage: $0 [ --runPath /runPath ] [ --logPath /logPath ] [ --port 8081 ]"
echo "### runPath is the location of server and www directories"
echo "### if logPath is set, the tmux panes will pipe their output to the logPath for easier debugging"
echo "### if logPath is not set, logs will be written to /tmp"
echo "### port is the port for the frontend Vite dev server (default: 8081)"
echo "-------------------------------------------------------------------------------"
echo ""

# Current script absolute path
scriptPath="$(dirname "$(readlink -f "$0")")"
# By default target dir is the script's directory, where server and www repos are located
targetDir="$scriptPath"

# Parse named arguments
while [ "$#" -gt 0 ]; do
    case "$1" in
        --runPath)
            runPath="$2"
            shift 2
            ;;
        --logPath)
            logPath="$2"
            shift 2
            ;;
        --port)
            port="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# If no runPath directory, use the target directory
if [ -z "$runPath" ]; then
    runPath=$targetDir
fi

echo "Run directory: ${runPath}"

# Check for the existence of 'server' directory
if [ -d "${runPath}/server" ]; then
    echo "'server' directory found in ${runPath}/server."
else
    echo "'server' directory not found in ${runPath}."
    exit 1
fi

# Check for the existence of 'www' directory
if [ -d "${runPath}/www" ]; then
    echo "'www' directory found in ${runPath}/www."
else
    echo "'www' directory not found in ${runPath}."
    exit 1
fi

# Local config, certificate, and cache. Never written into the repo's tracked tree.
dev_config="${runPath}/dev-config"
mkdir -p "${dev_config}/certs" "${dev_config}/subdomainConfig" "${dev_config}/cache"
if [ ! -f "${dev_config}/certs/cert.pem" ] || [ ! -f "${dev_config}/certs/key.pem" ]; then
    echo "Generating a local self-signed certificate in ${dev_config}/certs"
    openssl req -x509 -newkey rsa:2048 \
        -keyout "${dev_config}/certs/key.pem" \
        -out "${dev_config}/certs/cert.pem" \
        -days 365 -nodes \
        -subj "/CN=localhost" \
        -addext "subjectAltName=DNS:localhost,IP:127.0.0.1"
    chmod 600 "${dev_config}/certs/key.pem"
fi
if [ ! -f "${dev_config}/speedwholes.config.json" ]; then
    echo "Writing ${dev_config}/speedwholes.config.json from the example config"
    python3 - "${runPath}" "${dev_config}" <<'PY'
import json
import sys
from pathlib import Path

run_path, dev_config = sys.argv[1], sys.argv[2]
example = json.loads(
    Path(run_path, "server", "speedwholes.config.example.json").read_text()
)
example["port"] = 3000
example["cache_path"] = str(Path(dev_config, "cache"))
example["tls_cert"] = str(Path(dev_config, "certs", "cert.pem"))
example["tls_key"] = str(Path(dev_config, "certs", "key.pem"))
example["allow_all_sources"] = False
example["allow_private_networks"] = False
example["admin_token"] = "dev-only-change-me"
Path(dev_config, "speedwholes.config.json").write_text(
    json.dumps(example, indent=2) + "\n"
)
PY
fi

# Check if tmux session already exists
if tmux has-session -t $SESSION_NAME 2>/dev/null; then
    echo "Tmux session '$SESSION_NAME' already exists."
    echo "Attaching to existing session..."
    tmux attach-session -t $SESSION_NAME
    exit 0
fi

# Create log directory if logPath is specified
if [ -z "$logPath" ]; then
    logPath="/tmp/speedwholes-logs"
fi

mkdir -p "$logPath"
echo "Logs will be written to: ${logPath}"

# Create a new tmux session
echo "Creating tmux session: $SESSION_NAME"
tmux new-session -d -s $SESSION_NAME

# Split the window into two horizontal panes
tmux split-window -h -t $SESSION_NAME

# Configure pane titles
tmux select-pane -t $SESSION_NAME:0.0 -T "Backend Server (port 3000)"
tmux select-pane -t $SESSION_NAME:0.1 -T "Frontend Vite Dev (port ${port})"

# Start the Backend Server (left pane)
echo "Starting backend server in pane 0..."
tmux send-keys -t $SESSION_NAME:0.0 "cd ${runPath}/server" C-m
tmux send-keys -t $SESSION_NAME:0.0 "echo '🚀 Starting Speedwholes Backend Server...'" C-m
tmux send-keys -t $SESSION_NAME:0.0 "echo 'Log: ${logPath}/server.log'" C-m
tmux send-keys -t $SESSION_NAME:0.0 "echo ''" C-m
tmux send-keys -t $SESSION_NAME:0.0 "RUST_LOG=speedwholes=info cargo run -- --port 3000 --config-dir ${runPath}/dev-config --log-format text" C-m

# Start the Frontend Vite Dev Server (right pane)
echo "Starting frontend Vite dev server in pane 1..."
tmux send-keys -t $SESSION_NAME:0.1 "cd ${runPath}/www" C-m
tmux send-keys -t $SESSION_NAME:0.1 "echo '🌐 Starting Speedwholes Frontend (Vite)...'" C-m
tmux send-keys -t $SESSION_NAME:0.1 "echo 'Log: ${logPath}/frontend.log'" C-m
tmux send-keys -t $SESSION_NAME:0.1 "echo ''" C-m
tmux send-keys -t $SESSION_NAME:0.1 "npm run dev -- --port ${port} --host 127.0.0.1" C-m

# Pipe output to log files
echo "Piping output to log files..."
tmux pipe-pane -o -t $SESSION_NAME:0.0 "exec cat >> '${logPath}/server.log'"
tmux pipe-pane -o -t $SESSION_NAME:0.1 "exec cat >> '${logPath}/frontend.log'"

# Create a status bar message
tmux set-option -t $SESSION_NAME status-left "[Speedwholes Dev] "
tmux set-option -t $SESSION_NAME status-right "Backend: :3000 | Frontend: :${port}"

echo ""
echo "✅ Tmux session created successfully!"
echo ""
echo "📍 Services:"
echo "   Backend API:  https://localhost:3000"
echo "   Frontend UI:  http://127.0.0.1:${port}"
echo ""
echo "📝 Logs:"
echo "   Backend:  ${logPath}/server.log"
echo "   Frontend: ${logPath}/frontend.log"
echo ""
echo "💡 Tmux commands:"
echo "   Switch panes:     Ctrl+b then arrow keys"
echo "   Detach:           Ctrl+b then d"
echo "   Reattach:         tmux attach -t $SESSION_NAME"
echo "   Kill session:     tmux kill-session -t $SESSION_NAME"
echo ""
echo "Attaching to tmux session..."
sleep 2

# Attach to the tmux session
tmux attach-session -t $SESSION_NAME
