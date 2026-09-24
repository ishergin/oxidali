#!/usr/bin/env bash
set -eu
HIL_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HIL_VENV="$HIL_ROOT/.venv"
HIL_VENDOR="$HIL_ROOT/vendor"
HIL_STATE="$HIL_ROOT/state"
mkdir -p "$HIL_STATE"

echo "== python venv + hil package =="
[ -x "$HIL_VENV/bin/python3" ] || python3 -m venv "$HIL_VENV"
"$HIL_VENV/bin/pip" install --quiet --upgrade pip
"$HIL_VENV/bin/pip" install --quiet -e "$HIL_ROOT"
"$HIL_VENV/bin/python3" -c 'import cv2, numpy, serial, requests, pytest; print("deps ok")'
"$HIL_VENV/bin/hil" --help >/dev/null && echo "hil CLI ok"

echo "== uvc-util (UVC exposure/white-balance control) =="
UVC_DIR="$HIL_VENDOR/uvc-util"
UVC_BIN="$UVC_DIR/uvc-util"
if [ ! -x "$UVC_BIN" ]; then
    mkdir -p "$HIL_VENDOR"
    if [ ! -d "$UVC_DIR/.git" ]; then
        git clone --depth 1 https://github.com/jtfrey/uvc-util.git "$UVC_DIR"
    fi
    (
        cd "$UVC_DIR"
        if [ -f Makefile ]; then
            make
        else
            clang -o uvc-util src/*.m -framework Foundation -framework IOKit \
                  -framework CoreFoundation 2>/dev/null \
            || clang -o uvc-util ./*.m -framework Foundation -framework IOKit \
                  -framework CoreFoundation
        fi
        git rev-parse HEAD > "$HIL_STATE/uvc-util.commit"
    )
fi
if [ -x "$UVC_BIN" ]; then
    echo "uvc-util built: $UVC_BIN (commit $(cat "$HIL_STATE/uvc-util.commit" 2>/dev/null || echo '?'))"
else
    echo "WARNING: uvc-util build failed — camera locking unavailable (ae-relative only)" >&2
fi
echo "setup done"
