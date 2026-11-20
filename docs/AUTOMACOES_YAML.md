# Guia de Automações `.yml` no Workploy

Este guia explica como criar, estruturar, importar e diagnosticar automações `.yml` no Workploy v0.1.12.

> **Antes de escrever a receita, leia [`RECEITA_POR_VPS.md`](RECEITA_POR_VPS.md).**
> A receita roda na VPS escolhida no vínculo, não na sua máquina: o guia traz o
> contrato de ambiente (`HOME`, `PATH`, `WORKPLOY_WORKDIR`), o **preflight** para
> diagnosticar a VPS de destino (`scripts/vps-preflight.sh`) e os padrões de
> portabilidade entre Debian e RHEL.

## 1. Conceito central: Receita vs. Vínculo

O Workploy separa o fluxo em duas partes independentes para que você reaproveite a mesma receita em projetos e servidores diferentes.

| Conceito | O que define | Onde configurar |
|---|---|---|
| **Receita** (`Workflow`) | **O que** executar: nome, gatilho base, pasta de trabalho, jobs e passos, portões de aprovação e fluxo seguinte (`on_success`). | Barra lateral agrupada por projeto (`Novo fluxo`) ou modal **Importar YAML** / **Arquivo YAML**. |
| **Vínculo** (`RepoBinding`) | **Onde** e **quando** rodar: repositório GitHub (`dono/nome`), branches monitoradas, destino (`Local` ou uma `VPS`) e modo de disparo. | **Configurações → GitHub** (botão **Vincular** ou **+ Novo vínculo**). |

### Onde a automação roda

- **Local (Esta máquina):** o aplicativo desktop executa os comandos no seu computador. No Windows, cada passo roda via `cmd /C` (`app/src/engine.rs`); no Linux, via `sh -c`. Indique este modo para builds nativos Windows, testes locais e pasta vigiada.
- **VPS Linux (`workploy-runner`):** o serviço `systemd` provisionado na VPS clona ou atualiza o repositório em `/var/lib/workploy/workspaces/<slug>` (ou usa o `workdir` informado) e executa cada passo via `/bin/bash -lc` (ou `/bin/sh -lc`). Seus pipelines seguem ativos com o computador desligado.

### Quando o vínculo dispara

| Modo de disparo | Onde monitora | Requer PC ligado | Quando escolher |
|---|---|---|---|
| **Webhook 24/7 na VPS** | Endpoint HTTP na VPS (`POST /webhook/github`) | Não | Resposta imediata a cada `git push` com validação HMAC (`/etc/workploy/webhook_secret`). |
| **Sondagem 24/7 na VPS** | Loop contínuo do `workploy-runner` na VPS (intervalo mínimo de 15 s) | Não | VPS atrás de firewall ou sem porta HTTP pública exposta para o GitHub. |
| **Sondagem no Desktop** | Thread do aplicativo desktop (`spawn_github_poller`) | Sim | Repositórios que rodam na sua máquina local ou testes rápidos sem VPS. |

### Filtro multi-branch (`*`, `main` ou `main,dev`)

No campo **Branch monitorada** do vínculo, você controla quais branches disparam o fluxo:

- `main` (ou outro nome único): monitora apenas aquela branch.
- `main,dev` (lista separada por vírgula): monitora cada branch listada de forma independente e dispara a receita quando qualquer uma recebe novos commits.
- `*`: monitora todas as branches do repositório (clique no chip **Todas (*)** no editor de vínculo).

---

## 2. Estrutura do arquivo `.workploy.yml`

Um arquivo `.workploy.yml` descreve uma receita completa com os campos abaixo:

```yaml
name: Nome do fluxo
trigger: push
workdir: /srv/meu-app
on_success: id-do-proximo-fluxo

steps:
  - name: Nome da etapa
    run: comando shell
  - name: Etapa crítica
    run: docker compose up -d --build
    approval: true
```

### Campos suportados

