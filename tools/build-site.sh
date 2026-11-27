#!/bin/sh
# Assemble the website (https://krynv.github.io/ditto-rs/) into a folder: the pages in site/
# plus the app's own quick-paste interface, which runs as the live demo on the page.
#
#   tools/build-site.sh [output-folder]      (default: _site)
set -eu
cd "$(dirname "$0")/.."
out="${1:-_site}"

rm -rf "$out"
mkdir -p "$out/demo/ui"
cp -R site/. "$out/"

# The demo is the real interface, unchanged, with a stand-in for the backend loaded first.
cp ui/*.css ui/*.js ui/*.png "$out/demo/ui/"
sed 's#<script src="common.js"></script>#<script src="../mock.js"></script><script src="common.js"></script>#' \
    ui/index.html > "$out/demo/ui/index.html"
if ! grep -q '<script src="../mock.js"></script>' "$out/demo/ui/index.html"; then
    echo "error: ui/index.html no longer loads common.js the way this script expects" >&2
    exit 1
fi
echo "site assembled in $out"
