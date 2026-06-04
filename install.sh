#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:-latest}"
REPO="${REPO:-csr91/mdnav}"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
TMP_DIR="$(mktemp -d)"
ARCHIVE_PATH="$TMP_DIR/mdnav-linux-x86_64.tar.gz"
SHELL_NAME="$(basename "${SHELL:-}")"
RC_FILE=""

if [[ "$SHELL_NAME" == "bash" || "$SHELL_NAME" == "zsh" ]]; then
  RC_FILE="$HOME/.${SHELL_NAME}rc"
fi

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
    if [[ -n "$RC_FILE" ]]; then
      touch "$RC_FILE"
      if grep -Fq "$INSTALL_DIR" "$RC_FILE"; then
        echo "$RC_FILE ya contiene $INSTALL_DIR."
      else
        {
          echo
          echo "# mdnav"
          echo "export PATH=\"$INSTALL_DIR:\$PATH\""
        } >> "$RC_FILE"
        echo "Se agrego $INSTALL_DIR al PATH en $RC_FILE."
      fi
      echo "Para usarlo en esta terminal ejecuta:"
      echo "export PATH=\"$INSTALL_DIR:\$PATH\""
    else
      echo "Agrega esto a tu shell si queres usarlo globalmente:"
      echo "export PATH=\"$INSTALL_DIR:\$PATH\""
    fi
    ;;
esac

echo "Instalacion lista."
echo

if [[ -n "$RC_FILE" ]]; then
  if [ -t 0 ]; then
    read -r -p "Instalar shell hook para $SHELL_NAME (cd automatico con Shift+G)? [s/N] " resp
  else
    read -r -p "Instalar shell hook para $SHELL_NAME (cd automatico con Shift+G)? [s/N] " resp </dev/tty 2>/dev/null || resp="n"
  fi
  if [[ "$resp" == "s" || "$resp" == "S" ]]; then
    HOOK_LINE="source <(\"$INSTALL_DIR/mdnav\" --shell-hook $SHELL_NAME)"
    if grep -Fq "$HOOK_LINE" "$RC_FILE"; then
      echo "El hook ya estaba instalado en $RC_FILE."
    else
      echo "$HOOK_LINE" >> "$RC_FILE"
      echo "Hook instalado en $RC_FILE. Abri una nueva terminal para activarlo."
    fi
  fi
else
  echo "Para habilitar cd automatico con Shift+G:"
  echo "  bash: echo 'source <(\"$INSTALL_DIR/mdnav\" --shell-hook bash)' >> ~/.bashrc"
  echo "  zsh:  echo 'source <(\"$INSTALL_DIR/mdnav\" --shell-hook zsh)' >> ~/.zshrc"
fi