| Campo | Obrigatório | Valores / Tipo | Função |
|---|---|---|---|
| `name` | Sim | Texto | Nome exibido na barra lateral, no **Dashboard do Projeto**, no histórico de **Execuções** e nos vínculos. |
| `trigger` | Não (padrão `manual`) | `manual`, `push`, `watch` | `manual` roda ao clicar em **Executar agora** ou por encadeamento; `push` responde a atualizações do GitHub; `watch` dispara ao detectar mudanças na pasta local configurada em **Configurações → GitHub**. |
| `workdir` | Não | Caminho absoluto ou relativo | Diretório onde os comandos rodam (ex: `/srv/app` na VPS ou `C:\Projects\app` no Windows). Quando vazio na VPS, o `workploy-runner` usa `/var/lib/workploy/workspaces/<repo>`. |
| `on_success` | Não | ID de outro fluxo | Inicia a receita indicada quando todos os passos atuais terminam com sucesso. |
| `steps` | Sim | Lista (mínimo 1 item) | Etapas executadas em ordem. Se um passo retorna código diferente de zero, os seguintes são ignorados (`Ignorado`). |
| `steps[].name` | Não | Texto | Título curto exibido na linha do tempo e nos logs. Se omitido, vira `Passo N`. |
| `steps[].run` | Sim | String ou bloco multilinha (`\|`) | Comando shell da etapa. Use `\|` para scripts com várias linhas. |
| `steps[].approval` | Não (padrão `false`) | `true` ou `false` | Pausa a execução antes deste passo com o status **Aguardando aprovação** e exibe o cartão na aba **Aprovações**. |

> **Nota sobre o parser YAML (`app/src/yaml.rs`):** o modal **Importar YAML** / **Arquivo YAML** lê e exporta o formato compatível com GitHub Actions simplificado:
> - `on: [push]` ou `on: [workflow_dispatch]` no lugar de `trigger`
> - `jobs.<id>.runs-on: local` ou `self-hosted` para definir o runner padrão
> - `x-approval: true` dentro do passo no lugar de `approval: true`
> - `x-chain: <id-do-fluxo>` na raiz no lugar de `on_success`
> - `x-server: <id-da-vps>` e `x-repo: <id-do-vinculo>` para fixar servidor e vínculo na exportação
>
> Campos como `workdir` e o gatilho `Mudança na pasta` (`watch`) ficam disponíveis no formulário **Editar fluxo** após importar.

---

## 3. Importação de `.github/workflows/*.yml` e Jobs com Sub-etapas

Na versão `0.1.10`, o Workploy importa workflows existentes de `.github/workflows/*.yml` (e `.yaml`) direto pelo botão **Buscar fluxos no repo** ou pelo modal **Importar YAML**.

### Como o Workploy mapeia workflows do GitHub Actions

1. **Descoberta automática:** ao escanear o repositório (`app/src/github.rs`), o Workploy lista todos os arquivos `.yml` e `.yaml` em `.github/workflows/` além de manifestos na raiz (`package.json`, `Cargo.toml`, `docker-compose.yml`, `Dockerfile`).
2. **Mapeamento de jobs e concorrência:**
   - `on: [push]` vira gatilho automático por push; `on: [workflow_dispatch]` vira disparo manual.
   - `concurrency.cancel-in-progress: true` instrui o `workploy-runner` a cancelar runs anteriores ainda em andamento na mesma branch quando um novo commit chega.
   - `jobs.<id>.runs-on` define o destino (`local` para a máquina desktop, `self-hosted` para a VPS Linux).
   - Cada job é exibido na tela de execução agrupado por tipo, como no GitHub Actions: `Typecheck + Tests + Smoke`, `Secret scan`, `Contracts Zod` e `Deploy VPS`.
3. **Live Logs (`stick_bottom`):** durante a execução, o painel de detalhes rola automaticamente (`stick_bottom`) para a última linha emitida e mantém a sub-etapa ativa visível.

### Como estruturar Jobs com Sub-etapas (`::workploy-substep::<Nome>`)

Dentro de um job ou passo `run`, emita marcadores `::workploy-substep::<Nome>` na saída padrão (`stdout`) para dividir um script longo em sub-etapas expansíveis e cronometradas na tela de execução:

```text
::workploy-substep::Instalar dependências
```

Quando o `workploy-runner` ou o executor local lê uma linha com esse prefixo:

- Fecha a sub-etapa anterior com o tempo decorrido e status (`Concluído` ou `Falhou`).
- Abre uma nova sub-etapa expansível com `<Nome>` na árvore do job pai.
- Direciona as linhas seguintes de log para essa sub-etapa mantendo o acompanhamento ao vivo (`stick_bottom`).

