#!/usr/bin/env bash
# Build redragon-gui-<version>-<arch>.AppImage from a release build.
#
#   packaging/build-appimage.sh            # builds target/release if needed
#   OUT=dist VERSION=0.2.0 packaging/build-appimage.sh
#
# The binary links only libc/libm/libgcc and loads GL, Wayland and X11 at run
# time (eframe), so the host's graphics stack is used and nothing else is
# bundled. Build on an old glibc (the CI uses Ubuntu 22.04) so the AppImage runs
# on current and older distributions.
#
# An AppImage cannot install the udev rule the mouse needs; the rule is written
# next to the AppImage and also kept inside it (`--appimage-extract` it).
set -euo pipefail

cd "$(dirname "$0")/.."

ARCH="${ARCH:-x86_64}" # only x86_64 is built and checksum-pinned so far
VERSION="${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)}"
OUT="${OUT:-dist}"
BUILD="${BUILD:-build/appimage}"
BIN="${BIN:-target/release/redragon-gui}"

# appimagetool is pinned and verified; bump both together.
APPIMAGETOOL_VERSION="1.9.1"
APPIMAGETOOL_SHA256_x86_64="ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0"

APP_ID="io.github.redragon_gui.redragon-gui"
APPDIR="$BUILD/AppDir"
TOOLS="$BUILD/tools"

if [ ! -x "$BIN" ]; then
    cargo build --release --locked
fi

echo ">> assembling $APPDIR"
rm -rf "$APPDIR"
install -Dm755 "$BIN" "$APPDIR/usr/bin/redragon-gui"
mkdir -p "$APPDIR/usr/share/redragon-gui"
cp -r models "$APPDIR/usr/share/redragon-gui/models"
install -Dm644 assets/99-redragon-gui.rules "$APPDIR/usr/share/redragon-gui/99-redragon-gui.rules"
install -Dm644 assets/redragon-gui.desktop "$APPDIR/usr/share/applications/redragon-gui.desktop"
install -Dm644 assets/redragon-gui.desktop "$APPDIR/redragon-gui.desktop"
install -Dm644 assets/redragon-gui.metainfo.xml "$APPDIR/usr/share/metainfo/$APP_ID.appdata.xml"
install -Dm644 assets/redragon-gui.svg "$APPDIR/usr/share/icons/hicolor/scalable/apps/redragon-gui.svg"

# appimagetool wants a raster icon at the root and as .DirIcon.
if command -v rsvg-convert >/dev/null 2>&1; then
    rsvg-convert -w 256 -h 256 assets/redragon-gui.svg -o "$APPDIR/redragon-gui.png"
elif command -v magick >/dev/null 2>&1; then
    magick -background none -density 384 assets/redragon-gui.svg -resize 256x256 "$APPDIR/redragon-gui.png"
else
    echo "need rsvg-convert or ImageMagick to rasterise the icon" >&2
    exit 1
fi
install -Dm644 "$APPDIR/redragon-gui.png" "$APPDIR/usr/share/icons/hicolor/256x256/apps/redragon-gui.png"
ln -s redragon-gui.png "$APPDIR/.DirIcon"

cat >"$APPDIR/AppRun" <<'EOF'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
# Model descriptors travel inside the image; a user override still wins.
export REDRAGON_GUI_MODELS="${REDRAGON_GUI_MODELS:-$HERE/usr/share/redragon-gui/models}"
exec "$HERE/usr/bin/redragon-gui" "$@"
EOF
chmod 755 "$APPDIR/AppRun"

if [ -n "${APPIMAGETOOL:-}" ]; then
    TOOL="$APPIMAGETOOL"
else
    TOOL="$TOOLS/appimagetool-$ARCH.AppImage"
    if [ ! -x "$TOOL" ]; then
        echo ">> fetching appimagetool $APPIMAGETOOL_VERSION"
        mkdir -p "$TOOLS"
        curl -fsSL --retry 3 -o "$TOOL" \
            "https://github.com/AppImage/appimagetool/releases/download/$APPIMAGETOOL_VERSION/appimagetool-$ARCH.AppImage"
        expected_var="APPIMAGETOOL_SHA256_$ARCH"
        echo "${!expected_var}  $TOOL" | sha256sum --check --status || {
            echo "appimagetool checksum mismatch" >&2
            rm -f "$TOOL"
            exit 1
        }
        chmod +x "$TOOL"
    fi
fi

mkdir -p "$OUT"
IMAGE="$OUT/redragon-gui-$VERSION-$ARCH.AppImage"
echo ">> building $IMAGE"
# Extract-and-run avoids needing FUSE on CI runners and in containers.
ARCH="$ARCH" APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" --no-appstream "$APPDIR" "$IMAGE"
chmod +x "$IMAGE"
cp assets/99-redragon-gui.rules "$OUT/99-redragon-gui.rules"
(cd "$OUT" && sha256sum "$(basename "$IMAGE")" 99-redragon-gui.rules >SHA256SUMS)

echo ">> smoke test"
APPIMAGE_EXTRACT_AND_RUN=1 "$IMAGE" --version
echo ">> done: $IMAGE"
