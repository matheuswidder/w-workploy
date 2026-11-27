#!/usr/bin/env bash
# Preflight do Workploy: o que a receita PRECISA saber antes de rodar na VPS.
#
# Uso na VPS (via SSH, sem depender de nada instalado):
#   curl -fsSL https://raw.githubusercontent.com/matheuswidder/w-workploy/main/scripts/vps-preflight.sh | bash
#
# Ou dentro de um passo de receita (o jeito mais confiável):
#   bash scripts/vps-preflight.sh
#
# Saída: um relatório em texto que pode ser colado numa issue ou no chat para
# definir a receita com precisão (SO, gerenciador de pacotes, ferramentas,
# raízes de código disponíveis e o ambiente herdado do runner).

set -u

echo "=== Workploy · Preflight da VPS ==="
echo "data: $(date -Is 2>/dev/null || date)"
echo "host: $(hostname 2>/dev/null || echo '?')"
echo

echo "--- Sistema operacional ---"
if [ -r /etc/os-release ]; then
  # shellcheck disable=SC1091
  . /etc/os-release
  echo "id: ${ID:-?}  nome: ${PRETTY_NAME:-?}  versao: ${VERSION_ID:-?}"
else
  echo "id: ? (sem /etc/os-release)"
fi
echo "kernel: $(uname -srm 2>/dev/null || echo '?')"
echo "arquitetura: $(uname -m 2>/dev/null || echo '?')"
echo

echo "--- Gerenciador de pacotes (define como instalar) ---"
for pm in apt-get apt dnf yum microdnf zypper apk pacman; do
  if command -v "$pm" >/dev/null 2>&1; then
    echo "disponivel: $pm ($(command -v "$pm"))"
  fi
done
echo

echo "--- Ambiente herdado (HOME/PATH) ---"
echo "HOME=[${HOME:-<vazio>}]"
echo "PATH=[${PATH:-<vazio>}]"
echo "SHELL=[${SHELL:-<vazio>}]"
echo "user: $(id -un 2>/dev/null || echo '?') (uid=$(id -u 2>/dev/null || echo '?'))"
if [ -z "${HOME:-}" ]; then
  echo "AVISO: HOME vazio — comandos com ~ (ex.: ~/.cargo/env) vao quebrar."
fi
echo

echo "--- Ferramentas de build ---"
for tool in git curl wget jq flock tar unzip corepack docker node npm pnpm npx yarn bun cargo rustc python3 make gcc; do
  if command -v "$tool" >/dev/null 2>&1; then
    ver=$("$tool" --version 2>/dev/null | head -1)
    echo "ok: $tool -> $(command -v "$tool") ${ver:+($ver)}"
  else
    echo "AUSENTE: $tool"
  fi
done
echo

echo "--- Docker ---"
if command -v docker >/dev/null 2>&1; then
  docker --version 2>/dev/null || true
  if docker compose version >/dev/null 2>&1; then
    echo "ok: docker compose v2 ($(docker compose version --short 2>/dev/null))"
  else
    echo "AUSENTE: plugin docker compose (necessario para 'docker compose')"
  fi
  if docker info >/dev/null 2>&1; then
    echo "ok: daemon docker responde"
  else
    echo "ERRO: daemon docker nao responde (permissao/root/socket?)"
  fi
fi
echo

echo "--- Navegador (necessario para testes de interface) ---"
for b in google-chrome google-chrome-stable chromium chromium-browser chrome; do
  if command -v "$b" >/dev/null 2>&1; then
    echo "ok: $b -> $(command -v "$b") ($("$b" --version 2>/dev/null | head -1))"
  fi
done
if [ -z "$(command -v google-chrome || command -v google-chrome-stable || command -v chromium || command -v chromium-browser 2>/dev/null)" ]; then
  echo "AUSENTE: nenhum navegador instalado (use CHROME_PATH na receita)"
fi
echo

echo "--- Workploy runner ---"
if [ -f /opt/workploy/workploy_runner.py ]; then
  echo "ok: /opt/workploy/workploy_runner.py presente"
  if grep -q "_build_step_env" /opt/workploy/workploy_runner.py 2>/dev/null; then
    echo "ok: runner com ambiente robusto (HOME/PATH/WORKPLOY_WORKDIR)"
  else
    echo "AVISO: runner antigo (sem _build_step_env) — reprovisione a VPS"
  fi
else
  echo "AUSENTE: workploy-runner nao instalado nesta VPS"
fi
if command -v systemctl >/dev/null 2>&1; then
  echo "servico: $(systemctl is-active workploy-runner 2>/dev/null || echo '?')"
fi
for f in /etc/workploy/config.json /etc/workploy/github_token /etc/workploy/env; do
  if [ -e "$f" ]; then echo "presente: $f"; else echo "ausente: $f"; fi
done
echo

echo "--- Raizes de codigo disponiveis ---"
echo "WORKPLOY_WORKDIR=[${WORKPLOY_WORKDIR:-<vazio>}]"
echo "WORKPLOY_REPO=[${WORKPLOY_REPO:-<vazio>}]"
echo "WORKPLOY_BRANCH=[${WORKPLOY_BRANCH:-<vazio>}]"
if [ -n "${WORKPLOY_WORKDIR:-}" ] && [ -d "${WORKPLOY_WORKDIR}" ]; then
  echo "workspace do runner: ${WORKPLOY_WORKDIR}"
  (cd "$WORKPLOY_WORKDIR" && git log -1 --format='  HEAD: %h %s' 2>/dev/null) || true
fi
for candidate in /root/Projects/*; do
  [ -d "$candidate" ] || continue
  echo "copia de producao: ${candidate}"
  (cd "$candidate" && git log -1 --format='  HEAD: %h %s' 2>/dev/null) || true
done
echo

echo "--- Recursos ---"
free -h 2>/dev/null || true
df -h / 2>/dev/null | tail -1 || true
nproc 2>/dev/null | sed 's/^/vCPUs: /' || true
uptime 2>/dev/null || true
echo
echo "=== Preflight concluido ==="