### Exemplo completo em `.github/workflows/ci-deploy.yml`

Este exemplo agrupa os quatro tipos de job na tela de execução (`Typecheck + Tests + Smoke`, `Secret scan`, `Contracts Zod` e `Deploy VPS`) e usa `::workploy-substep::<Nome>` dentro de cada job:

```yaml
name: CI e Deploy VPS
on: [push]

concurrency:
  group: workploy-${{ github.ref }}
  cancel-in-progress: true

jobs:
  typecheck-tests-smoke:
    name: Typecheck + Tests + Smoke
    runs-on: self-hosted
    steps:
      - name: Typecheck + Tests + Smoke
        run: |
          echo "::workploy-substep::Instalar dependências"
          pnpm install --frozen-lockfile

          echo "::workploy-substep::Typecheck em todos os pacotes"
          pnpm -r typecheck

          echo "::workploy-substep::Testes unitários e integração"
          pnpm test:ci

          echo "::workploy-substep::Readiness smoke"
          node scripts/readiness-smoke.mjs

  secret-scan:
    name: Secret scan
    runs-on: self-hosted
    steps:
      - name: Secret scan
        run: |
          echo "::workploy-substep::Gitleaks no repositório"
          gitleaks detect --source . --no-banner

          echo "::workploy-substep::Auditoria de dependências (high+)"
          pnpm audit --audit-level high

  contracts-zod:
    name: Contracts Zod
    runs-on: self-hosted
    steps:
      - name: Contracts Zod
        run: |
          echo "::workploy-substep::Validar schemas Zod"
          pnpm --filter contracts typecheck

          echo "::workploy-substep::Testar compatibilidade de payloads"
          pnpm --filter contracts test

  deploy-vps:
    name: Deploy VPS
    needs: [typecheck-tests-smoke, secret-scan, contracts-zod]
    runs-on: self-hosted
    steps:
      - name: Deploy VPS
        x-approval: true
        run: |
          echo "::workploy-substep::Atualizar código e construir imagens"
          git fetch origin && git pull --ff-only
          docker compose -f infra/api/docker-compose.yml build

          echo "::workploy-substep::Subir containers e validar healthcheck"
          docker compose -f infra/api/docker-compose.yml up -d --remove-orphans
          curl -fsS http://127.0.0.1:3001/healthz
```

> **Dica:** use nomes curtos e claros depois de `::workploy-substep::` (como `Instalar dependências`, `Typecheck`, `Build Docker` e `Healthcheck`) para facilitar a leitura na lista recolhível e no filtro de busca dos logs.

---

## 4. Importar do repositório (fonte da verdade)

Além de colar o YAML no modal **Importar YAML** ou converter workflows de `.github/workflows/`, você pode versionar as receitas no próprio repositório. O arquivo commitado no repo vira a fonte da verdade: o Workploy importa dele, registra de onde cada receita veio e avisa quando o arquivo muda.

### Onde o Workploy procura os ymls

| Local no repo | Formato aceito | Quando usar |
|---|---|---|
| `.workploy.yml` (ou `.workploy.yaml`) na raiz | Uma receita única, com os campos da seção 2 direto na raiz do arquivo, ou uma lista `workflows:` com várias receitas no mesmo arquivo | Quando a automação descreve o próprio projeto e pode viver junto ao código. |
| `infra/workploy/*.yml` | Uma receita por arquivo, com os mesmos campos da seção 2 | Convenção usada pelo `w-erp-central` para manter cada pipeline em um arquivo próprio, perto do restante de `infra/`. |

Na lista `workflows:`, cada item usa os mesmos campos da seção 2 e aceita um `id` opcional, que serve de referência para o `on_success` de outra receita do mesmo arquivo. Workflows de `.github/workflows/*.yml` continuam pelo botão **Buscar fluxos no repo**, descrito na seção 3.

### Passo a passo da importação

1. **Commite o yml no repo:** escreva o `.workploy.yml` na raiz (ou um arquivo por receita em `infra/workploy/`) e commite na branch monitorada.
2. **Vincule o repositório:** em **Configurações → GitHub**, clique em **Vincular** na linha do repositório e configure branch monitorada, destino e modo de disparo.
3. **Clique em Importar do repositório:** o app lista os ymls encontrados na raiz e em `infra/workploy/`; em um arquivo com lista `workflows:`, cada item aparece como uma receita própria.
4. **Revise o preview e confirme:** antes de importar, o app mostra um preview com os passos de cada receita. Confirme para criar os fluxos.

