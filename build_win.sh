#!/bin/bash
set -euo pipefail

export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"
export NVM_DIR="$HOME/.nvm"
[ -s "$NVM_DIR/nvm.sh" ] && \. "$NVM_DIR/nvm.sh"

if command -v nvm >/dev/null 2>&1; then
  nvm use 20.20.0 >/dev/null 2>&1 || nvm use stable >/dev/null 2>&1 || true
fi

# Cache Windows SDK/CRT for cargo-xwin to avoid repeated downloads.
export XWIN_CACHE_DIR="$HOME/.cache/xwin"
mkdir -p "$XWIN_CACHE_DIR"

# Use the native macOS makensis extracted locally.
export NSISDIR="$HOME/.local/opt/makensis/3.12/share/nsis"
MAKENSIS_BIN="${MAKENSIS_BIN:-makensis}"

TARGET_DIR="src-tauri/target/x86_64-pc-windows-msvc/release"
NSIS_SCRIPT_DIR="$TARGET_DIR/nsis/x64"
BUNDLE_DIR="$TARGET_DIR/bundle/nsis"
NSIS_OUTPUT="$NSIS_SCRIPT_DIR/nsis-output.exe"
PRODUCT_EXE="$TARGET_DIR/liaoyitong-print-bridge.exe"
CRATE_EXE="$TARGET_DIR/lyt_print_bridge.exe"
SETUP_EXE="$BUNDLE_DIR/liaoyitong-print-bridge_0.1.0_x64-setup.exe"
LEGACY_SETUP_EXE="$BUNDLE_DIR/lyt_print_bridge_0.1.0_x64-setup.exe"

assert_no_task_dialog_import() {
  if command -v objdump >/dev/null 2>&1 && [ -f "$PRODUCT_EXE" ]; then
    if objdump -p "$PRODUCT_EXE" | grep -q "TaskDialogIndirect"; then
      echo "Windows executable still imports TaskDialogIndirect; aborting unsafe package."
      exit 1
    fi
  fi
}

echo "Current Node Version:"
node -v

echo "XWIN_CACHE_DIR: $XWIN_CACHE_DIR"
echo "NSISDIR: $NSISDIR"
echo "Starting Windows Build..."

rm -f "$NSIS_OUTPUT" "$SETUP_EXE" "$LEGACY_SETUP_EXE"

if npm run build:win; then
  assert_no_task_dialog_import
  exit 0
fi

echo "Tauri bundling failed, trying native makensis fallback..."

if [ ! -f "$NSIS_SCRIPT_DIR/installer.nsi" ]; then
  echo "Missing NSIS script: $NSIS_SCRIPT_DIR/installer.nsi"
  exit 1
fi

if ! command -v "$MAKENSIS_BIN" >/dev/null 2>&1; then
  echo "makensis is not installed or not in PATH: $MAKENSIS_BIN"
  exit 1
fi

if [ -f "$CRATE_EXE" ]; then
  REBUILT_EXE="$CRATE_EXE"
elif [ -f "$PRODUCT_EXE" ]; then
  REBUILT_EXE="$PRODUCT_EXE"
else
  echo "Missing rebuilt executable: $CRATE_EXE or $PRODUCT_EXE"
  exit 1
fi

echo "Using rebuilt executable from cargo-xwin:"
echo "  $REBUILT_EXE"
if [ "$REBUILT_EXE" != "$PRODUCT_EXE" ]; then
  cp "$REBUILT_EXE" "$PRODUCT_EXE"
fi

assert_no_task_dialog_import

mkdir -p "$BUNDLE_DIR"
(
  cd "$NSIS_SCRIPT_DIR" && \
  "$MAKENSIS_BIN" installer.nsi
)

if [ ! -f "$NSIS_OUTPUT" ]; then
  echo "NSIS fallback did not produce $NSIS_OUTPUT"
  exit 1
fi

cp "$NSIS_OUTPUT" "$SETUP_EXE"
cp "$NSIS_OUTPUT" "$LEGACY_SETUP_EXE"

echo "Windows installer created:"
echo "  $SETUP_EXE"
