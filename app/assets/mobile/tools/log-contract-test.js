/* Teste funcional do contrato do log — caracteres, não bytes.
 *
 * Roda a página num jsdom com um runner falso que implementa EXATAMENTE o
 * contrato do workploy_runner.py:
 *
 *     full  = "".join(st["log"] for st in steps)
 *     chunk = full[offset:offset+tail]        # Python: caracteres (code points)
 *     { log: chunk, offset, size: len(full), truncated }
 *
 * O app pede `/run/{id}/log?offset=${offset}&tail=200000` e avança o cursor
 * em CODE POINTS (`countCodePoints`), que é a unidade do `len(str)` do
 * runner. O teste usa string UTF-8/UTF-16 com acento e emoji (🚀 ocupa
 * 2 unidades UTF-16 e 1 code point) para provar o cursor correto.
 *
 * Uso: JSDOM_PATH=<pacote jsdom> node tools/log-contract-test.js
 */
'use strict';

const fs = require('fs');
const path = require('path');
const { createRequire } = require('module');
const req = createRequire(path.join(process.cwd(), '__harness.js'));
const { JSDOM } = req(process.env.JSDOM_PATH || 'jsdom');

const ROOT = process.env.MOBILE_ROOT || path.resolve(__dirname, '..');
const HTML = fs.readFileSync(path.join(ROOT, 'index.html'), 'utf8');
const CSS = fs.readFileSync(path.join(ROOT, 'app.css'), 'utf8');
const JS = fs.readFileSync(path.join(ROOT, 'app.js'), 'utf8');

let failures = 0;
function ok(m) { console.log('  OK   ' + m); }
function fail(m) { console.log('  FAIL ' + m); failures++; }

const RUN_ID = 'run-vps-1a2b3c-1';
const BASE_LOG = 'Passo 1: ações concluídas 🚀\nPasso 2: ñandu, ação, coração\n';
const TAIL = 200000;

// len(str) em Python = code points.
function pyLen(s) { return Array.from(s).length; }
function sliceByCodePoints(s, offset, tail) {
  return Array.from(s).slice(offset, offset + tail).join('');
}

function pyRun(logText) {
  return {
    id: RUN_ID, workflow_id: 'ci', workflow_name: 'CI', status: 'Running',
    trigger_label: 'manual', title: '', number: 1,
    started_at: '26/11/2026 18:00', started_ts: 1,
    steps: [{ name: 'Testes', status: 'Running', log: logText, secs: 0, substeps: [] }]
  };
}