### Proveniência e atualização

Toda receita importada do repo registra a proveniência:

- **Repo:** repositório de origem (`dono/nome`).
- **Caminho:** local do arquivo no repo (ex.: `.workploy.yml` ou `infra/workploy/deploy-api.yml`).
- **SHA:** commit lido no momento da importação.

Com a proveniência registrada, o fluxo passa a oferecer **Atualizar do repo**: um clique relê o arquivo no repo e atualiza os passos do fluxo. Se o yml mudou no repo desde a importação, a tela da execução exibe o aviso **"A receita mudou no repo"**. Ao vê-lo, clique em **Atualizar do repo** para trazer a receita vigente antes de rodar de novo.

> **Boa prática:** com o repo como fonte da verdade, mude as receitas no arquivo commitado e atualize pelo botão. Edições feitas só no app ficam fora do arquivo; use o repo como ponto único de mudança para não perder a sincronia.

### Exemplo completo pronto para copiar

CI com testes reais, deploy com portão de aprovação e encadeamento via `on_success`, em um único `.workploy.yml` na raiz:

```yaml
workflows:
  - id: wf-ci-principal
    name: CI Principal
    trigger: push
    on_success: wf-deploy-producao

    steps:
      - name: Instalar dependências
        run: pnpm install --frozen-lockfile

      - name: Checagem de tipos em todos os pacotes
        run: pnpm -r typecheck

      - name: Testes unitários e de integração
        run: pnpm test:ci

      - name: Varredura de segredos
        run: gitleaks detect --source . --no-banner

  - id: wf-deploy-producao
    name: Deploy Produção
    trigger: manual
    workdir: /srv/erp-producao

    steps:
      - name: Atualizar código na VPS
        run: git fetch origin && git pull --ff-only

      - name: Subir nova versão em produção
        run: docker compose --env-file /srv/erp/.env -p erp-api -f infra/api/docker-compose.yml up -d --build --remove-orphans
        approval: true

      - name: Conferir healthcheck da API
        run: curl -fsS https://api.seudominio.com/healthz
```

Como o exemplo se comporta depois de importado:

1. Cada push na branch monitorada dispara o **CI Principal**. O destino (Local ou VPS) vem do vínculo configurado em **Configurações → GitHub**.
2. Se typecheck, testes e varredura passarem, o `on_success` inicia o **Deploy Produção**.
3. O deploy para no passo `Subir nova versão em produção` (`approval: true`) com status **Aguardando aprovação** e envia o cartão para a aba **Aprovações**.
4. Ao clicar em **Aprovar e continuar**, o Workploy sobe os containers e encerra com o healthcheck da API.

Para usar uma receita única, remova a chave `workflows:` e os `id` e escreva os campos direto na raiz do arquivo, no formato da seção 2.

---

## 5. Exemplos prontos para copiar

### Exemplo 1: Node / Next.js + Docker Compose Redeploy na VPS

Fluxo para validar build Node/pnpm e atualizar containers Docker em `/srv/meu-app-web` na VPS Linux.

```yaml
name: Next.js CI e Redeploy VPS
trigger: push
workdir: /srv/meu-app-web

steps:
  - name: Instalar dependências
    run: pnpm install --frozen-lockfile

  - name: Lint e checagem de tipos
    run: |
      pnpm lint
      pnpm typecheck

  - name: Build de produção
    run: pnpm build

  - name: Reconstruir e subir containers
    run: docker compose up -d --build --remove-orphans

  - name: Validar saúde HTTP
    run: curl -fsS http://127.0.0.1:3000/api/health
```

**Dica de vínculo:** em **Configurações → GitHub**, aponte este projeto para a sua VPS Linux e selecione **Webhook 24/7 na VPS** ou **Sondagem 24/7 na VPS**. Se `workdir` ficar vazio, o `workploy-runner` faz o `git clone` e `git pull` em `/var/lib/workploy/workspaces/<repo>` antes do primeiro passo.

---

### Exemplo 2: Rust (`cargo check` + `clippy` + `test`)

Receita de verificação contínua para crates e workspaces em Rust.

