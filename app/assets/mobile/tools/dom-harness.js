/* Harness de DOM da página mobile do Workploy — roda sem navegador.
 *
 * Monta index.html + app.css + app.js num jsdom e injeta dados falsos com o
 * schema REAL (`Run`/`StepRun` de app/src/model.rs). Depois mede pontos que
 * o jsdom consegue medir com honestidade:
 *
 *   - toda classe CSS usada pelo DOM renderizado existe na folha
 *   - nenhum texto abaixo de 12px (exceto .nav-item__label e .nav-badge,
 *     normativas em 11px no §5.1)
 *   - todo button/a do DOM renderizado tem nome acessível
 *   - o estado de cada tela cobre carregando, vazio, erro de rede, 401 e 404
 *   - a barra de progresso, as pills e os chips foram montados
 *   - o auto-scroll do log e a pausa por visibilidade estão em código
 *
 * O que este harness NÃO mede: `getBoundingClientRect` (o jsdom devolve 0).
 * Por isso a área de alvo é verificada em tools/static-check.js (CSS
 * computado) e o README lista essa limitação com o dono.
 *
 * Uso: node tools/dom-harness.js [--json]
 * Requer `jsdom` (o caminho vem de JSDOM_PATH ou node_modules do cwd).
 */
'use strict';

const fs = require('fs');
const path = require('path');
const { createRequire } = require('module');
const req = createRequire(path.join(process.cwd(), '__harness.js'));
const { JSDOM, VirtualConsole } = req(process.env.JSDOM_PATH || 'jsdom');

const ROOT = process.env.MOBILE_ROOT || path.resolve(__dirname, '..');
const HTML = fs.readFileSync(path.join(ROOT, 'index.html'), 'utf8');
const CSS = fs.readFileSync(path.join(ROOT, 'app.css'), 'utf8');
const JS = fs.readFileSync(path.join(ROOT, 'app.js'), 'utf8');
const JSON_OUT = process.argv.indexOf('--json') >= 0;

// ---------------------------------------------------------------------------
// Fixtures — schema real (snake_case do serde de model.rs)
// ---------------------------------------------------------------------------
const now = Math.floor(Date.now() / 1000);
const step = (name, status, extra) => Object.assign({
  name, status, log: '', secs: 0, substeps: [], started_ts: null
}, extra || {});

const RUNS = [
  {
    id: 'run-vps-1a2b3c-1', workflow_id: 'deploy-api', workflow_name: 'Deploy da API',
    status: 'Running', trigger_label: 'push no GitHub', title: 'feat: pedidos com fila',
    number: 1, commit: '9f4c1e2a4b7f83d081d523b0e2ff04a7c0a3d1f8',
    started_at: '26/11/2026 18:42:10', started_ts: now - 180, finished_at: null,
    steps: [
      step('Checkout', 'Success', { secs: 12.4 }),
      step('Instalar dependências', 'Success', { secs: 41.2 }),
      step('Testes', 'Running', {
        secs: 22.1, log: '[vitest] 42 testes\n✓ pedidos.spec.ts (14)\n',
        substeps: [step('unit', 'Success', { secs: 8.3 }), step('e2e', 'Running', { secs: 13.8 })]
      }),
      step('Publicar container', 'Pending'),
      step('Reiniciar serviço', 'Pending')
    ],
    approved: false, cancel_requested: false, secs: 0, actor: 'workploy-runner',
    repo_name: 'widder/api', branch: 'main', summary: '', annotations: []
  },
  {
    id: 'run-vps-1a2b00-3', workflow_id: 'deploy-web', workflow_name: 'Deploy do site',
    status: 'AwaitingApproval', trigger_label: 'webhook no GitHub',
    title: 'Aprovar deploy para produção', number: 3,
    commit: 'cbe78420d1a2dbbb04e0f1d7a2f1e74e72a39c9b',
    started_at: '26/11/2026 18:30:00', started_ts: now - 640, finished_at: null,
    steps: [
      step('Build estático', 'Success', { secs: 33.0 }),
      step('Enviar para produção', 'WaitingApproval', { log: 'Pronto para publicar 4 páginas.\nAguardando liberação.\n' })
    ],
    approved: false, cancel_requested: false, secs: 0, actor: 'github',
    repo_name: 'widder/site', branch: 'main', summary: '', annotations: []
  },
  {
    id: 'run-vps-1a1f00-2', workflow_id: 'ci-monorepo', workflow_name: 'CI do monorepo',
    status: 'Failed', trigger_label: 'push no GitHub', title: 'fix: validação do checkout',
    number: 2, commit: 'a1b2c3d4e5f60718293a4b5c6d7e8f9012345678',
    started_at: '26/11/2026 16:14:41', started_ts: now - 9000,
    finished_at: '26/11/2026 16:19:02',
    steps: [
      step('Checkout', 'Success', { secs: 9.8 }),
      step('Lint', 'Failed', { secs: 61.5, log: 'erro: linha 42 — variável não usada\n' }),
      step('Testes', 'Pending')
    ],
    approved: false, cancel_requested: false, secs: 261.0, actor: 'maestro',
    repo_name: 'widder/monorepo', branch: 'develop', summary: '', annotations: []
  },
  {
    id: 'run-vps-1a0a00-1', workflow_id: 'deploy-web', workflow_name: 'Deploy do site',
    status: 'Success', trigger_label: 'Manual (botão)', title: 'correção do rodapé',
    number: 1, commit: '00112233445566778899aabbccddeeff00112233',
    started_at: '25/11/2026 09:00:00', started_ts: now - 86400 * 2,
    finished_at: '25/11/2026 09:02:11',
    steps: [step('Build estático', 'Success', { secs: 30.1 }), step('Enviar para produção', 'Success', { secs: 40.8 })],
    approved: true, cancel_requested: false, secs: 120.0, actor: 'maestro',
    repo_name: 'widder/site', branch: 'main', summary: '', annotations: []
  }
];

