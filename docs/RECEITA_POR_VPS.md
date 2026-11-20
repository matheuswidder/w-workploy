# Escrevendo a receita pensando na VPS que vai rodar

> A receita (`.workploy.yml`) **não roda onde você escreveu** — ela roda na máquina
> escolhida no **Vínculo** (Configurações → GitHub). Uma receita que funciona no seu
> Windows e falha na VPS, ou que funciona na VPS A e quebra na VPS B, quase sempre
> é uma receita que **assumiu um ambiente** em vez de **descobri-lo**.
>
> Este guia é o contrato entre a receita e a VPS: o que o runner garante, o que a
> receita precisa garantir sozinha e como descobrir o ambiente antes de escrever.

---

## 1. O contrato do ambiente

### 1.1 O que o Workploy garante para cada passo

Quando um passo roda na VPS (via `workploy-runner` 24/7 ou via SSH direto do desktop),
o Workploy entrega um ambiente **mínimo e previsível**:

| Variável | Valor | Por quê |
|---|---|---|
| `HOME` | `/root` (ou o `HOME` do serviço) | Sem `HOME`, `~` e `$HOME/.cargo/env` quebram — foi exatamente o erro `/.cargo/env: No such file or directory` que apareceu no log do CI do Oktopus. |
| `PATH` | `/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin` **primeiro**, depois `$HOME/.local/share/pnpm`, `/usr/local/share/pnpm`, `$HOME/.cargo/bin`, `/usr/local/cargo/bin`, e por fim o `PATH` herdado | Garante que `git`, `curl`, `dnf`/`apt-get`, `docker`, `node` e `pnpm` sejam encontrados **independentemente** do shell de login da VPS. |
| `WORKPLOY_WORKDIR` | Raiz do workspace do runner (ex.: `/var/lib/workploy/workspaces/<dono>-<repo>`) ou o `workdir` da receita | Permite que a receita encontre o código **sem chutar caminho**. |
| `WORKPLOY_REPO` | `dono/repo` do vínculo | Para logs e checagens de provenance. |
| `WORKPLOY_BRANCH` | Branch monitorada | Idem. |

> **Regra de ouro:** a receita **nunca** deve assumir `HOME`, `PATH` ou caminho de
> código. Ela deve **descobrir** e **normalizar**.

### 1.2 O que a receita precisa garantir sozinha

O runner garante o *ambiente*, mas **não garante as ferramentas**. O que a VPS tem
depende do que você instalou — e o SO importa:

| Tarefa | Debian / Ubuntu | RHEL / AlmaLinux / Rocky / Fedora |
|---|---|---|
| Atualizar índices | `apt-get update` | `dnf install -y <pkg>` |
| Instalar pacote | `apt-get install -y <pkg>` | `dnf install -y <pkg>` |
| Pacote de navegador | `google-chrome-stable` (deb oficial) | `google-chrome-stable` (rpm oficial) ou `chromium` (EPEL) |
| Libera o apt/dnf | `DEBIAN_FRONTEND=noninteractive` | — |

**A Recipe deve detectar o gerenciador, nunca assumir um.**

---

## 2. Passo 0 obrigatório: descubra a VPS antes de escrever

Nunca escreva a receita às cegas. Rode o **preflight** na VPS de destino — ele imprime
exatamente o que a receita precisa saber:

```bash
# Via SSH, usando o alias da VPS em ~/.ssh/config
ssh concef-vps 'bash -s' < scripts/vps-preflight.sh

# Ou direto na VPS
bash scripts/vps-preflight.sh
```

O relatório responde:

- **Qual é o SO** e qual gerenciador de pacotes existe (`apt-get`? `dnf`?).
- **Qual é o `HOME` e o `PATH` herdados** (e se `HOME` está vazio).
- **Quais ferramentas existem** (`git`, `docker`, `node`, `pnpm`, `cargo`, `curl`, `flock`).
- **Se o Docker responde** e se o plugin `docker compose` existe.
- **Se há navegador** (para testes de interface com Puppeteer).
- **Qual a versão do `workploy-runner`** (avisa se está desatualizado).
- **Quais raízes de código existem** na máquina: o `WORKPLOY_WORKDIR` e as cópias de
  produção em `/root/Projects/*` — isso decide se o deploy usa o workspace do runner
  ou uma cópia tradicional de produção.

Alternativa dentro do app: **Configurações → Conexões & Diagnóstico → Testar agora**
mostra SO, vCPU, RAM, disco e a lista de ferramentas detectadas.

### 2.1 Receita de diagnóstico pronta para importar

Para deixar o diagnóstico dentro do próprio Workploy (e versionado no repo), use
esta receita no `.workploy.yml` do seu projeto:

```yaml
workflows:
  - id: diagnostico-vps
    name: Diagnóstico da VPS
    on: [workflow_dispatch]
    jobs:
      build:
        runs-on: self-hosted
        steps:
          - name: "Relatório do ambiente"
            run: |
              set -e
              export HOME="${HOME:-/root}"
              export PATH="/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH"
              echo "::workploy-substep::Identificação do host"
              cat /etc/os-release 2>/dev/null | head -3 || uname -a
              echo "HOME=[$HOME]"
              echo "WORKPLOY_WORKDIR=[${WORKPLOY_WORKDIR:-vazio}]"
              echo "::workploy-substep::Ferramentas"
              for t in git curl docker node pnpm cargo chromium google-chrome; do
                command -v "$t" >/dev/null 2>&1 && echo "ok: $t" || echo "ausente: $t"
              done
```

---

## 3. Os 5 padrões que evitam 90% das falhas

### 3.1 Normalize `HOME` e `PATH` em **todo** passo

```bash
set -e
export HOME="${HOME:-/root}"
export PATH="/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH"
```

Repita nas duas primeiras linhas de cada passo. O Workploy já faz isso, mas o
`PATH` exportado **dentro do passo** é o que vale — se você faz `cd` para outro
diretório ou usa `env -i`, o contexto se perde.

### 3.2 Resolva a raiz do código dinamicamente

Nunca fixe `/root/Projects/meu-repo`. A mesma receita roda em máquinas diferentes:

```bash
REPO_ROOT=""
if [ -n "${WORKPLOY_WORKDIR:-}" ] && [ -f "$WORKPLOY_WORKDIR/infra/api/docker-compose.yml" ]; then
  REPO_ROOT="$WORKPLOY_WORKDIR"          # workspace do runner (já sincronizado)
elif [ -d /root/Projects/meu-repo ]; then
  REPO_ROOT="/root/Projects/meu-repo"    # cópia de produção
else
  echo "ERRO: raiz do repositório não encontrada." >&2
  exit 1
fi
cd "$REPO_ROOT"
```

A validação com um **arquivo marcador** (ex.: `infra/api/docker-compose.yml`) evita
"entrou no diretório errado" — o erro clássico de `configuration file provided: not found`.

### 3.3 Instale o navegador por detecção de SO

```bash
BROWSER=""
for c in google-chrome google-chrome-stable chromium chromium-browser; do
  command -v "$c" >/dev/null 2>&1 && { BROWSER="$(command -v "$c")"; break; }
done
if [ -z "$BROWSER" ]; then
  SUDO=""; [ "$(id -u)" -ne 0 ] && SUDO="sudo -n"
  if command -v dnf >/dev/null 2>&1; then
    curl -fsSL -o /tmp/chrome.rpm https://dl.google.com/linux/direct/google-chrome-stable_current_x86_64.rpm
    $SUDO dnf install -y /tmp/chrome.rpm; rm -f /tmp/chrome.rpm
  elif command -v apt-get >/dev/null 2>&1; then
    export DEBIAN_FRONTEND=noninteractive
    curl -fsSL -o /tmp/chrome.deb https://dl.google.com/linux/direct/google-chrome-stable_current_amd64.deb
    $SUDO apt-get install -y /tmp/chrome.deb; rm -f /tmp/chrome.deb
  fi
  for c in google-chrome google-chrome-stable chromium chromium-browser; do
    command -v "$c" >/dev/null 2>&1 && { BROWSER="$(command -v "$c")"; break; }
  done
fi
[ -z "$BROWSER" ] && { echo "ERRO: nenhum navegador disponível." >&2; exit 1; }
export CHROME_PATH="$BROWSER"
```

Repositórios com `puppeteer-core` leem `CHROME_PATH` e caem em
`/usr/bin/google-chrome`, `/usr/bin/chromium`, `/usr/bin/chromium-browser` — o `export`
torna o teste determinístico em qualquer distro.

### 3.4 Instale dependências do projeto com o gerenciador certo

| Necessidade | Debian/Ubuntu | RHEL/Alma |
|---|---|---|
| Node 22 | `nodesource` + `apt-get` | `nodesource` + `dnf` |
| pnpm | `corepack enable` | `corepack enable` |
| Docker | `docker.io` + plugin compose | `docker-ce` (repo oficial) |

Verifique **depois** de instalar (`node --version && pnpm --version && docker compose version`)
— instalar sem verificar é a forma mais rápida de descobrir a falha 20 minutos depois.

### 3.5 Use `nice` em VPS que também hospeda produção

A `widder-vps`/`CONCEF` têm produção no ar. O gate completo concorre com ela:

```bash
nice -n 10 pnpm typecheck
nice -n 10 node scripts/run-package-checks.mjs --concurrency=1
```

`--concurrency=1` evita suíte que estoura timeout por contenção de CPU.

---

## 4. Checklist antes de salvar a receita

- [ ] Rodei o **preflight** na VPS de destino e li o relatório.
- [ ] A receita detecta o gerenciador de pacotes em vez de assumir `apt-get`.
- [ ] Todo passo exporta `HOME` e `PATH` normalizados.
- [ ] A raiz do código é resolvida via `WORKPLOY_WORKDIR` com fallback — nenhum caminho fixo.
- [ ] As ferramentas necessárias (navegador, Node, pnpm, Docker) são **verificadas** (`--version`) e, se ausentes, **instaladas** ou o passo falha com mensagem clara.
- [ ] `docker compose version` responde (plugin v2, não o `docker-compose` v1).
- [ ] O passo destrutivo tem `x-approval: true`.
- [ ] A receita é **portátil entre as VPS do projeto** (CI e deploy podem rodar em máquinas diferentes).
- [ ] O `shell` daspas o bloco `run: |` e a indentação relativa está correta.
- [ ] Validei a sintaxe: `bash -n` em cada bloco e importei no Workploy sem erro.

---

## 5. Anti-padrões (o que já quebrou em produção)

| Anti-padrão | O que aconteceu | Correção |
|---|---|---|
| `apt-get install` em receita genérica | `apt-get: command not found` em AlmaLinux 9 | Detectar `dnf`/`apt-get` |
| Confiar em `HOME` | `/.cargo/env: No such file or directory` | `export HOME="${HOME:-/root}"` |
| `cd /root/Projects/repo` fixo | `No such file or directory` (cópia de produção não existe na VPS de CI) | Resolver via `WORKPLOY_WORKDIR` |
| Assumir `google-chrome` instalado | Testes de interface pulam ou falham em silêncio | Detectar + instalar por SO |
| `docker-compose` (v1) | `docker-compose: command not found` | Usar `docker compose` (plugin v2) |
| `git reset --hard origin/main` sem checar ancestralidade | Deploy pode reverter código | `git merge-base --is-ancestor` antes |
| Rebuild sem healthcheck | "Deploy passou" com serviço quebrado | Loop de healthcheck local + `docker logs` na falha |
| Assumir que `pnpm` está no PATH do serviço | `pnpm: command not found` | `PATH` com `~/.local/share/pnpm` e `corepack enable` |

---

## 6. Onde a VPS executa cada coisa

| Caminho | Significado |
|---|---|
| `/var/lib/workploy/workspaces/<dono>-<repo>` | Workspace do runner: clone do GitHub sincronizado com a branch/commit disparado. É o `WORKPLOY_WORKDIR`. |
| `/etc/workploy/config.json` | Receitas e vínculos sincronizados na VPS (via *Sincronizar na VPS*). |
| `/etc/workploy/github_token` | PAT usado pelo runner para `git clone/pull` e sondagem. |
| `/etc/workploy/env` | Arquivo opcional de segredos do ambiente (ex.: `CLOUDFLARE_API_TOKEN`). A receita pode carregá-lo com `set -a; . /etc/workploy/env; set +a`. |
| `/var/lib/workploy/runs.json` | Histórico de execuções do runner (espelhado no desktop). |
| `/opt/workploy/workploy_runner.py` | O daemon. **Reprovisione** a VPS após atualizar o Workploy para receber a versão nova. |

---

## 7. Receitas por tipo de VPS

| Perfil da VPS | Foco |
|---|---|
| **Leve** (1–2 vCPU, 2 GB) | Só sondagem/webhook e scripts curtos; evite builds pesados. |
| **Padrão** (2–4 vCPU, 4–8 GB) | Monorepo Node/TypeScript, Docker e CI contínuo. `nice -n 10` e `--concurrency=1` se há produção no ar. |
| **Pesada** (4–8 vCPU, 8–16 GB) | Builds Rust/Android paralelos e múltiplos containers. |

---

## 8. Referência rápida

```bash
# Diagnóstico completo
ssh <alias-vps> 'bash -s' < scripts/vps-preflight.sh

# Validar a sintaxe de um bloco shell antes de salvar a receita
bash -n bloco.sh

# Ver o que o runner enxerga
cat /etc/workploy/config.json | head -40
systemctl status workploy-runner
journalctl -u workploy-runner -n 50 --no-pager
```

**Guias relacionadas:** [`AUTOMACOES_YAML.md`](AUTOMACOES_YAML.md) (gramática completa da
receita, encadeamento, aprovações e importação) e o
[`README.md`](../README.md) (visão geral do produto).