```yaml
name: Rust Cargo CI
trigger: push

steps:
  - name: Verificar compilação
    run: cargo check --all-targets

  - name: Análise estática (Clippy)
    run: cargo clippy --no-deps -- -D warnings

  - name: Executar suíte de testes
    run: cargo test
```

**Dica de infraestrutura:** para builds Rust pesados, configure uma VPS classificada como **Padrão** (2–4 vCPU, 4–8 GB RAM) ou **Pesada** (4–8 vCPU, 8–16 GB RAM) na aba **Conexões & Diagnóstico**, e ative a **Regra de limpeza (Purge)** para evitar acúmulo da pasta `target/`.

---

### Exemplo 3: Pipeline encadeado (CI -> Portão de Aprovação -> Deploy VPS)

Separe validação e publicação em duas receitas encadeadas. A primeira roda a cada push e dispara a segunda apenas se todos os testes passarem; a segunda pausa no portão de aprovação antes de reiniciar produção.

**Arquivo 1 (`ci-principal.yml`):**

```yaml
name: CI Principal
trigger: push
on_success: wf-deploy-vps

steps:
  - name: Testes e checagem de tipos
    run: |
      pnpm install --frozen-lockfile
      pnpm -r typecheck
      pnpm test:ci

  - name: Varredura de segredos
    run: gitleaks detect --source . --no-banner
```

**Arquivo 2 (`deploy-vps.yml`):**

```yaml
name: Deploy VPS
trigger: manual
workdir: /srv/erp-producao

steps:
  - name: Atualizar código na VPS
    run: git fetch origin && git pull --ff-only

  - name: Subir nova versão em produção
    run: docker compose up -d --build api web worker
    approval: true

  - name: Conferir healthchecks
    run: curl -fsS https://api.seudominio.com/healthz
```

**Como ligar o encadeamento na prática:**
1. Crie ou importe primeiro a receita **Deploy VPS** para que ela receba um ID interno.
2. Abra **CI Principal → Editar fluxo** e, em **Encadear ao concluir com sucesso**, selecione **Deploy VPS** (o aplicativo grava o ID real em `on_success` / `x-chain`).
3. Quando o CI terminar sem falhas, o Workploy abre a execução de **Deploy VPS**, para no passo `Subir nova versão em produção` e envia um alerta para a aba **Aprovações**.
4. Clique em **Aprovar e continuar** (ou **Rejeitar** para abortar sem reiniciar os containers).

---

### Exemplo 4: Projeto Windows / Multiplataforma

Use este padrão para escolher entre **Runner Local (Esta máquina)** no Windows e **VPS Linux**.

#### Quando usar cada runner

- **Runner Local (Windows):** projetos Win32/MSIX, scripts `.ps1`, instaladores `.exe`, soluções Visual Studio (`.sln`, `.vcxproj`), apps desktop ou automações disparadas ao salvar arquivos (`trigger: watch`).
- **VPS Linux:** APIs Node/Next.js/Rust, bancos em Docker Compose, serviços `systemd` e deploys que precisam rodar 24/7 com seu computador desligado.

> Ao escanear um repositório em **Buscar fluxos no repo**, o Workploy (`github::detect_platform_from_paths`) identifica arquivos como `installer/`, `.ps1`, `.sln` ou `.vcxproj` como alvo **Windows**, e `Dockerfile`, `docker-compose.yml` ou `.service` como alvo **Linux**.

#### Receita para build e empacotamento local no Windows

```yaml
name: Build e Empacotamento Windows
trigger: manual
workdir: C:\Projects\meu-app-desktop

steps:
  - name: Validar compilação e testes
    run: cargo test --manifest-path app/Cargo.toml

  - name: Gerar instalador e pacote portátil
    run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-installer.ps1

  - name: Conferir artefatos gerados
    run: dir dist
```

Para rodar o mesmo projeto ao editar arquivos na máquina Windows:
1. Em **Configurações → GitHub**, role até **Vigia de pasta local & Sondagem**, ative **Ativar vigia local** e informe a pasta (ex: `C:\Projects\meu-app-desktop`).
2. Na receita, defina o gatilho como **Mudança na pasta** (`watch`).
3. Deixe o vínculo apontado para **Local (Esta máquina)**.

---

## 6. Requisitos estimados por tipo de projeto e diagnóstico de subpastas