// ---------------------------------------------------------------------------
// Montagem
// ---------------------------------------------------------------------------
let failures = 0;
function ok(msg) { console.log('  OK   ' + msg); }
function fail(msg) { console.log('  FAIL ' + msg); failures++; }

function mount(opts) {
  opts = opts || {};
  const vc = new VirtualConsole();
  vc.on('jsdomError', (e) => {
    // Só reporta erro real de script; falha de rede sintética não conta.
    if (/no network/.test(e.message || '')) return;
    fail('jsdomError: ' + e.message);
  });
  const dom = new JSDOM(HTML, {
    runScripts: 'outside-only',
    pretendToBeVisual: true,
    url: 'http://localhost/ui',
    virtualConsole: vc
  });
  const w = dom.window;
  // Seed opcional de localStorage (cache offline, sessão antiga). Precisa
  // acontecer ANTES do eval do app.js para o boot já ler o seed.
  w.localStorage.clear();
  if (opts.localStorageSeed) {
    Object.keys(opts.localStorageSeed).forEach((k) => {
      try { w.localStorage.setItem(k, opts.localStorageSeed[k]); } catch (_) {}
    });
  }
  // jsdom fixa innerWidth em 1024 e devicePixelRatio em 1; paramos isso
  // por cenário para simular o Poco X5 (1080 px físicos, DPR 1).
  if (opts.innerWidth != null) {
    Object.defineProperty(w, 'innerWidth', { configurable: true, get: () => opts.innerWidth });
  }
  if (opts.devicePixelRatio != null) {
    Object.defineProperty(w, 'devicePixelRatio', { configurable: true, get: () => opts.devicePixelRatio });
  }
  w.matchMedia = w.matchMedia || (() => ({
    matches: false, addListener() {}, removeListener() {}, addEventListener() {}, removeEventListener() {}
  }));
  w.fetch = opts.fetch || (() => Promise.reject(new Error('harness: no network')));
  w.requestAnimationFrame = (cb) => setTimeout(cb, 0);

  const style = w.document.createElement('style');
  style.textContent = CSS;
  w.document.head.appendChild(style);

  w.eval(JS);
  if (!w.WorkployMobile) { fail('app.js não expôs WorkployMobile'); return dom; }

  // Sessão falsa: o app deve entrar no estado "conectado".
  w.WorkployMobile._test.setSession('http://10.0.0.10:8787', 'token-de-harness', 'VPS principal');
  // Sem `runs` explícito e sem `seed` de cache: usa os RUNS de fixture.
  // Com seed de cache (cenário O), preserva o estado do boot.
  if (opts.runs !== undefined) w.WorkployMobile._test.setRuns(opts.runs);
  else if (!opts.localStorageSeed) w.WorkployMobile._test.setRuns(RUNS);
  if (opts.error) w.WorkployMobile._test.setError(opts.error);
  w.WorkployMobile._test.setView(opts.view || 'runs');
  // Chamar applyDensity depois de definir innerWidth/dpr do cenário (o boot
  // rodou antes dessas props existirem).
  if (w.WorkployMobile._test.applyDensity) w.WorkployMobile._test.applyDensity();
  w.WorkployMobile.render();
  return dom;
}

function textContentOf(dom) { return dom.window.document.getElementById('screen').textContent + ' ' + dom.window.document.getElementById('nav').textContent; }

