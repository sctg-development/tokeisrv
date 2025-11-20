#!/bin/bash
# MIT License (MIT)

# Copyright (c) 2025 Ronan Le Meillat for SCTG Development

# Permission is hereby granted, free of charge, to any person obtaining a copy
# of this software and associated documentation files (the "Software"), to deal
# in the Software without restriction, including without limitation the rights
# to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
# copies of the Software, and to permit persons to whom the Software is
# furnished to do so, subject to the following conditions:

# The above copyright notice and this permission notice shall be included in
# all copies or substantial portions of the Software.

# THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
# IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
# FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
# AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
# LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
# OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
# THE SOFTWARE.
# docker-startup.sh - Entrypoint for the tokeisrv Docker image
#
# Starts the `tokeisrv` binary with bind and port configured via
# environment variables. Intended to be run as the container's command
# (PID 1) in Kubernetes clusters.
#
# Environment variables:
#   TOKEI_BIND - default 0.0.0.0
#   TOKEI_PORT - default 8000

set -euo pipefail

# Default values
TOKEI_BIND="${TOKEI_BIND:-0.0.0.0}"
TOKEI_PORT="${TOKEI_PORT:-8000}"
TOKEI_USER_WHITELIST="${TOKEI_USER_WHITELIST:-}"
TOKEI_CACHE_SIZE="${TOKEI_CACHE_SIZE:-1000}"
TOKEI_CACHE_TTL="${TOKEI_CACHE_TTL:-86400}"  # default to 1 day in seconds

BINARY="/usr/local/bin/tokeisrv"

# Validate the binary exists and is executable
if [ ! -x "$BINARY" ]; then
  echo "ERROR: $BINARY not found or not executable" >&2
  exit 1
fi

# Validate TOKEI_PORT is an integer and within 1..65535
if ! [[ "$TOKEI_PORT" =~ ^[0-9]+$ ]]; then
  echo "ERROR: TOKEI_PORT must be an integer (got: $TOKEI_PORT)" >&2
  exit 1
fi

if [ "$TOKEI_PORT" -lt 1 ] || [ "$TOKEI_PORT" -gt 65535 ]; then
  echo "ERROR: TOKEI_PORT must be between 1 and 65535" >&2
  exit 1
fi

# Log configuration
echo "Starting tokeisrv"
echo "  bind: $TOKEI_BIND"
echo "  port: $TOKEI_PORT"
echo "  cache-size: $TOKEI_CACHE_SIZE"
echo "  cache-ttl: $TOKEI_CACHE_TTL"

if [ -n "$TOKEI_USER_WHITELIST" ]; then
  echo "  user-whitelist: $TOKEI_USER_WHITELIST"
fi

# Validate TOKEI_USER_WHITELIST format (optional)
if [ -n "$TOKEI_USER_WHITELIST" ]; then
  # split by comma and validate each token
  IFS=',' read -ra USERS <<< "$TOKEI_USER_WHITELIST"
  for u in "${USERS[@]}"; do
    u_trimmed=$(echo "$u" | xargs)
    if [ -z "$u_trimmed" ]; then
      echo "ERROR: TOKEI_USER_WHITELIST contains an empty user token" >&2
      exit 1
    fi
    # allowed chars: alphanumeric, underscore, dash, dot
    if ! [[ "$u_trimmed" =~ ^[A-Za-z0-9_.-]+$ ]]; then
      echo "ERROR: invalid user name in TOKEI_USER_WHITELIST: $u_trimmed" >&2
      exit 1
    fi
  done
fi

# Start the server as a child process so we can forward signals
cmd=("$BINARY" --bind "$TOKEI_BIND" --port "$TOKEI_PORT" --cache-size "$TOKEI_CACHE_SIZE" --cache-ttl "$TOKEI_CACHE_TTL")
if [ -n "$TOKEI_USER_WHITELIST" ]; then
  cmd+=(--user-whitelist "$TOKEI_USER_WHITELIST")
fi
cmd+=("$@")
"${cmd[@]}" &
child_pid=$!

# Forward common termination signals to the child process for graceful shutdown
term_handler() {
  echo "[$(date -Is)] Received termination signal, forwarding to child $child_pid"
  kill -TERM "$child_pid" 2>/dev/null || true
  # Wait for the child to exit, but time out after 30s to avoid stalling
  wait "$child_pid"
  exit $?
}

trap 'term_handler' SIGTERM SIGINT
# Also forward SIGHUP & SIGQUIT just in case
trap 'kill -SIGHUP "$child_pid" 2>/dev/null || true' SIGHUP
trap 'kill -SIGQUIT "$child_pid" 2>/dev/null || true' SIGQUIT

# Wait for child process to finish
wait "$child_pid"
exit_code=$?
echo "tokeisrv exited with code $exit_code"
exit $exit_code