O **Dashboard do Projeto** e a tela **Conexões & Diagnóstico** classificam suas VPSs pelas faixas definidas em `CapacityTier` (`app/src/model.rs`). Use a tabela abaixo para dimensionar cada projeto antes de vincular o deploy 24/7.

### Requisitos estimados por tipo de projeto

| Tipo de projeto | Perfil Workploy | vCPU | RAM | Disco livre | Observações operacionais |
|---|---|---|---|---|---|
| **Node / Next.js monorepo** | **Padrão** (ou **Pesada** acima de 15 pacotes) | 2–4 vCPU | 4–8 GB (swap 2 GB recomendado) | 50+ GB SSD | `pnpm -r typecheck` e `next build` consomem picos de RAM; mantenha `NODE_OPTIONS="--max-old-space-size=3072"` se a VPS tiver 4 GB. |
| **Rust / Cargo** | **Padrão** a **Pesada** | 4–8 vCPU | 8–16 GB | 80+ GB NVMe | O diretório `target/` cresce rápido entre builds; ative a **Regra de limpeza (Purge)** por teto em GB na VPS. |
| **Docker Compose** | **Leve** a **Padrão** | 1–4 vCPU | 2–8 GB | 25–50+ GB SSD | Builds multi-stage e imagens antigas acumulam camadas; combine `docker compose up -d --build` com **Purge** de imagens não usadas. |

### Diagnóstico de deploy em subpastas (`infra/api`, `infra/web`)

Monorepos costumam guardar arquivos Compose e Dockerfiles em subpastas como `infra/api/docker-compose.yml` ou `infra/web/docker-compose.yml`. Se o deploy falhar logo nos primeiros segundos, verifique estes quatro pontos:

1. **`workdir` vs. `-f <subpasta>`:** mantenha `workdir` na raiz do repositório (ou vazio para usar `/var/lib/workploy/workspaces/<repo>`) e passe `-f infra/api/docker-compose.yml` explícito. Se você trocar `workdir` para `infra/api`, comandos anteriores como `pnpm -r typecheck` ou `gitleaks detect --source .` perdem o acesso à raiz do monorepo.
2. **Contexto de build no `docker-compose.yml`:** quando o `Dockerfile` dentro de `infra/api/` copia `package.json`, `pnpm-lock.yaml` ou `packages/` da raiz, o `build.context` precisa apontar para `../..` (raiz do repo) e `dockerfile` para `infra/api/Dockerfile`:
   ```yaml
   services:
     api:
       build:
         context: ../..
         dockerfile: infra/api/Dockerfile
   ```
   Se `context` estiver como `.` dentro de `infra/api`, o Docker falha com `COPY failed: file not found in build context`.
3. **Arquivo `.env` e `--project-directory`:** o `docker compose` lê por padrão o `.env` da pasta onde está o arquivo YAML (`infra/api/.env`), ignorando o `.env` da raiz. Para usar o `.env` da raiz ou um caminho fixo na VPS, passe `--env-file`:
   ```bash
   docker compose --env-file /srv/meu-app/.env -f infra/api/docker-compose.yml up -d --build
   ```
4. **Caminhos independentes para API e Web:** ao separar **Deploy API** (`infra/api`) e **Deploy Web** (`infra/web`), dê nomes distintos com `-p` (`--project-name`) para que `--remove-orphans` de um stack não derrube containers do outro:
   ```bash
   docker compose -p erp-api -f infra/api/docker-compose.yml up -d --build --remove-orphans
   docker compose -p erp-web -f infra/web/docker-compose.yml up -d --build --remove-orphans
   ```

---

## 7. Como importar e testar passo a passo na UI

### Passo 1: Criar ou importar a receita

1. **Geração automática pelo repo:** com o repositório vinculado, clique em **Gerar receita** no card do projeto em **Execuções**. O Workploy analisa `package.json`, `Cargo.toml` e `docker-compose.yml` para criar uma receita pronta de CI e deploy.
2. **Importar `.github/workflows/*.yml` ou colar YAML:** clique em **Buscar fluxos no repo** no modal de vínculo para importar workflows existentes de `.github/workflows/`, ou clique em **Importar YAML** em **Execuções** para colar seu YAML.
3. **Pelo editor visual:** clique em **+ Novo fluxo** na barra lateral (agrupada por projeto), escolha um preset (**Node / pnpm CI**, **Rust Cargo CI**, **Docker Compose Redeploy**, **Auditoria & Smoke**) ou monte os passos manualmente e clique em **Salvar fluxo**.
4. **Importar do repositório (fonte da verdade):** com o repo vinculado, clique em **Importar do repositório** para ler o `.workploy.yml` da raiz ou os arquivos de `infra/workploy/*.yml`, revise o preview dos passos e importe (seção 4).