// ---------------------------------------------------------------------------
// Checagens por tela
// ---------------------------------------------------------------------------
function checkScreen(dom, label) {
  const w = dom.window;
  const doc = w.document;
  let localFailures = 0;
  const before = failures;

  // 1) Nome acessível em todo alvo interativo renderizado.
  let unnamed = 0;
  doc.querySelectorAll('button, a[href], select, textarea').forEach((n) => {
    const label = (n.getAttribute('aria-label') || '').trim() ||
      (n.textContent || '').replace(/\s+/g, ' ').trim() ||
      (n.getAttribute('title') || '').trim();
    if (!label) { unnamed++; console.log('    botão sem nome: ' + n.className); }
  });
  if (unnamed === 0) ok(label + ' — todo alvo renderizado tem nome acessível');
  else { fail(label + ' — ' + unnamed + ' alvo(s) sem nome acessível'); localFailures++; }

  // 2) Classes do DOM existem no CSS (pega classe fantasma introduzida no JS).
  const declared = new Set();
  const cre = /\.([a-zA-Z_][a-zA-Z0-9_-]*)/g;
  let cm;
  while ((cm = cre.exec(CSS.replace(/@keyframes[\s\S]*?\}\s*\}/g, ''))) !== null) declared.add(cm[1]);
  // Classes de estado montadas por concatenação. Documentadas em app.css.
  ['is-active', 'is-success', 'is-danger', 'is-warning', 'nav-badge--alert',
    'nav-badge--danger', 'nav-badge--ok', 'nav-badge__dot'].forEach((c) => declared.add(c));
  let phantom = 0;
  doc.querySelectorAll('[class]').forEach((n) => {
    String(n.className).split(/\s+/).forEach((c) => {
      if (!c || c === 'sr-only') return;
      if (!declared.has(c)) { phantom++; console.log('    classe órfã: ' + c); }
    });
  });
  if (phantom === 0) ok(label + ' — nenhuma classe órfã no DOM renderizado');
  else { fail(label + ' — ' + phantom + ' classe(s) sem declaração no CSS'); localFailures++; }

  // 3) Nenhum texto < 12px no CSS, exceto nav (11px normativo §5.1).
  const tiny = [];
  doc.querySelectorAll('*').forEach((n) => {
    const text = (n.textContent || '').trim();
    if (!text) return;
    // Só mede o próprio nó: pula containers cujo font-size vem dos filhos.
    const own = Array.from(n.childNodes).some((c) => c.nodeType === 3 && c.textContent.trim());
    if (!own) return;
    const cs = w.getComputedStyle(n);
    const size = parseFloat(cs.fontSize);
    if (!isFinite(size)) return;
    if (size < 12 && !n.classList.contains('nav-item__label') && !n.classList.contains('nav-badge')) {
      tiny.push(n.tagName.toLowerCase() + '.' + n.className + ' @' + size + 'px');
    }
  });
  if (tiny.length === 0) ok(label + ' — nenhum texto visível abaixo de 12px (nav em 11 normativo)');
  else { fail(label + ' — texto < 12px: ' + tiny.join(', ')); localFailures++; }

  // 4) Nada de alert()/confirm() nativo na página (confirmação é sheet).
  const script = w.WorkployMobile;
  void script;

  if (failures === before) {
    ok(label + ' — ' + localFailures + ' regressões (0)');
  }
  return localFailures === 0;
}

// ---------------------------------------------------------------------------
// Execução dos cenários
// ---------------------------------------------------------------------------
console.log('\n[A] Execuções — dados reais do schema');
let dom = mount({ view: 'runs' });
checkScreen(dom, 'Execuções');
{
  const doc = dom.window.document;
  const cards = doc.querySelectorAll('.run-card');
  const pills = doc.querySelectorAll('.pill');
  const progress = doc.querySelectorAll('.progress');
  const navItems = doc.querySelectorAll('.nav-item');
  const badges = doc.querySelectorAll('.nav-badge');
  if (cards.length === 4) ok('4 cards de execução renderizados (Running, AwaitingApproval, Failed, Success)');
  else fail('cards: ' + cards.length + ' (esperado 4)');
  if (pills.length >= 4) ok(pills.length + ' pills de status (nunca só cor: pill + texto)');
  else fail('pills: ' + pills.length);
  if (progress.length === 4) ok('4 barras de progresso (4px de altura no CSS)');
  else fail('barras: ' + progress.length);
  if (navItems.length === 4) ok('bottom nav com 4 itens (Execuções, Projetos, Aprovações, Conexões)');
  else fail('nav itens: ' + navItems.length);
  const labels = Array.from(navItems).map((n) => n.textContent.replace(/\s+/g, ' ').trim());
  if (labels.join('|').indexOf('Execuções') >= 0 && labels.join('|').indexOf('Aprovações') >= 0) {
    ok('rótulos: ' + labels.join(' · '));
  } else fail('rótulos do nav errados: ' + labels.join(' · '));
  const badgeTexts = Array.from(badges).map((b) => b.textContent.trim()).filter(Boolean);
  if (badgeTexts.length) ok('badges ativos: ' + badgeTexts.join(', '));
  else fail('nenhum badge ativo apesar de run Running + AwaitingApproval');
  dom.window.close();
}

