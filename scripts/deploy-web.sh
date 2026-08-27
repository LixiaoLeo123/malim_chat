#!/usr/bin/env bash
set -euo pipefail

remote="${1:-root@106.53.70.76}"
target="/opt/malim_chat/web/"

VITE_BASE_PATH=/malim_chat/ npm run build
rsync -a "$PWD/dist/" "$remote:$target"
ssh "$remote" 'nginx -t && systemctl reload nginx'
