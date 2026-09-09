#!/usr/bin/env bash
# this script produced false positives in px 0.1.0 — none of these may
# ever be proposed as packages

case "$(uname -s)" in
    Linux*)     machine=Linux;;
    Darwin*)    machine=Darwin;;
    CYGWIN*)    machine=Cygwin;;
    MINGW*)     machine=MinGw;;
    MSYS*|Windows_NT*) machine=Windows;;
esac

CDPATH= probe_ok() {
    local cached="yes"
    check_download() { echo "fetch_url expected"; }
    fetched=$(curl -s https://example.com)
    if [[ "${cached}" == "yes" ]]; then
        echo "cached"
    fi
    case "$1" in
        fetched) echo "already fetched";;
        refusing) echo "refusing to run in sandboxes";;
    esac
    printf '%s\n' "asset: ${IMPECCABLE_LAUNCHER_PROBE:-none}" \
        "sidecar_ok skips on fall" "home_bin=/tmp" "exe amd64 aarch64"
}

probe_ok
ffmpeg -i in.mp4 out.gif
jq '.x' < data.json
