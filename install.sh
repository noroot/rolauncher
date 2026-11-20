#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Builds RoLauncher.app and installs it into /Applications (or $INSTALL_DIR).
#
#   ./install.sh               build, install, launch
#   ./install.sh --no-launch   build and install only
set -euo pipefail
cd "$(dirname "$0")"

APP_NAME=RoLauncher.app
INSTALL_DIR=${INSTALL_DIR:-/Applications}
DEST="$INSTALL_DIR/$APP_NAME"
LAUNCH=1

for arg in "$@"; do
    case "$arg" in
        --no-launch) LAUNCH=0 ;;
        -h|--help) sed -n '3,6p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "Unknown option: $arg" >&2; exit 1 ;;
    esac
done

./bundle.sh

# Use sudo only if the install directory isn't writable by the current user.
SUDO=""
if [[ ! -w "$INSTALL_DIR" ]] || { [[ -e "$DEST" ]] && [[ ! -w "$DEST" ]]; }; then
    echo "$INSTALL_DIR is not writable; using sudo."
    SUDO=sudo
fi

# Quit a running copy so the bundle can be replaced cleanly.
if pgrep -x rolauncher >/dev/null; then
    echo "Quitting running RoLauncher..."
    pkill -x rolauncher || true
    for _ in {1..20}; do pgrep -x rolauncher >/dev/null || break; sleep 0.1; done
fi

echo "Installing to $DEST"
$SUDO rm -rf "$DEST"
$SUDO ditto "target/release/$APP_NAME" "$DEST"

# Make Spotlight/Launchpad/Finder pick up the new bundle right away.
LSREGISTER=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
[[ -x "$LSREGISTER" ]] && "$LSREGISTER" -f "$DEST" || true

if [[ $LAUNCH -eq 1 ]]; then
    open "$DEST"
    echo "RoLauncher is running. Press ⌘Space to open it."
else
    echo "Installed. Launch with: open \"$DEST\""
fi