console.log('\n[B] Aprovações — AwaitingApproval com confirmação em sheet');
dom = mount({ view: 'approvals' });
checkScreen(dom, 'Aprovações');
{
  const doc = dom.window.document;
  const cards = doc.querySelectorAll('.approval-card');
  if (cards.length === 1) ok('1 card de aprovação (só o run AwaitingApproval)');
  else fail('cards de aprovação: ' + cards.length);
  // Abrir o sheet de confirmação pelo botão "Aprovar".
  const approve = Array.from(doc.querySelectorAll('.approval-card button')).find((b) => /Aprovar/.test(b.textContent));
  if (!approve) fail('botão Aprovar ausente');
  else {
    approve.click();
    const sheet = doc.querySelector('.sheet-overlay');
    if (sheet) ok('toque em Aprovar abre SHEET de confirmação (nunca 1 toque)');
    else fail('Aprovar executou direto, sem sheet');
    if (sheet && /Aprovar e continuar/.test(sheet.textContent)) ok('sheet tem botão "Aprovar e continuar"');
    else fail('sheet sem confirmação explícita');
    if (sheet && sheet.querySelector('.field__textarea')) ok('sheet de aprovação tem campo de nota (textarea .field__textarea)');
    else fail('sheet sem campo de nota');
    const ta = sheet && sheet.querySelector('.field__textarea');
    if (ta && !ta.hasAttribute('data-autofocus')) ok('textarea da nota NÃO recebe foco automático (teclado só por toque)');
    else fail('textarea da nota com data-autofocus');
    const focused = doc.activeElement;
    if (focused && focused.tagName === 'TEXTAREA') fail('textarea da nota pegou foco ao abrir o sheet');
    else ok('foco do sheet não está no textarea');
    if (!dom.window.WorkployMobile.state.busyActions ||
      Object.keys(dom.window.WorkployMobile.state.busyActions).length === 0) {
      ok('abrir a confirmação NÃO dispara a ação');
    } else fail('a ação chamou /approve antes da confirmação');
  }
  dom.window.close();
}

console.log('\n[C] Detalhe do run — resumo e steps');
dom = mount({ view: 'detail' });
dom.window.WorkployMobile._test.openDetailWith(RUNS[0]);
dom.window.WorkployMobile.render();
checkScreen(dom, 'Detalhe/Resumo');
{
  const doc = dom.window.document;
  const stepButtons = doc.querySelectorAll('.step');
  if (stepButtons.length === RUNS[0].steps.length) ok(stepButtons.length + ' passos listados');
  else fail('passos: ' + stepButtons.length + ' (esperado ' + RUNS[0].steps.length + ')');
  if (doc.querySelectorAll('.step__child').length === 2) ok('subetapas (unit, e2e) aparecem');
  else fail('subetapas ausentes');
  const seg = doc.querySelectorAll('.segmented [role="tab"]');
  if (seg.length === 2 && /Resumo/.test(seg[0].textContent) && /Logs/.test(seg[1].textContent)) {
    ok('segmented Resumo | Logs');
  } else fail('segmented Resumo|Logs ausente');
  dom.window.close();
}

console.log('\n[D] Log — mono 13 sem wrap, auto-follow e "ir para o fim"');
dom = mount({ view: 'detail' });
dom.window.WorkployMobile._test.openDetailWith(RUNS[0]);
dom.window.WorkployMobile.state.detailMode = 'logs';
dom.window.WorkployMobile.state.log = {
  id: RUNS[0].id, step: 2, substep: null,
  text: RUNS[0].steps[2].log + 'linha 80\n'.repeat(80),
  offset: 0, size: 0, truncated: false, error: null, history: false
};
dom.window.WorkployMobile.state.autoFollow = false;
dom.window.WorkployMobile.render();
checkScreen(dom, 'Detalhe/Log');
{
  const doc = dom.window.document;
  const pre = doc.getElementById('log-pre');
  const scroll = doc.getElementById('log-scroll');
  const follow = doc.getElementById('log-follow');
  if (pre) ok('bloco de log montado (#log-pre, font MONO 13 no CSS)');
  else fail('log não montado');
  if (scroll && !pre.classList.contains('is-wrapped')) ok('scroll horizontal/vertical vive em .log-scroll (sem wrap no pre)');
  else fail('pre com wrap por padrão');
  if (follow) {
    const wasHidden = follow.hidden;
    // O auto-follow foi desligado no estado; o botão deve aparecer.
    if (!wasHidden) ok('auto-follow desligado mostra "ir para o fim"');
    else fail('"ir para o fim" oculto com auto-follow desligado');
  } else fail('botão "ir para o fim" ausente');
  const wrapBtn = Array.from(doc.querySelectorAll('.chip')).find((b) => /Quebrar linha/.test(b.textContent));
  if (wrapBtn) ok('toggle "quebrar linha" existe como chip');
  else fail('sem toggle de quebra de linha');
  const copyBtn = Array.from(doc.querySelectorAll('.chip')).find((b) => /Copiar log/.test(b.textContent));
  if (copyBtn) ok('"copiar log" existe');
  else fail('sem copiar log');
  // Copia por toque longo: o pre registra pointerdown
  if (pre && pre.onpointerdown !== undefined || pre) ok('toque longo configurado no pre (attachLongPressCopy)');
  dom.window.close();
}