### Passo 2: Vincular o repositório e escolher o servidor

1. Acesse **Configurações → GitHub** e valide seu Token PAT em **Testar conexão** (no Windows, o token fica no Gerenciador de Credenciais; no Linux, no cofre local `0600`).
2. Localize o repositório na lista e clique em **Vincular** (ou **+ Novo vínculo**).
3. No modal de vínculo:
   - Informe ou selecione a **Branch monitorada** (`main`, `main,dev` ou **Todas (*)**).
   - Escolha o **Servidor de destino**: **Local (Esta máquina)** ou uma das **VPSs Linux** cadastradas em **Configurações → VPS**.
   - Selecione o **Modo de disparo** (**Webhook 24/7 na VPS**, **Sondagem 24/7 na VPS** ou **Sondagem no Desktop**).
   - Escolha a receita em **Fluxo disparado** (ou deixe **Automático (por gatilho)** para disparar os fluxos marcados com `push`).
4. Clique em **Salvar vínculo**. O Workploy sincroniza a configuração na VPS alvo (ou clique em **Sincronizar na VPS** na linha do vínculo para forçar o envio imediato de `/etc/workploy/config.json`).
   - Para **Webhook 24/7 na VPS**, use **Copiar URL Webhook** e **Copiar Segredo** na linha do vínculo e cadastre em **Settings → Webhooks** no GitHub.

### Passo 3: Testar e acompanhar execuções

1. **Disparar e abrir o Dashboard do Projeto:** selecione o projeto na barra lateral para ver os KPIs (onde roda, recursos recomendados, duração média, taxa de sucesso e commits) e clique em **Executar agora**.
2. **Acompanhar Jobs, Sub-etapas e Live Logs:** abra a execução em andamento para expandir cada job e suas sub-etapas (`::workploy-substep::`). Os logs rolam automaticamente (`stick_bottom`) acompanhando a linha mais recente e a sub-etapa ativa.
3. **Conferir commits e filtrar por branch:** use o seletor de **Branch** em **Execuções** e o botão **Ver commits** para consultar os commits recentes via API do GitHub (SHA abreviado, mensagem, autor e data).
4. **Liberar portões de aprovação:** passos com `approval: true` ou `x-approval: true` geram um cartão destacado em âmbar na aba **Aprovações**. Revise o resumo ou os logs e clique em **Aprovar e continuar** ou **Rejeitar**.

---

## 8. Ambiente do passo: o contrato com a VPS

Cada passo executado numa VPS recebe um ambiente padronizado pelo Workploy:

| Variável | Conteúdo | Motivo |
|---|---|---|
| `HOME` | `/root` (ou o `HOME` do serviço) | Sem `HOME`, `~` e `$HOME/.cargo/env` quebram. |
| `PATH` | `/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin` primeiro, depois `~/.local/share/pnpm`, `/usr/local/share/pnpm`, `~/.cargo/bin`, `/usr/local/cargo/bin`, e o `PATH` herdado | `git`, `curl`, `dnf`/`apt-get`, `docker`, `node` e `pnpm` sempre resolvidos. |
| `WORKPLOY_WORKDIR` | Raiz do workspace do runner ou o `workdir` da receita | A receita encontra o código sem chutar caminho. |
| `WORKPLOY_REPO` | `dono/repo` do vínculo | Provenance nos logs. |
| `WORKPLOY_BRANCH` | Branch monitorada | Provenance nos logs. |

### 8.1 Template de recipe portátil (Debian × RHEL)

