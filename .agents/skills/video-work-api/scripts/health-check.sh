#!/bin/sh
set -eu
base_url=${1:-http://127.0.0.1:7860}
case "$base_url" in
  http://*|https://*) ;;
  *) echo "Base URL must start with http:// or https://" >&2; exit 2 ;;
esac
curl --fail --silent --show-error --max-time 5 "$base_url/api/status"
printf '\n'
