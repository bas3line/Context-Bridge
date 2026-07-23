#!/bin/sh
set -eu

if [ "${1:-}" = "--version" ]; then
  echo "context-bridge fake agent 1.0.0"
  exit 0
fi

: "${CB_EVENT_SINK:?Context Bridge did not provide CB_EVENT_SINK}"
: "${CB_SESSION_METADATA:?Context Bridge did not provide CB_SESSION_METADATA}"

printf '%s\n' \
  '{"external_event_id":"example-1","kind":"user_message","payload":{"type":"message","data":{"content":"Example objective"}},"metadata":{}}' \
  > "$CB_EVENT_SINK"
printf '%s\n' \
  '{"external_session_id":"example-session"}' \
  > "$CB_SESSION_METADATA"

if [ -n "${CB_BOOTSTRAP_PATH:-}" ]; then
  printf 'Received a handoff at %s\n' "$CB_BOOTSTRAP_PATH"
fi
