#!/usr/bin/env bash
set -euo pipefail

remote="${1:-root@106.53.70.76}"
target="/opt/malim_chat/web/"

VITE_BASE_PATH=/malim_chat/ npm run build
rsync -a "$PWD/dist/" "$remote:$target"
ssh "$remote" 'nginx -t && systemctl reload nginx'

# rsync never deletes, so hashed bundles and their source maps pile up every deploy.
# Keep the two newest of each: the live one, plus the previous one for a client that is
# still holding the old index.html.
ssh "$remote" 'cd /opt/malim_chat/web/assets && for pattern in "index-*.js" "index-*.css" "index-*.js.map"; do ls -t $pattern 2>/dev/null | tail -n +3 | xargs -r rm -v -f; done'
