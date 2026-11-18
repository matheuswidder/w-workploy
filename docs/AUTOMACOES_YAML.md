# Guia de Automações `.yml` no Workploy

Este guia ensina como criar, estruturar, importar e testar automações `.yml` no Workploy.

## 1. Conceito central: Receita vs. Vínculo

O Workploy separa o fluxo em duas partes independentes para que você reaproveite a mesma receita em projetos e servidores diferentes.

| Conceito | O que define | Onde você configura |
|---|---|---|
| **Receita** (`Workflow`) | **O que** executar: nome, gatilho base, pasta de trabalho, lista sequencial de passos, portões de aprovação e fluxo seguinte (`on_success`). | Barra lateral (`Novo fluxo`) ou modal **Importar YAML** / **Arquivo YAML**. |
| **Vínculo** (`RepoBinding`) | **Onde** e **quando** rodar: repositório GitHub (`dono/nome`), branches monitoradas, destino (`Local` ou uma `VPS`) e modo de disparo. | **Configurações → GitHub** (botão **Vincular** ou **Novo vínculo**). |

### Onde a automação roda

- **Local (Esta máquina):** o aplicativo desktop executa os comandos direto no seu computador. No Windows, cada passo roda via `cmd /C` (`app/src/engine.rs`); no Linux, via `sh -c`. Ideal para builds nativos Windows, testes locais e pasta vigiada.
- **VPS Linux (`workploy-runner`):** o serviço `systemd` provisionado na VPS clona ou atualiza o repositório em `/var/lib/workploy/workspaces/<slug>` (ou usa o `workdir` informado) e executa cada passo via `/bin/bash -lc` (ou `/bin/sh -lc`). Seus pipelines continuam rodando mesmo com o computador desligado.

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
| `name` | Sim | Texto | Nome exibido na barra lateral, no histórico de **Execuções** e nos vínculos. |
| `trigger` | Não (padrão `manual`) | `manual`, `push`, `watch` | `manual` roda ao clicar em **Executar** ou por encadeamento; `push` responde a atualizações do GitHub; `watch` dispara ao detectar mudanças na pasta local configurada em **Configurações → GitHub**. |
| `workdir` | Não | Caminho absoluto ou relativo | Diretório onde os comandos rodam (ex: `/srv/app` na VPS ou `C:\Projects\app` no Windows). Quando vazio na VPS, o `workploy-runner` usa `/var/lib/workploy/workspaces/<repo>`. |
| `on_success` | Não | ID de outro fluxo | Inicia automaticamente a receita indicada quando todos os passos atuais terminam com sucesso. |
| `steps` | Sim | Lista (mínimo 1 item) | Etapas executadas em ordem. Se um passo retorna código diferente de zero, os seguintes são ignorados (`Ignorado`). |
| `steps[].name` | Não | Texto | Título curto exibido na linha do tempo e nos logs. Se omitido, vira `Passo N`. |
| `steps[].run` | Sim | String ou bloco multilinha (`\|`) | Comando shell da etapa. Use `\|` para scripts com várias linhas. |
| `steps[].approval` | Não (padrão `false`) | `true` ou `false` | Pausa a execução antes deste passo com o status **Aguardando aprovação** e exibe o cartão na aba **Aprovações**. |

> **Nota sobre o importador atual (`app/src/yaml.rs`):** o parser integrado do modal **Importar YAML** / **Arquivo YAML** lê e exporta um formato compatível com GitHub Actions simplificado:
> - `on: [push]` ou `on: [workflow_dispatch]` no lugar de `trigger`
> - `jobs.build.runs-on: local` ou `self-hosted` para definir o runner padrão
> - `x-approval: true` dentro do passo no lugar de `approval: true`
> - `x-chain: <id-do-fluxo>` na raiz no lugar de `on_success`
> - `x-server: <id-da-vps>` e `x-repo: <id-do-vinculo>` para fixar servidor e vínculo na exportação
>
> Campos como `workdir` e o gatilho `Mudança na pasta` (`watch`) ficam disponíveis direto no formulário **Editar fluxo** após importar. Se você colar um YAML direto no modal atual, use a sintaxe equivalente abaixo:
>
> ```yaml
> name: Exemplo compatível com importador atual
> on: [push]
>
> jobs:
>   build:
>     runs-on: self-hosted
>     steps:
>       - name: Validar código
>         run: npm test
>       - name: Publicar em produção
>         run: docker compose up -d --build
>         x-approval: true
> x-chain: wf-deploy-id
> ```

---

## 3. Exemplos prontos para copiar

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

**Dica de vínculo:** em **Configurações → GitHub**, aponte este projeto para a sua VPS Linux e selecione **Webhook 24/7 na VPS** ou **Sondagem 24/7 na VPS**. Se `workdir` ficar vazio, o `workploy-runner` faz o `git clone` e `git pull` automático em `/var/lib/workploy/workspaces/<repo>` antes do primeiro passo.

