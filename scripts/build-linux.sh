#!/usr/bin/env bash
# Build do artefato Linux do Workploy (x86_64) usando Docker.
#
# Usado quando o GitHub Actions não pode buildar (limite de conta da conta) —
# o artefato final é o mesmo `workploy-<versão>-linux-x86_64.tar.gz` da release.
#
# Uso (na raiz do repositório):
#   bash scripts/build-linux.sh
#
# Requisitos: Docker com suporte a linux/amd64 e o submodule `vendor/rune`
# já inicializado na árvore de trabalho.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' app/Cargo.toml | head -1)"
if [ -z "$VERSION" ]; then
  echo "ERRO: versão não encontrada em app/Cargo.toml" >&2
  exit 1
fi

echo "==> Workploy v$VERSION · build Linux via Docker"
docker build --platform linux/amd64 \
  -f scripts/Dockerfile.linux \
  -t "workploy-linux:$VERSION" \
  --target builder \
  .

echo "==> Extraindo o binário"
CONTAINER="$(docker create "workploy-linux:$VERSION")"
trap 'docker rm "$CONTAINER" >/dev/null 2>&1 || true' EXIT
mkdir -p "dist/linux-build"
docker cp "$CONTAINER:/out/workploy" "dist/linux-build/workploy"
chmod +x "dist/linux-build/workploy"

PKG_DIR="dist/workploy-${VERSION}-linux-x86_64"
rm -rf "$PKG_DIR"
mkdir -p "$PKG_DIR"
cp "dist/linux-build/workploy" "$PKG_DIR/workploy"
cp app/assets/icon.png "$PKG_DIR/workploy.png"

cat > "$PKG_DIR/workploy.desktop" <<'DESKTOP'
[Desktop Entry]
Name=Workploy
Comment=Orquestrador desktop para CI/CD e Deploy self-hosted multi-VPS
Exec=%h/.local/bin/workploy
Icon=workploy
Terminal=false
Type=Application
Categories=Development;Utility;
StartupWMClass=workploy
DESKTOP

cat > "$PKG_DIR/install.sh" <<'INSTALL'
#!/usr/bin/env bash
set -euo pipefail
DIR="$(cd "$(dirname "$0")" && pwd)"
mkdir -p "$HOME/.local/bin" "$HOME/.local/share/applications" "$HOME/.local/share/icons/hicolor/512x512/apps"
install -m 755 "$DIR/workploy" "$HOME/.local/bin/workploy"
install -m 644 "$DIR/workploy.png" "$HOME/.local/share/icons/hicolor/512x512/apps/workploy.png"
sed "s|%h|$HOME|g" "$DIR/workploy.desktop" > "$HOME/.local/share/applications/workploy.desktop"
chmod 644 "$HOME/.local/share/applications/workploy.desktop"
echo "Workploy instalado em $HOME/.local/bin/workploy"
echo "Certifique-se de que $HOME/.local/bin esta no seu PATH."
INSTALL
chmod +x "$PKG_DIR/install.sh"

cat > "$PKG_DIR/uninstall.sh" <<'UNINSTALL'
#!/usr/bin/env bash
set -euo pipefail
rm -f "$HOME/.local/bin/workploy" \
      "$HOME/.local/share/applications/workploy.desktop" \
      "$HOME/.local/share/icons/hicolor/512x512/apps/workploy.png"
echo "Workploy removido."
UNINSTALL
chmod +x "$PKG_DIR/uninstall.sh"

TAR_NAME="dist/workploy-${VERSION}-linux-x86_64.tar.gz"
tar -czf "$TAR_NAME" -C dist "workploy-${VERSION}-linux-x86_64"
( cd dist && sha256sum "$(basename "$TAR_NAME")" > "$(basename "$TAR_NAME").sha256" )

echo ""
echo "==> Artefatos gerados:"
ls -la "$TAR_NAME" "$TAR_NAME.sha256"
cat "$TAR_NAME.sha256"
