#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:-latest}"
REPO="${REPO:-csr91/mdnav}"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
TMP_DIR="$(mktemp -d)"
ARCHIVE_PATH="$TMP_DIR/mdnav-linux-x86_64.tar.gz"

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

if [[ "$VERSION" == "latest" ]]; then
  RELEASE_API="https://api.github.com/repos/$REPO/releases/latest"
else
  RELEASE_API="https://api.github.com/repos/$REPO/releases/tags/$VERSION"
fi

DOWNLOAD_URL="$(curl -fsSL "$RELEASE_API" | grep browser_download_url | grep mdnav-linux-x86_64.tar.gz | cut -d '"' -f 4 | head -n 1)"

if [[ -z "$DOWNLOAD_URL" ]]; then
  echo "No se encontro el asset mdnav-linux-x86_64.tar.gz en la release solicitada." >&2
  exit 1
fi

mkdir -p "$INSTALL_DIR"
curl -fsSL "$DOWNLOAD_URL" -o "$ARCHIVE_PATH"
tar -xzf "$ARCHIVE_PATH" -C "$TMP_DIR"
cp "$TMP_DIR/mdnav" "$INSTALL_DIR/mdnav"
chmod +x "$INSTALL_DIR/mdnav"

case ":$PATH:" in
  *":$INSTALL_DIR:"*)
    echo "$INSTALL_DIR ya esta en PATH."
    ;;
  *)
    echo "mdnav instalado en $INSTALL_DIR"
    echo "Agrega esto a tu shell si queres usarlo globalmente:"
    echo "export PATH=\"$INSTALL_DIR:\$PATH\""
    ;;
esac

echo "Instalacion lista."
echo
echo "Para habilitar cd automatico con Shift+G:"
echo "  bash: echo 'source <(mdnav --shell-hook bash)' >> ~/.bashrc"
echo "  zsh:  echo 'source <(mdnav --shell-hook zsh)' >> ~/.zshrc"
