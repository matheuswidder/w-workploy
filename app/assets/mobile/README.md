# Workploy Mobile — página web servida pelo runner

Implementação da **opção C** do `docs/mobile/PLANO-WORKPLOY-MOBILE.md` (v2),
fases **F2 + F3 + F4**: lista de execuções, detalhe com log ao vivo, disparo
de fluxo e aprovação de portões.

Tudo é estático e sem dependência — nada de bundler, CDN, framework ou fonte
remota. O aparelho fica offline com frequência e a página precisa abrir sem
rede para o app.

## Arquivos

| Arquivo | Para que serve |
|---|---|
| `index.html` | Esqueleto. Monta `#screen`, `#nav`, `#fab` e hosts de sheet/toast. |
| `app.css` | Tokens e componentes. Cores de `app/src/ui/theme.rs`; tipografia de phone (§7). |
| `app.js` | Cliente vanilla: fetch com Bearer, poll, estado, sheets, log incremental. |
| `tools/static-check.js` | Gate estático: HTML/CSS/JS, contraste WCAG 2.1, alturas, rotas. |
| `tools/dom-harness.js` | Renderiza as telas com o schema real (`Run`/`StepRun`) num jsdom. |
| `tools/log-contract-test.js` | Prova o contrato do log: `offset`/`size` em caracteres. |

## Qual rota deve servir (trabalho de outra frente)

Sugestão, alinhada ao contrato do runner:

```
GET /ui              -> app/assets/mobile/index.html
                        Content-Type: text/html; charset=utf-8
                        Cache-Control: no-cache
GET /ui/app.css      -> app/assets/mobile/app.css
                        Content-Type: text/css; charset=utf-8
GET /ui/app.js       -> app/assets/mobile/app.js
                        Content-Type: text/javascript; charset=utf-8
```

Regras para quem for servir:

- **Sem bearer no `/ui`, `/ui/app.css`, `/ui/app.js`.** A página é pública, mas
  cada chamada de dados (`/runs`, `/run/…`, `/trigger`) exige
  `Authorization: Bearer <token>`. O token é digitado no aparelho e guardado em
  `localStorage` (`wp.mobile.session.v1`); nunca aparece no HTML nem no log.
- Servir relativo (`href="app.css"`, `src="app.js"`) é o que o `index.html`
  espera. Se o runner acrescentar prefixo extra, ajuste os dois `<link>`/`<script>`.
- `Content-Security-Policy` recomendada:
  `default-src 'self'; style-src 'self'; script-src 'self'; img-src 'self' data:; connect-src 'self'`.
- Não use cache imutável nos três arquivos: eles mudam com o runner.

## Contrato do runner que a página assume

| Rota | Uso |
|---|---|
| `GET /runs?limit=100` | lista (o runner aceita `?limit=` de 1 a 100; padrão 50) |
| `GET /run/{id}` | run inteiro (`Run` do `model.rs`) |
| `GET /run/{id}/log?offset=N&tail=200000[&step=idx]` | trecho incremental do log |
| `POST /trigger` | disparar fluxo (`workflow_id`, `trigger_label`, `repo`, `branch`, `actor`) |
| `POST /runs/{id}/approve` | decidir portão (§8.4): corpo `{"decision":"approve"\|"reject","note":"..."}` |
| `POST /runs/{id}/cancel` | cancelar execução em andamento (fora do portão) |

> **Sem rota de catálogo de fluxos.** O `POST /trigger` exige um `workflow_id`,
> e o runner não expõe `GET /workflows`. A folha "Disparar fluxo" usa os
> `workflow_id`s vistos nas execuções recentes e aceita o identificador digitado
> à mão. Quando o runner expuser o catálogo, a folha passa a ler dali — hoje
> ela não inventa lista.

### Aprovação: como a página age

`POST /runs/{id}/approve` com corpo JSON:

```json
{ "decision": "approve", "note": "liberar só o rodapé" }
```

- `decision` é `"approve"` (Aprovar) ou `"reject"` (Recusar). `note` é a nota
  do campo opcional; em branco vai como `""`.