---

### Exemplo 2: Rust (`cargo check` + `clippy` + `test`)

Receita direta de verificação contínua para crates e workspaces em Rust.

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

### Exemplo 3: Pipeline encadeado (CI → Portão de Aprovação → Deploy VPS)

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
1. Crie ou importe primeiro a receita **Deploy VPS** para que ela receba um ID interno (ou configure pelo formulário).
2. Abra **CI Principal → Editar fluxo** e, em **Encadear ao concluir com sucesso**, selecione **Deploy VPS** (o aplicativo grava o ID real em `on_success` / `x-chain`).
3. Quando o CI terminar sem falhas, o Workploy abre automaticamente a execução de **Deploy VPS**, para no passo `Subir nova versão em produção` e envia um alerta para a aba **Aprovações**.
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

Se quiser rodar o mesmo projeto automaticamente quando editar arquivos na máquina Windows:
1. Em **Configurações → GitHub**, role até **Vigia de pasta local & Sondagem**, ative **Ativar vigia local** e informe a pasta (ex: `C:\Projects\meu-app-desktop`).
2. Na receita, defina o gatilho como **Mudança na pasta** (`watch`).
3. Deixe o vínculo apontado para **Local (Esta máquina)**.

---

## 4. Como importar e testar passo a passo na UI

### Passo 1: Criar ou importar a receita

1. **Pelo editor visual:** clique em **+ Novo fluxo** na barra lateral (ou **Novo fluxo** no topo de **Execuções**). Escolha um dos presets rápidos (**Node / pnpm CI**, **Rust Cargo CI**, **Docker Compose Redeploy**, **Auditoria & Smoke**) ou monte os passos manualmente, preencha a **Pasta de trabalho** se precisar e clique em **Salvar fluxo**.
2. **Pelo YAML:** na tela **Execuções**, clique em **Importar YAML** para criar um fluxo novo, ou abra uma receita existente na barra lateral e clique em **Arquivo YAML** para visualizar, copiar e editar a definição. Cole o conteúdo e clique em **Aplicar YAML**.

### Passo 2: Vincular o repositório e escolher o servidor

1. Acesse **Configurações → GitHub** e valide seu Token PAT em **Testar conexão** (no Windows, o token fica protegido no Gerenciador de Credenciais; nunca em arquivos JSON).
2. Localize o repositório na lista e clique em **Vincular** (ou clique em **Novo vínculo**).
3. No modal de vínculo:
   - Informe ou selecione a **Branch monitorada** (clique em **Buscar branches** para marcar múltiplas branches como `main,dev`, ou clique em **Todas (*)**).
   - Escolha o **Servidor de destino**: **Local (Esta máquina)** ou uma das **VPSs Linux** cadastradas em **Configurações → VPS & Motor**.
   - Selecione o **Modo de disparo** (**Webhook 24/7 na VPS**, **Sondagem 24/7 na VPS** ou **Sondagem no Desktop**).
   - Escolha a receita em **Fluxo disparado** (ou deixe **Automático (por gatilho)** para disparar os fluxos marcados com `push`).
4. Clique em **Salvar vínculo**. Se o destino for uma VPS, clique em **Sincronizar na VPS** na linha do vínculo para enviar o `config.json` ao `workploy-runner`.
   - Caso tenha escolhido **Webhook 24/7 na VPS**, use os botões **Copiar URL Webhook** e **Copiar Segredo** na mesma linha e cole em **Settings → Webhooks** do seu repositório no GitHub.

### Passo 3: Testar e acompanhar execuções

1. **Disparar na hora:** na tela **Execuções**, encontre o card do seu projeto em **Projetos & Repositórios configurados** e clique em **Executar agora** (ou abra uma receita na barra lateral e clique em **Executar fluxo**).
2. **Acompanhar logs e resumo:** clique na linha da execução para abrir a visão detalhada com duração por etapa, anotações e logs em tempo real. Use o botão **Arquivo YAML** na lateral direita do detalhe se quiser inspecionar ou ajustar a receita.
3. **Conferir commits e filtrar por branch:**
   - Na barra de filtros de **Execuções**, use o seletor de **Branch** para isolar execuções da branch desejada (ex: `main`, `dev`, `staging`).
   - Quando a consulta de commits por branch (**Ver commits**) estiver disponível para o vínculo ou branch selecionada, o Workploy consulta os commits recentes via API do GitHub (`github::fetch_repo_commits`) com SHA abreviado, mensagem, autor e data para confirmar qual revisão será testada.
4. **Liberar portões de aprovação:** se o fluxo tiver algum passo com `approval: true` (ou `x-approval: true`), o card fica destacado em âmbar na aba **Aprovações**. Revise o resumo ou os logs e clique em **Aprovar e continuar** ou **Rejeitar**.
