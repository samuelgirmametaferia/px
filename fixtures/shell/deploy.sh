#!/usr/bin/env bash
set -euo pipefail
ffmpeg -i input.mp4 output.gif
jq '.name' < data.json
inotifywait -r /tmp &
curl -s https://example.com
echo done