- Confirmação **sempre em sheet** (§8.4): a folha tem o resumo do run, campo
  `textarea` de nota **sem foco automático** (o teclado só abre por toque) e
  dois botões — **Aprovar e continuar** (primário) e **Recusar e cancelar**
  (ghost). A recusa mostra um aviso diferente, deixando claro que
  **cancela** o run (o runner grava `cancel_requested`).

Erros do portão, como o maestro lê (nunca código HTTP, nunca stack):

| Resposta | Texto na tela |
|---|---|
| `409 not_awaiting_approval` | "Este portão já foi decidido." |
| `404 run_not_found` | "Essa execução não existe mais." |
| `400 invalid_decision` | "A decisão não foi aceita." |
| `401` | fecha a folha, volta para a tela de conexão e pede reconectar |
| rede / timeout | "Sem resposta do servidor." / "O servidor demorou demais." |

409 e 404 fecham a folha e atualizam a lista: a tela reflete o servidor em vez
de insistir na decisão. 400 mantém a folha aberta com a mensagem no próprio
sheet. Recusar um run fora de portão continua usando `POST /runs/{id}/cancel`
na tela de detalhe.

### Log: `offset` e `size` são CARACTERES, não bytes

O runner faz `len(full[offset:offset+tail])` sobre uma `str` do Python. O app
pede `offset=N&tail=200000` e avança com `countCodePoints(chunk)` (code
points). **Nunca** use `TextEncoder`, `byteLength` ou `chunk.length` cru como
cursor: emoji ocupa 2 unidades UTF-16 e 1 code point, e o cursor deslizaria.

Campos extras do payload também são tratados:

- `size` — total em caracteres (exibido no aviso de recorte);
- `truncated: true` — o bloco ganha o aviso "Carregado a partir do caractere
  X de Y. Trechos anteriores não estão na tela." com botão **Recarregar do
  início** (`offset=0`);
- `log: ""` com `size > offset` — poll sem novidade: a tela **mantém** o
  conteúdo já renderizado, não pisca nem apaga.

### Respostas de erro que a página reconhece

| Status | Estado na tela |
|---|---|
| `401` | "Token inválido" + **Reconectar** (abre a folha de conexão) |
| `404` | "Rota não encontrada" / "Execução não encontrada" + tentar de novo |
| timeout/rede | "Sem conexão" / "Servidor demorou demais" + tentar de novo |
| `403` | mensagem do servidor + "o token não pode essa ação" |
| qualquer outro | "Erro ao carregar" com a mensagem do servidor |

Se a lista já tinha dados e o poll falha, a tela mostra o último estado com
um aviso discreto — nunca finge lista vazia.

## Design e acessibilidade (normativo, §6 e §7 do plano)

- Margem lateral 16, gutter 16, ritmo 8; cards de raio 12 com padding 16.
- Botão primário 48 de altura / mínimo 120 de largura. Secundário/ghost 44.
  Chip 40 visual com área tocável 44 (`::after` de -4px). Ícone de header 24
  em caixa 48. Campo de formulário 48.
- Texto: H1 24/30, H2 18/24, H3 16/22, BODY 15/22, SMALL 13/18, CAPTION 12/16,
  MONO 13/19. Só a legenda do bottom nav e o badge ficam em 11, como manda o
  §5.1.
- Bottom nav de 64 com 4 itens (Execuções, Projetos, Aprovações, Conexões),
  ícone 24 + rótulo 11.
- **24 px de folga dos gestos do Lomiri**: `--safe: 24px` reservado no topo
  e no pé do conteúdo; a bottom nav é `position: fixed` acima dos últimos
  24 px e a área útil termina antes dela. Nenhum alvo interativo cai nessa
  faixa.
- Status sempre **pill + texto**, nunca só cor.
- Log: mono 13, sem wrap por padrão, scroll horizontal só dentro de
  `.log-scroll`, botão "Ir para o fim" quando o auto-scroll é desligado,
  cópia por toque longo. Toggle "Quebrar linha" e "Copiar log" num toolbar.