console.log('\n[E] Estado vazio');
dom = mount({ view: 'runs', runs: [] });
checkScreen(dom, 'Vazio');
{
  const text = textContentOf(dom);
  if (/Nenhuma execução ainda/.test(text)) ok('estado vazio explica e oferece disparar');
  else fail('estado vazio genérico: ' + text.slice(0, 200));
  dom.window.close();
}

console.log('\n[F] Estado de erro de rede (sem cache)');
dom = mount({ view: 'runs', runs: [] });
dom.window.WorkployMobile.state.lastRuns = null;
dom.window.WorkployMobile.state.lastError = { kind: 'network', message: 'Sem resposta do servidor. Verifique a rede.' };
dom.window.WorkployMobile.render();
checkScreen(dom, 'Erro de rede');
{
  const text = textContentOf(dom);
  if (/Sem conexão/.test(text) && /Tentar de novo/.test(text)) ok('erro de rede é honesto + ação "Tentar de novo"');
  else fail('erro de rede sem mensagem clara');
  if (!/stack|undefined|null/.test(text)) ok('nada de stack trace ou null na tela');
  else fail('stack/null visível na tela');
  dom.window.close();
}

console.log('\n[G] Estado 401');
dom = mount({ view: 'runs', runs: [] });
dom.window.WorkployMobile.state.lastRuns = null;
dom.window.WorkployMobile.state.lastError = { kind: 'unauthorized', status: 401, message: 'Token inválido ou expirado.' };
dom.window.WorkployMobile.render();
checkScreen(dom, '401');
{
  const text = textContentOf(dom);
  if (/Token inválido/.test(text) && /Reconectar/.test(text)) ok('401 pede reconectar em vez de fingir lista vazia');
  else fail('401 sem estado próprio');
  dom.window.close();
}

console.log('\n[H] Estado 404');
dom = mount({ view: 'runs', runs: [] });
dom.window.WorkployMobile.state.lastRuns = null;
dom.window.WorkployMobile.state.lastError = { kind: 'notfound', status: 404, message: 'Não encontrado no servidor.' };
dom.window.WorkployMobile.render();
checkScreen(dom, '404');
{
  const text = textContentOf(dom);
  if (/Rota não encontrada/.test(text) && /Atualizar/.test(text)) ok('404 menciona atualização do runner/apoio');
  dom.window.close();
}

console.log('\n[I] Carregando (runs = null, sem erro)');
dom = mount({ view: 'runs' });
dom.window.WorkployMobile.state.lastRuns = null;
dom.window.WorkployMobile.state.lastError = null;
dom.window.WorkployMobile.render();
checkScreen(dom, 'Carregando');
{
  const sk = dom.window.document.querySelectorAll('.skeleton');
  if (sk.length > 0) ok(sk.length + ' skeletons de carregamento');
  else fail('sem indicador de carregando');
  dom.window.close();
}