```yaml
name: CI e Deploy (portátil)
on: [push]

jobs:
  build:
    runs-on: self-hosted
    steps:
      - name: "Preparar ambiente"
        run: |
          set -e
          export HOME="${HOME:-/root}"
          export PATH="/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH"

          echo "::workploy-substep::Ambiente"
          cat /etc/os-release 2>/dev/null | head -2 || true
          node --version; pnpm --version; git --version

          echo "::workploy-substep::Navegador (se a suíte usar)"
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
          [ -z "$BROWSER" ] && { echo "ERRO: sem navegador." >&2; exit 1; }
          export CHROME_PATH="$BROWSER"

      - name: "Rodar os testes"
        run: |
          set -e
          export HOME="${HOME:-/root}"
          export PATH="/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH"
          export CI=1
          export CHROME_PATH="${CHROME_PATH:-$(command -v google-chrome || command -v chromium || true)}"

          REPO_ROOT=""
          if [ -n "${WORKPLOY_WORKDIR:-}" ] && [ -f "$WORKPLOY_WORKDIR/package.json" ]; then
            REPO_ROOT="$WORKPLOY_WORKDIR"
          elif [ -d /root/Projects/meu-repo ]; then
            REPO_ROOT="/root/Projects/meu-repo"
          else
            echo "ERRO: raiz do repositório não encontrada." >&2; exit 1
          fi
          cd "$REPO_ROOT"

          pnpm install --frozen-lockfile
          nice -n 10 pnpm typecheck
          nice -n 10 node scripts/run-package-checks.mjs --concurrency=1
```

### 8.2 Diagnóstico da VPS de destino

Rode o preflight **antes** de escrever a receita:

```bash
ssh <alias-vps> 'bash -s' < scripts/vps-preflight.sh
```

O relatório detalha SO, gerenciador de pacotes, `HOME`/`PATH` herdados, ferramentas,
Docker + plugin compose, navegador, versão do `workploy-runner` e as raízes de código
disponíveis na máquina.

Detalhes, checklist e tabela de anti-padrões em
[`RECEITA_POR_VPS.md`](RECEITA_POR_VPS.md).

---

## 9. Limites de recursos e passos pulados

Uma receita pode **limitar os recursos** consumidos na VPS (para não
estourar uma máquina que também hospeda produção) e **pular passos**.

### 9.1 Limites de CPU e RAM

```yaml
name: CI Oktopus
on: [push]
x-cpu: 2        # prende o passo em 2 vCPUs (taskset)
x-mem: 4096     # teto de 4 GB de RAM por passo (cgroup systemd)

jobs:
  build:
    runs-on: self-hosted
    steps:
      - name: Suítes
        run: pnpm test
```

| Campo | Efeito na VPS | Implementação |
|---|---|---|
| `x-cpu: N` | O passo roda apenas em `N` núcleos; os demais ficam livres para a produção. | `taskset -c 0-(N-1)` (só aplica se a VPS tiver mais núcleos que o limite). |
| `x-mem: MB` | O passo é encerrado se passar do teto — OOM controlado, sem derrubar a VPS. | `systemd-run --scope -p MemoryMax=MB` (fallback: `ulimit -v`). |

Os limites também podem ser ajustados pela interface: **Editar fluxo → Recursos na VPS**.
O Workploy **sugere** valores a partir do peso dos passos, da capacidade real da VPS
de destino (medida pelo diagnóstico) e do histórico medido das execuções — você aceita
ou edita como preferir.

### 9.2 Pular passos

```yaml
      - name: Testes de navegador
        run: pnpm test:e2e
        x-skip: true      # ignorado nesta execução
```

Na interface, cada passo do editor tem o interruptor **"Pular este passo"**. Passos
pulados aparecem como *Ignorado* na linha do tempo e **não quebram o fluxo** — os
demais continuam rodando normalmente.

> O YAML exportado pelo app mantém `x-skip: true` e `x-cpu`/`x-mem`, então o arquivo
> continua sendo a fonte da verdade de verdade.

### 9.3 Consumo medido (KPIs)

Depois de rodar na VPS, o runner mede o consumo real de cada execução
(pico de RAM pela cgroup e CPU acumulada) e o Workploy mostra em **Resumo da execução →
Consumo medido na VPS**:

- **Pico de RAM** — o maior consumo de memória do passo mais pesado.
- **CPU acumulada** — total de segundos de CPU consumidos.
- **Aviso de OOM** — quando o passo estourou o limite de memória configurado.

Esses números alimentam a próxima sugestão de limites: quanto maior o pico medido,
maior a margem sugerida.