- **Sem hover e sem tooltip** (§9): o que não aparece no toque não existe.
  Foco visível (`:focus-visible`) mantido para leitor de tela e teclado.
- `prefers-color-scheme` (dark padrão, light quando o navegador pede) e
  `prefers-reduced-motion` (movimento sai, nada de conteúdo é removido).

### Densidade (gate 9) — o app declara a escala

O Ubuntu Touch entrega `devicePixelRatio = 1` com 1080 px físicos. Um layout
de 393 dp desenhado 1:1 vira formiga. A v2 do plano resolve declarando a
escala no app; `applyDensity()` faz isso em `app.js`:

- lê `devicePixelRatio` e `innerWidth`, calcula `physical = round(innerWidth * dpr)`;
- se `physical >= 900` e `dpr < 1.5`, aplica
  `documentElement.style.zoom = round(physical / 393 * 100) / 100`
  (1080 / 393 ≈ `2.75`);
- grava `documentElement.dataset.density = "forced" | "native"` para auditoria;
- é idempotente e roda no boot (antes da primeira tela) e em
  `resize`/`orientationchange`.

O CSS continua em 48 dp de alvo: com `zoom`, `getBoundingClientRect` devolve
o retângulo já escalado, então o gate do §12 mede ≥ 48 **após** a escala.
Nenhuma altura foi reescrita para compensar.

### Cache offline (gate 8) — último estado conhecido

O último `GET /runs?limit=100` bem-sucedido é guardado em
`localStorage['workploy.cache.runs.v1']` como
`{ savedAt: <epoch ms>, runs: [...] }`.

- Teto de ~512 KB: se passar, guarda os **50 mais recentes** com
  `truncated: true` em vez de falhar.
- Só é reescrito quando o topo da lista muda (o poll de 2 s não toca no
  localStorage a cada ciclo).
- Sem rede e reaberto, o app renderiza a lista guardada com a faixa
  **"Sem conexão. Mostrando o estado de dd/mm HH:mm."** no fuso
  `America/Sao_Paulo`. Passando de 24 h, acrescenta a idade
  ("Esse estado tem 2 dias."). O cabeçalho diz "Estado guardado de HH:mm" e
  **nunca** "Atualizado agora".
- `401` não usa cache: vai para a tela de reconectar.
- Nada de dado inventado: o cache é do aparelho, e a tela diz isso.

### Paleta (fonte: `app/src/ui/theme.rs`)

Dark: `bg_canvas #161616`, `surface_1 #1e1e1e`, `surface_2 #262626`,
`text_primary #fafafa`, `text_secondary #d4d4d4`, `text_muted #9e9e9e`,
`accent #7698fd`, `success #6bd586`, `warning #f2cf76`, `danger #f17471`.
Light equivalente do `UiTheme::new(false)`.

## Mudanças que o mobile pode fazer na árvore

Só `app/assets/mobile/**`. `app/src/**` é de outra frente.

## Verificação

```bash
# 1) Gate estático (sem jsdom)
node app/assets/mobile/tools/static-check.js

# 2) Telas com o schema real (precisa de jsdom)
JSDOM_PATH=<caminho do jsdom>/lib/api.js \
  node app/assets/mobile/tools/dom-harness.js

# 3) Contrato do log em caracteres
JSDOM_PATH=<caminho do jsdom>/lib/api.js \
  node app/assets/mobile/tools/log-contract-test.js
```

O gate estático cobre HTML, CSS e JS por leitura de arquivo. O harness de DOM
prova que as telas renderizam com dados no formato real, mas **não mede
layout** — `getBoundingClientRect` num jsdom devolve 0 para tudo. Por isso a
área de alvo é verificada pelo CSS (`static-check.js`) e a medida final no
aparelho (393×873) é a humana, no navegador.

### Limitação conhecida

O harness não reproduz `visibilitychange`, `focus` nem `scrollTop` de um
navegador real. O "ir para o fim" e a pausa do poll por aba em segundo plano
estão implementados e cobertos por leitura de código, não por evento simulado.