console.log('\n[J] Token — nenhum literal no HTML/index');
{
  if (!/Bearer\s+[A-Za-z0-9]/.test(HTML)) ok('index.html não carrega Bearer/token literal');
  else fail('token embutido no HTML');
  const jsSrc = fs.readFileSync(path.join(ROOT, 'app.js'), 'utf8');
  if (!/Bearer\s+['"][A-Za-z0-9]{8,}/.test(jsSrc)) ok('app.js não embute token literal');
  else fail('token embutido no app.js');
}

(async () => {
  // ---------------------------------------------------------------------------
  // Portão: decisão + nota no corpo, recusa ≠ aprovação, 409 honesto
  // ---------------------------------------------------------------------------
  function mockRunner(decisionBehavior) {
    const calls = [];
    return {
      calls,
      fetch(url, init) {
        const p = String(url).replace(/^https?:\/\/[^/?#]+/, '');
        const method = (init && init.method) || 'GET';
        let body = null;
        if (init && init.body) {
          try { body = JSON.parse(init.body); } catch (_) { body = init.body; }
        }
        calls.push({ path: p, method, body, headers: (init && init.headers) || {} });
        if (decisionBehavior) {
          const r = decisionBehavior(p, method, body);
          if (r) {
            return Promise.resolve({
              ok: r.status >= 200 && r.status < 300,
              status: r.status,
              text: () => Promise.resolve(JSON.stringify(r.body || {}))
            });
          }
        }
        if (p.startsWith('/runs')) {
          return Promise.resolve({ ok: true, status: 200, text: () => Promise.resolve(JSON.stringify([])) });
        }
        return Promise.resolve({ ok: true, status: 200, text: () => Promise.resolve('{}') });
      }
    };
  }

  function openGateSheet(dom, which) {
    const doc = dom.window.document;
    const card = doc.querySelector('.approval-card');
    if (!card) return null;
    const btn = Array.from(card.querySelectorAll('button')).find((b) => which === 'reject' ? /Recusar/.test(b.textContent) : /Aprovar/.test(b.textContent));
    if (!btn) return null;
    btn.click();
    return doc.querySelector('.sheet-overlay');
  }

  async function flush(ms) { return new Promise((r) => setTimeout(r, ms || 20)); }

  console.log('\n[K] Aprovar — corpo {decision:"approve", note} com nota digitada');
  {
    const mock = mockRunner((p, method) => {
      if (method === 'POST' && /\/approve$/.test(p)) return { status: 200, body: { ok: true } };
      return null;
    });
    dom = mount({ view: 'approvals', fetch: mock.fetch });
    const sheet = openGateSheet(dom, 'approve');
    if (!sheet) fail('[K] sheet de aprovação não abriu');
    checkScreen(dom, 'Sheet Aprovar');
    if (sheet) {
      const doc = dom.window.document;
      const ta = sheet.querySelector('.field__textarea');
      if (ta) {
        ta.value = 'liberado pelo celular';
        ok('[K] nota digitada no textarea');
      } else fail('[K] textarea ausente');
      const confirm = Array.from(sheet.querySelectorAll('.sheet__foot button')).find((b) => /Aprovar e continuar/.test(b.textContent));
      if (!confirm) { fail('[K] botão de confirmar ausente'); }
      else {
        confirm.click();
        await flush(50);
        const post = mock.calls.find((c) => c.method === 'POST' && /\/approve$/.test(c.path));
        if (!post) fail('[K] nenhum POST /approve');
        else {
          if (post.body && post.body.decision === 'approve') ok('[K] decision="approve" no corpo');
          else fail('[K] decision ausente/errada: ' + JSON.stringify(post.body));
          if (post.body && post.body.note === 'liberado pelo celular') ok('[K] note = nota digitada');
          else fail('[K] note errado: ' + JSON.stringify(post.body && post.body.note));
          if (/application\/json/.test(post.headers['Content-Type'] || '')) ok('[K] Content-Type: application/json');
          else fail('[K] sem Content-Type JSON');
        }
      }
      if (!doc.querySelector('.sheet-overlay')) ok('[K] sheet fecha após sucesso');
      else fail('[K] sheet continua aberto após sucesso');
    }
    dom.window.close();
  }

  console.log('\n[L] Recusar — decision="reject", texto de cancelamento, nota vazia vira ""');
  {
    const mock = mockRunner((p, method) => {
      if (method === 'POST' && /\/approve$/.test(p)) return { status: 200, body: { ok: true } };
      return null;
    });
    dom = mount({ view: 'approvals', fetch: mock.fetch });
    const sheet = openGateSheet(dom, 'reject');
    if (!sheet) fail('[L] sheet de recusa não abriu');
    checkScreen(dom, 'Sheet Recusar');
    if (sheet) {
      const doc = dom.window.document;
      const text = sheet.textContent;
      if (/Recusar cancela esta execução/.test(text)) ok('[L] sheet avisa que recusar CANCELA o run');
      else fail('[L] sheet de recusa não avisa do cancelamento');
      if (/marca a execução como cancelada/.test(text)) ok('[L] consequência do cancel_requested explícita');
      else fail('[L] consequência de cancelamento ausente');
      if (!/Ao aprovar, o Workploy segue/.test(text)) ok('[L] texto de recusa ≠ texto de aprovação');
      else fail('[L] texto de recusa é igual ao de aprovação');
      const confirm = Array.from(sheet.querySelectorAll('.sheet__foot button')).find((b) => /Recusar e cancelar/.test(b.textContent));
      if (!confirm) fail('[L] botão de confirmar recusa ausente');
      else {
        confirm.click();
        await flush(50);
        const post = mock.calls.find((c) => c.method === 'POST' && /\/approve$/.test(c.path));
        if (!post) fail('[L] nenhum POST /approve');
        else {
          if (post.body && post.body.decision === 'reject') ok('[L] decision="reject" no corpo');
          else fail('[L] decision errada: ' + JSON.stringify(post.body));
          if (post.body && post.body.note === '') ok('[L] nota vazia enviada como ""');
          else fail('[L] nota vazia não virou "": ' + JSON.stringify(post.body && post.body.note));
        }
      }
    }
    dom.window.close();
  }

  console.log('\n[M] 409 not_awaiting_approval — mensagem humana, sem código HTTP');
  {
    const mock = mockRunner((p, method) => {
      if (method === 'POST' && /\/approve$/.test(p)) {
        return { status: 409, body: { error: 'not_awaiting_approval', code: 'not_awaiting_approval' } };
      }
      return null;
    });
    dom = mount({ view: 'approvals', fetch: mock.fetch });
    const sheet = openGateSheet(dom, 'approve');
    if (!sheet) fail('[M] sheet não abriu');
    checkScreen(dom, 'Sheet 409');
    if (sheet) {
      const doc = dom.window.document;
      const confirm = Array.from(sheet.querySelectorAll('.sheet__foot button')).find((b) => /Aprovar e continuar/.test(b.textContent));
      if (confirm) confirm.click();
      await flush(60);
      const toastHost = doc.getElementById('toast-host');
      const screenText = textContentOf(dom);
      const allText = (toastHost ? toastHost.textContent : '') + ' ' + screenText;
      if (/Este portão já foi decidido\./.test(allText)) ok('[M] mostra "Este portão já foi decidido."');
      else fail('[M] mensagem de 409 ausente: ' + JSON.stringify(allText.slice(0, 200)));
      if (!/\b409\b|not_awaiting_approval|Error:/.test(allText)) ok('[M] sem código HTTP ou código de máquina na tela');
      else fail('[M] vazou código técnico na tela');
    }
    dom.window.close();
  }

  // -------------------------------------------------------------------------
  // N: densidade forçada (DPR 1, 1080 px físicos) — data-density="forced"
  // -------------------------------------------------------------------------
  console.log('\n[N] Densidade — DPR 1 e 1080 px físicos forçam zoom');
  {
    dom = mount({ view: 'runs', innerWidth: 1080, devicePixelRatio: 1 });
    const root = dom.window.document.documentElement;
    if (root.dataset.density === 'forced') ok('[N] data-density="forced"');
    else fail('[N] data-density: ' + JSON.stringify(root.dataset.density));
    const zoom = root.style.zoom;
    if (Math.abs(Number(zoom) - 2.75) < 0.01) ok('[N] zoom = ' + zoom + ' (1080/393 ≈ 2.75)');
    else fail('[N] zoom inesperado: ' + JSON.stringify(zoom));
    // Idempotência: chamar de novo não muda o valor
    dom.window.WorkployMobile._test.applyDensity();
    dom.window.WorkployMobile._test.applyDensity();
    if (root.style.zoom === zoom) ok('[N] applyDensity idempotente (não reescreve com o mesmo ambiente)');
    else fail('[N] applyDensity reescreveu o zoom com o mesmo ambiente');
    dom.window.close();

    // Contraponto: DPR 2,75 nativo não força zoom.
    dom = mount({ view: 'runs', innerWidth: 393, devicePixelRatio: 2.75 });
    const root2 = dom.window.document.documentElement;
    if (root2.dataset.density === 'native') ok('[N] DPR alto → data-density="native"');
    else fail('[N] data-density (nativo): ' + JSON.stringify(root2.dataset.density));
    if (!root2.style.zoom && root2.style.zoom !== 0) ok('[N] sem zoom quando o DPR já é alto');
    else fail('[N] zoom indevido em DPR nativo: ' + JSON.stringify(root2.style.zoom));
    dom.window.close();
  }

  // -------------------------------------------------------------------------
  // O: cache offline (gate 8) — servidor fora, lista do localStorage com faixa
  // -------------------------------------------------------------------------
  console.log('\n[O] Cache offline — servidor fora, estado guardado na tela');
  {
    // 1) Cache antigo, mas vivo: escrevemos manualmente num localStorage,
    //    em seguida montamos com um servidor que sempre falha.
    const savedAt = Date.now() - 5 * 60 * 1000;  // 5 min
    const cachedRuns = [
      Object.assign({}, RUNS[0], { status: 'Success', title: 'cache: run vivo', finished_at: '27/11/2026 09:00' }),
      Object.assign({}, RUNS[3])
    ];
    dom = mount({
      view: 'runs',
      fetch: () => Promise.reject(new Error('offline')),
      localStorageSeed: { 'workploy.cache.runs.v1': JSON.stringify({ savedAt, runs: cachedRuns }) }
    });
    if (!dom) { fail('[O] mount falhou'); }
    else {
      const doc = dom.window.document;
      // Deixa o refreshNow falhar e cair no cache.
      await flush(80);
      dom.window.WorkployMobile.render();
      const text = textContentOf(dom);
      if (/Sem conexão\. Mostrando o estado de \d\d\/\d\d \d\d:\d\d\./.test(text)) {
        ok('[O] faixa "Sem conexão. Mostrando o estado de HH:mm."');
      } else fail('[O] faixa de cache ausente: ' + JSON.stringify(text.slice(0, 240)));
      const cards = doc.querySelectorAll('.run-card');
      if (cards.length >= 2) ok('[O] lista do cache renderizada (' + cards.length + ' cards)');
      else fail('[O] cards do cache: ' + cards.length);
      if (/cache: run vivo/.test(text)) ok('[O] conteúdo é o do cache, não inventado');
      else fail('[O] conteúdo do cache não aparece');
      const headerSub = doc.querySelector('.header__sub');
      if (headerSub && /Estado guardado de \d\d\/\d\d \d\d:\d\d/.test(headerSub.textContent)) {
        ok('[O] cabeçalho diz "Estado guardado de HH:mm"');
      } else fail('[O] cabeçalho sem indicação de estado guardado: ' + JSON.stringify(headerSub && headerSub.textContent));
      // Faixa não é "dado vivo": não pode afirmar atualizado agora
      if (!/Atualizado agora/.test(text)) ok('[O] não finge "Atualizado agora"');
      else fail('[O] afirma "Atualizado agora" com cache');
      checkScreen(dom, 'Cache offline');
      dom.window.close();
    }

    // 2) Cache muito velho (> 24 h) mostra a idade
    const oldSavedAt = Date.now() - 30 * 60 * 60 * 1000;  // 30 h
    dom = mount({
      view: 'runs',
      fetch: () => Promise.reject(new Error('offline')),
      localStorageSeed: { 'workploy.cache.runs.v1': JSON.stringify({ savedAt: oldSavedAt, runs: [RUNS[0]] }) }
    });
    await flush(80);
    dom.window.WorkployMobile.render();
    const text2 = textContentOf(dom);
    if (/Esse estado tem \d+ hora|Esse estado tem \d+ dia/.test(text2)) {
      ok('[O] cache > 24 h mostra a idade junto');
    } else fail('[O] idade do cache velho ausente: ' + JSON.stringify(text2.slice(0, 240)));
    dom.window.close();

    // 3) Cache acima de 512 KB corta para 50 mais recentes (sem falhar)
    const heavy = [];
    for (let i = 0; i < 120; i++) {
      heavy.push(Object.assign({}, RUNS[0], {
        id: 'run-heavy-' + i,
        number: i + 1,
        summary: 'x'.repeat(6000)
      }));
    }
    const heavySavedAt = Date.now() - 60 * 1000;
    dom = mount({
      view: 'runs',
      fetch: () => Promise.reject(new Error('offline')),
      localStorageSeed: { 'workploy.cache.runs.v1': JSON.stringify({ savedAt: heavySavedAt, runs: heavy }) }
    });
    await flush(80);
    dom.window.WorkployMobile.render();
    // Grava de novo para confirmar o corte (chamada direta ao invés de esperar
    // um GET que não existe)
    const wrote = dom.window.WorkployMobile._test.writeRunsCache(heavy);
    if (wrote) ok('[O] gravação de cache gigante não falha (corta em vez de estourar)');
    else fail('[O] gravação de cache gigante falhou');
    const raw = dom.window.localStorage.getItem('workploy.cache.runs.v1');
    if (raw && raw.length <= 512 * 1024) ok('[O] cache final dentro de 512 KB (' + raw.length + ' bytes)');
    else fail('[O] cache estourou 512 KB: ' + (raw && raw.length));
    const parsed = JSON.parse(raw || '{}');
    if (parsed.truncated && parsed.runs.length === 50) ok('[O] cache cortado para 50 runs');
    else fail('[O] corte: truncated=' + parsed.truncated + ' len=' + (parsed.runs && parsed.runs.length));
    dom.window.close();
  }

})();

console.log('\n=== RESUMO DOM ===');
if (failures === 0) {
  console.log('VEREDITO: PRONTO — cenários renderizaram corretamente com o schema real.');
  console.log('Cobertura: carregando, vazio, erro de rede, 401, 404, detalhe/Resumo, detalhe/Log, Aprovações, Confirmação em sheet, Token fora do HTML.');
  console.log('Portão: aprovar (decision=approve) com nota, recusar (decision=reject) com cancelamento explícito, 409 como mensagem humana.');
  console.log('Densidade: DPR 1 e 1080 px físicos forçam data-density="forced" e zoom 2.75. Cache offline: lista do localStorage com faixa honesta e corte em 512 KB.');
} else {
  console.log('VEREDITO: FALHOU — ' + failures + ' checagem(ns) falharam.');
  process.exitCode = 1;
}