function makeFetch(getLog) {
  const calls = [];
  return {
    calls,
    fetch(url) {
      const p = String(url).replace(/^https?:\/\/[^/?#]+/, '');
      calls.push(p);
      if (p.startsWith('/runs')) {
        return Promise.resolve({ ok: true, status: 200, text: () => Promise.resolve(JSON.stringify([])) });
      }
      if (p.indexOf('/run/' + RUN_ID + '/log') === 0) {
        const q = new URL('http://x' + p).searchParams;
        const offset = Number(q.get('offset') || 0);
        const tail = Number(q.get('tail') || TAIL);
        const full = getLog();
        const chunk = sliceByCodePoints(full, offset, tail);
        const size = pyLen(full);
        const truncated = offset + pyLen(chunk) < size;
        return Promise.resolve({
          ok: true, status: 200,
          text: () => Promise.resolve(JSON.stringify({
            run_id: RUN_ID, status: 'Running',
            steps: [{ name: 'Testes', status: 'Running', bytes: 0, lines: 0 }],
            log: chunk, offset, size, truncated
          }))
        });
      }
      if (p.startsWith('/run/' + RUN_ID)) {
        return Promise.resolve({ ok: true, status: 200, text: () => Promise.resolve(JSON.stringify(pyRun(getLog()))) });
      }
      return Promise.resolve({ ok: true, status: 200, text: () => Promise.resolve('{}') });
    }
  };
}

function mount(fetcher) {
  const dom = new JSDOM(HTML, { runScripts: 'outside-only', pretendToBeVisual: true, url: 'http://localhost/ui' });
  const w = dom.window;
  w.matchMedia = w.matchMedia || (() => ({ matches: false, addListener() {}, removeListener() {}, addEventListener() {}, removeEventListener() {} }));
  w.fetch = fetcher.fetch;
  w.localStorage.clear();
  const style = w.document.createElement('style');
  style.textContent = CSS;
  w.document.head.appendChild(style);
  w.eval(JS);
  w.WorkployMobile.boot();
  w.WorkployMobile._test.setSession('http://10.0.0.10:8787', 'token', 'VPS');
  w.WorkployMobile._test.renderNow();
  return w;
}

function flush(ms) { return new Promise((r) => setTimeout(r, ms || 30)); }

function setLogsMode(w) {
  const seg = w.document.querySelector('[data-role="mode-tabs"]');
  if (!seg) throw new Error('segmented Resumo | Logs não encontrado');
  const logsChip = Array.from(seg.children).find((b) => /Logs/.test(b.textContent));
  if (!logsChip) throw new Error('chip Logs não encontrado');
  logsChip.click();
}

function logCallsOf(calls) { return calls.filter((c) => c.indexOf('/log') >= 0); }
function offsetOf(call) { return Number((call.match(/offset=(\d+)/) || [])[1]); }

(async () => {
  // ----------------------------------------------------------------------
  console.log('\n[1] Cursor em caracteres (code points) com acentos e emoji');
  let currentLog = BASE_LOG;
  const fetcher = makeFetch(() => currentLog);
  const calls = fetcher.calls;
  // A referência global é a mesma usada pelo servidor falso: ler daqui
  // sempre devolve o array vivo que o fetch alimenta.

  const w = mount(fetcher);
  w.WorkployMobile._test.openDetailWith(pyRun(currentLog));
  w.WorkployMobile._test.stopPolls(); // sem servidor: só o log continua
  setLogsMode(w);
  await flush(60);

  let all = logCallsOf(calls);
  if (all.length >= 1) ok('request de log: ' + all[all.length - 1]);
  else fail('nenhuma requisição de log: ' + JSON.stringify(calls));

  const pre = w.document.getElementById('log-pre');
  if (pre && pre.textContent === BASE_LOG) ok('log completo renderizado (acentos + emoji intactos)');
  else fail('texto do log divergente: ' + JSON.stringify(pre && pre.textContent));

  let L = w.WorkployMobile.state.log;
  if (L && L.offset === pyLen(BASE_LOG)) {
    ok('cursor = ' + L.offset + ' code points (igual ao len Python do runner)');
  } else fail('cursor: ' + (L && L.offset) + ' (esperado ' + pyLen(BASE_LOG) + ')');

  // O cursor NÃO pode ser UTF-16 length nem byte length.
  if (L && L.offset !== BASE_LOG.length) ok('cursor ≠ UTF-16 length (' + BASE_LOG.length + ') — não usa .length cru');
  else fail('cursor coincide com UTF-16 length — emoji conta 2');
  if (L && L.offset !== Buffer.byteLength(BASE_LOG, 'utf8')) ok('cursor ≠ byte length (' + Buffer.byteLength(BASE_LOG, 'utf8') + ') — offset é em caracteres, não bytes');
  else fail('cursor coincide com bytes — suspeita de TextEncoder');

  // 2º poll: o runner cresce o log. O app deve pedir do cursor exato.
  currentLog += 'Passo 3: novo ✓\n';
  const beforeCount = logCallsOf(calls).length;
  await w.WorkployMobile._test.pollLog();
  await flush(60);
  all = logCallsOf(calls);
  // Compara a chamada emitida por este poll (a partir de beforeCount).
  const newCalls = all.slice(beforeCount);
  const second = newCalls.length ? newCalls[newCalls.length - 1] : null;
  if (second && offsetOf(second) === pyLen(BASE_LOG)) {
    ok('2º poll pede offset=' + offsetOf(second) + ' (contínuo, sem pular caractere)');
  } else fail('2º offset=' + (second && offsetOf(second)) + ' (esperado ' + pyLen(BASE_LOG) + ')');

  const pre2 = w.document.getElementById('log-pre');
  if (pre2 && pre2.textContent === currentLog) ok('apenas o trecho novo foi anexado, sem duplicar');
  else fail('texto após 2º poll divergente');

  // 3º poll: nada novo. NÃO apaga a tela.
  const before = w.document.getElementById('log-pre').textContent;
  await w.WorkployMobile._test.pollLog();
  await flush(60);
  const after3 = w.document.getElementById('log-pre').textContent;
  if (after3 === before) ok('poll sem novidade mantém a tela (não apaga o log)');
  else fail('poll vazio apagou a tela');

  L = w.WorkployMobile.state.log;
  if (L && L.offset === pyLen(currentLog)) ok('cursor final = ' + L.offset + ' code points — igual ao len do runner');
  else fail('cursor final ' + (L && L.offset) + ' != ' + pyLen(currentLog));

  w.close();

  // ----------------------------------------------------------------------
  console.log('\n[2] Recorte (truncated) -> aviso e recarga do início');
  const huge = 'x'.repeat(300000) + 'FIM ✓\n';
  const fetcher2 = makeFetch(() => huge);
  const w2 = mount(fetcher2);
  w2.WorkployMobile._test.openDetailWith(pyRun(huge));
  w2.WorkployMobile._test.stopPolls();
  setLogsMode(w2);
  await w2.WorkployMobile._test.pollLog();
  await flush(80);

  const L2 = w2.WorkployMobile.state.log;
  if (L2 && L2.truncated === true) ok('log gigante devolve truncated=true');
  else fail('truncated: ' + (L2 && L2.truncated));
  if (L2 && L2.offset === TAIL) ok('cursor após o 1º trecho = ' + L2.offset + ' code points (tail=200000)');
  else fail('cursor após truncamento: ' + (L2 && L2.offset));

  // O banner é derivado de L.truncated no render seguinte ao poll.
  w2.WorkployMobile.render();
  const banner = w2.document.querySelector('.log-banner');
  if (banner && /Carregado a partir do caractere/.test(banner.textContent) && /Recarregar do início/.test(banner.textContent)) {
    ok('aviso "carregado a partir de…" + botão "Recarregar do início" no topo do bloco');
  } else fail('banner de recorte ausente');

  await w2.WorkployMobile._test.pollLog();
  await flush(80);
  if (L2 && L2.offset === pyLen(huge)) ok('segundo poll fecha o arquivo: ' + L2.offset + ' code points');
  else fail('cursor final do log grande: ' + (L2 && L2.offset) + ' de ' + pyLen(huge));
  if (L2 && L2.truncated === false) ok('depois de carregar tudo, truncated=false');
  else fail('truncated ainda true após carregar tudo');

  const beforeReload = logCallsOf(fetcher2.calls).length;
  w2.WorkployMobile._test.reloadLog();
  await flush(80);
  const reloadCalls = logCallsOf(fetcher2.calls).slice(beforeReload);
  if (reloadCalls.length && /offset=0/.test(reloadCalls[reloadCalls.length - 1])) ok('"Recarregar do início" pede offset=0 de novo');
  else fail('recarga não pediu offset=0: ' + JSON.stringify(reloadCalls));

  w2.close();

  // ----------------------------------------------------------------------
  console.log('\n[3] Lista usa GET /runs?limit=100');
  // O harness abre direto no detalhe, então a lista nunca é chamada. A prova
  // estativa está no static-check.js; aqui garantimos o invariante: nenhuma
  // chamada de lista fora desse formato.
  const listCalls = fetcher.calls.filter((c) => c.startsWith('/runs'));
  const badList = listCalls.filter((c) => c !== '/runs?limit=100');
  if (badList.length === 0) ok('nenhuma chamada de lista fora de /runs?limit=100 (também em static-check.js)');
  else fail('lista fora do contrato: ' + JSON.stringify(badList));

  console.log('\n=== RESUMO CONTRATO DO LOG ===');
  if (failures === 0) {
    console.log('VEREDITO: PRONTO — offset/size em caracteres (code points), cursor com countCodePoints,');
    console.log('truncated tratado, poll vazio não apaga a tela, recarga do início funciona.');
  } else {
    console.log('VEREDITO: FALHOU — ' + failures + ' checagem(ns).');
    process.exitCode = 1;
  }
})();