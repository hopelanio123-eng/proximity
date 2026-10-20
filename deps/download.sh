#!/usr/bin/env bash

set -ex

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/refs.sh"
source "$SCRIPT_DIR/hashes.sh"

clone_at_commit() {
    local url="$1" dir="$2" commit="$3"
    if [ ! -d "$dir" ]; then
        git clone --filter=blob:none "$url" "$dir"
        git -C "$dir" checkout "$commit"
    fi
}

clone_at_commit "$X264_URL" x264 "$X264_COMMIT"
clone_at_commit "$FFMPEG_URL" ffmpeg "$FFMPEG_COMMIT"

if [ "$TARGET_OS" == "linux" ]; then
    clone_at_commit "$NV_CODEC_URL" nv-codec-headers "$NV_CODEC_COMMIT"
    clone_at_commit "$LIBVA_URL" libva "$LIBVA_COMMIT"
fi
if [ "$TARGET_OS" == "windows" ]; then
    clone_at_commit "$NV_CODEC_URL" nv-codec-headers "$NV_CODEC_COMMIT"
fi

if [ "$TARGET_OS" == "windows" ] && [ "$HOST_OS" == "windows" ]; then
    cd ffmpeg
    git apply ../command_limit.patch
    git apply ../awk.patch
fi




