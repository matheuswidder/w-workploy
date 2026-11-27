/* Workploy Mobile — cliente puro do runner da VPS (caminho C do plano).
 * Vanilla, sem dependência, sem CDN. Consome:
 *   GET  /runs?limit=100              (o runner aceita ?limit= 1..100)
 *   GET  /run/{id}
 *   GET  /run/{id}/log?offset=N&tail=200000[&step=idx]
 *   POST /trigger
 *   POST /runs/{id}/approve
 *   POST /runs/{id}/cancel
 *
 * CONTRATO DO LOG: `offset` e `size` são em CARACTERES (o runner faz
 * `offset += len(log)` sobre uma str Python), nunca em bytes. O cursor da
 * página é `String.length` do JS; TextEncoder/byte length não entram aqui.
 *
 * Contrato de erro tratado honestamente: 401 (token inválido), 404 (rota ou
 * run inexistente), erro de rede e timeout. Nada é inventado quando falta dado.
 */
(function () {
  'use strict';

  // --------------------------------------------------------------------
  // Constantes / configuração
  // --------------------------------------------------------------------

  var LS_SESSION = 'wp.mobile.session.v1';   // { base, token, server }
  var LS_LAST_FILTER = 'wp.mobile.filter.v1';
  var LS_PREFS = 'wp.mobile.prefs.v1';       // { reduziuMotion? autoFollow? }
  var LS_RUNS_CACHE = 'workploy.cache.runs.v1'; // { savedAt: epoch ms, runs: [...] }
  var RUNS_CACHE_MAX_BYTES = 512 * 1024;
  var RUNS_CACHE_MAX_RUNS = 100;
  var RUNS_CACHE_TRIM_RUNS = 50;
  var RUNS_CACHE_STALE_MS = 24 * 60 * 60 * 1000;
  var POLL_MS = 2000;
  var FETCH_TIMEOUT = 12000;

  // Densidade do gate 9 do §12: o Ubuntu Touch entrega DPR 1 com 1080 px
  // físicos. A v2 do plano decide que a escala é do app (2,75), não do
  // ambiente. Esta página escolhe um tema claro/escuro com scheme nativo;
  // quando a resolução física é alta e o DPR é baixo, aplicamos `zoom` para
  // que 393 CSS px continuem sendo 393 CSS px de trabalho.
  var DENSITY_TARGET_PX = 393;   // largura lógica de referência (§2.1)
  var DENSITY_MIN_PHYSICAL = 900; // abaixo disso a tela é pequena demais
  var DENSITY_MAX_NATIVE_DPR = 1.5;

  // --------------------------------------------------------------------
  // Estado em memória
  // --------------------------------------------------------------------

  var state = {
    base: '',
    token: '',
    server: '',                // rótulo humano do servidor
    view: 'runs',              // runs | projects | approvals | connections | detail | log
    filter: 'all',             // all | active | failures
    lastRuns: null,            // último Run[] bem recebido (ou restaurado do cache)
    lastError: null,           // { kind, status, message }  (para mostrar cache honesto)
    lastFetchedAt: 0,
    fromCache: false,          // true só quando a lista veio do localStorage
    cacheSavedAt: 0,           // epoch ms do cache em uso (quando fromCache)
    cacheTruncated: false,     // true quando o cache passou do teto e foi cortado
    detail: null,              // { id, run, error }
    log: null,                 // { id, step, text, offset, truncated, run, stepMeta }
    pollTimer: null,
    visible: true,
    busyActions: {},           // { 'approve:<id>': true }
    cache: {}                  // cache por chave localStorage (não usado pelo app; placeholder)
  };

  // --------------------------------------------------------------------
  // Utilidades de DOM
  // --------------------------------------------------------------------

  function $(sel, root) { return (root || document).querySelector(sel); }
  function el(tag, props, children) {
    var node = document.createElement(tag);
    if (props) {
      for (var k in props) {
        if (!Object.prototype.hasOwnProperty.call(props, k)) continue;
        var v = props[k];
        if (v == null || v === false) continue;
        if (k === 'text') { node.textContent = v; }
        else if (k === 'html') { node.innerHTML = v; }
        else if (k === 'class') { node.className = v; }
        else if (k === 'dataset') { Object.keys(v).forEach(function (dk) { node.dataset[dk] = v[dk]; }); }
        else if (k === 'style') { node.setAttribute('style', v); }
        else if (k.indexOf('on') === 0 && typeof v === 'function') {
          node.addEventListener(k.slice(2).toLowerCase(), v);
        } else if (v === true) { node.setAttribute(k, ''); }
        else { node.setAttribute(k, v); }
      }
    }
    if (children != null) {
      if (typeof children === 'string') node.textContent = children;
      else if (Array.isArray(children)) children.forEach(function (c) {
        if (c == null) return;
        node.appendChild(typeof c === 'string' ? document.createTextNode(c) : c);
      });
      else node.appendChild(children);
    }
    return node;
  }

  // --------------------------------------------------------------------
  // Ícones (vocabulário do desktop, sem dependência externa)
  // --------------------------------------------------------------------
  var ICONS = {
    runs: '<path d="M3 5.5h18M3 12h18M3 18.5h11" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"/>',
    projects: '<g fill="currentColor"><path d="M6 3.5A2.5 2.5 0 0 0 3.5 6v12A2.5 2.5 0 0 0 6 20.5h3.75a.75.75 0 0 0 0-1.5H6a1 1 0 0 1-1-1V7h14v1.25a.75.75 0 0 0 1.5 0V6A2.5 2.5 0 0 0 18 3.5H6Z"/><path d="M8 16.5a6 6 0 1 1 12 0 5.98 5.98 0 0 1-1.76 4.27.75.75 0 0 1-1.06-1.06A4.5 4.5 0 1 0 14 21.75a.75.75 0 0 1 0 1.5 6 6 0 0 1-6-6.75Z"/></g>',
    approvals: '<g fill="currentColor"><path d="M12 2a4 4 0 0 0-4 4v1.1A4 4 0 0 0 4.5 11v8A3 3 0 0 0 7.5 22h9a3 3 0 0 0 3-3v-8A4 4 0 0 0 16 7.1V6a4 4 0 0 0-4-4Zm-2.5 5V6a2.5 2.5 0 1 1 5 0v1h-5Zm5.78 3.97a.75.75 0 0 1 0 1.06l-4 4a.75.75 0 0 1-1.06 0l-2-2a.75.75 0 1 1 1.06-1.06l1.47 1.47 3.47-3.47a.75.75 0 0 1 1.06 0Z"/></g>',
    connections: '<path d="M14.5 8.5a3 3 0 1 1-4.28 2.72l-5.47 5.47a1 1 0 1 0 1.42 1.42l5.47-5.47A3 3 0 0 1 14.5 8.5Zm2.1-4.6a3.5 3.5 0 0 1 3.5 3.5h1.3a.75.75 0 0 1 0 1.5h-1.3a3.5 3.5 0 0 1-6.93.73l-1.5 1.5a.75.75 0 1 1-1.06-1.06l1.5-1.5a3.5 3.5 0 0 1 4.5-4.67ZM3.6 13.4a3.5 3.5 0 0 0 3.5 3.5h1.3a.75.75 0 0 0 0-1.5h-1.3a2 2 0 1 1 1.97-2.32.75.75 0 0 0 1.48-.24A3.5 3.5 0 0 0 3.6 13.4Z" fill="currentColor"/>',
    play: '<path d="M7.2 4.8C6.4 4.35 5.4 4.92 5.4 5.84V18.16C5.4 19.08 6.4 19.65 7.2 19.2L18.4 13.04C19.22 12.58 19.22 11.42 18.4 10.96L7.2 4.8Z" fill="currentColor"/>',
    refresh: '<path d="M20 6.5V11.5H15" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/><path d="M19.2 11.2A7.6 7.6 0 1 0 17.8 17.2" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"/>',
    check: '<path fill="currentColor" fill-rule="evenodd" d="M12 22C17.5228 22 22 17.5228 22 12C22 6.47715 17.5228 2 12 2C6.47715 2 2 6.47715 2 12C2 17.5228 6.47715 22 12 22ZM16.0303 8.96967C16.3232 9.26256 16.3232 9.73744 16.0303 10.0303L11.0303 15.0303C10.7374 15.3232 10.2626 15.3232 9.96967 15.0303L7.96967 13.0303C7.67678 12.7374 7.67678 12.2626 7.96967 11.9697C8.26256 11.6768 8.73744 11.6768 9.03033 11.9697L10.5 13.4393L14.9697 8.96967C15.2626 8.67678 15.7374 8.67678 16.0303 8.96967Z"/>',
    x: '<path fill="currentColor" fill-rule="evenodd" d="M12 22C17.5228 22 22 17.5228 22 12C22 6.47715 17.5228 2 12 2C6.47715 2 2 6.47715 2 12C2 17.5228 6.47715 22 12 22ZM8.96967 8.96967C9.26256 8.67678 9.73744 8.67678 10.0303 8.96967L12 10.9393L13.9697 8.96967C14.2626 8.67678 14.7374 8.67678 15.0303 8.96967C15.3232 9.26256 15.3232 9.73744 15.0303 10.0303L13.0607 12L15.0303 13.9697C15.3232 14.2626 15.3232 14.7374 15.0303 15.0303C14.7374 15.3232 14.2626 15.3232 13.9697 15.0303L12 13.0607L10.0303 15.0303C9.73744 15.3232 9.26256 15.3232 8.96967 15.0303C8.67678 14.7374 8.67678 14.2626 8.96967 13.9697L10.9393 12L8.96967 10.0303C8.67678 9.73744 8.67678 9.26256 8.96967 8.96967Z"/>',
    clock: '<path fill="currentColor" fill-rule="evenodd" d="M12 22C17.5228 22 22 17.5228 22 12C22 6.47715 17.5228 2 12 2C6.47715 2 2 6.47715 2 12C2 17.5228 6.47715 22 12 22ZM12.75 8C12.75 7.58579 12.4142 7.25 12 7.25C11.5858 7.25 11.25 7.58579 11.25 8V12C11.25 12.1989 11.329 12.3897 11.4697 12.5303L13.9697 15.0303C14.2626 15.3232 14.7374 15.3232 15.0303 15.0303C15.3232 14.7374 15.3232 14.2626 15.0303 13.9697L12.75 11.6893V8Z"/>',
    git: '<circle cx="7" cy="5.5" r="2.7" fill="currentColor"/><circle cx="7" cy="18.5" r="2.7" fill="currentColor"/><circle cx="17.5" cy="8.5" r="2.7" fill="currentColor"/><path d="M7 8.2V15.8M7 15.2C7 11.8 17.5 13.5 17.5 11.2" stroke="currentColor" stroke-width="2.1" stroke-linecap="round"/>',
    bolt: '<path d="M13.6 2.4L5.6 13.2H11.6L10.2 21.6L18.4 10.8H12.4L13.6 2.4Z" fill="currentColor"/>',
    server: '<path fill="currentColor" fill-rule="evenodd" d="M4.6 3h14.8c.9 0 1.6.75 1.6 1.7v4.6c0 .95-.7 1.7-1.6 1.7H4.6A1.65 1.65 0 0 1 3 9.3V4.7C3 3.75 3.7 3 4.6 3Zm2.5 5A1.25 1.25 0 1 0 5.85 6.75 1.25 1.25 0 0 0 6.85 8ZM4.6 12.5h14.8c.9 0 1.6.75 1.6 1.7v4.6c0 .95-.7 1.7-1.6 1.7H4.6A1.65 1.65 0 0 1 3 18.8v-4.6c0-.95.7-1.7 1.6-1.7Zm3.85 4A1.25 1.25 0 1 0 7.2 15.25 1.25 1.25 0 0 0 8.45 16.5Z"/>',
    layers: '<path d="M12 2.5 2.6 6.9c-.4.2-.4.8 0 1L12 12.5l9.4-4.6c.4-.2.4-.8 0-1L12 2.5Z" fill="currentColor"/><path d="M3.1 11.8 12 16.2l8.9-4.4c.9-.4 1.7.9.8 1.4L12 18.2 2.3 13.2c-.9-.5-.1-1.8.8-1.4Z" fill="currentColor" opacity=".7"/><path d="M3.1 17 12 21.4 20.9 17c.9-.4 1.7.9.8 1.4L12 23.4 2.3 18.4c-.9-.5-.1-1.8.8-1.4Z" fill="currentColor" opacity=".4"/>',
    home: '<path d="M12 3 2.8 10.5a1 1 0 1 0 1.3 1.5L5 11.3V20a1 1 0 0 0 1 1h4.2v-5h3.6v5H18a1 1 0 0 0 1-1v-8.7l.9.7a1 1 0 0 0 1.3-1.5L12 3Z" fill="currentColor"/>',
    back: '<path d="M15.5 5 8.5 12l7 7" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/>',
    chev: '<path d="M9 6l6 6-6 6" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/>',
    dots: '<circle cx="6" cy="12" r="1.8" fill="currentColor"/><circle cx="12" cy="12" r="1.8" fill="currentColor"/><circle cx="18" cy="12" r="1.8" fill="currentColor"/>',
    arrowDown: '<path d="M12 4v14m0 0-5-5m5 5 5-5" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/>',
    wrap: '<path d="M4 6h16M4 12h11a3 3 0 1 1 0 6h-3m0 0 2-2m-2 2 2 2M4 18h4" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"/>',
    copy: '<rect x="8" y="3.5" width="12.5" height="13.5" rx="2.4" stroke="currentColor" stroke-width="2"/><rect x="3.5" y="7" width="12.5" height="13.5" rx="2.4" stroke="currentColor" stroke-width="2"/>',
    key: '<path fill="currentColor" fill-rule="evenodd" d="M15.5 2a6.5 6.5 0 0 0-6.14 8.63L2.3 17.7a1 1 0 0 0-.3.71V20a1 1 0 0 0 1 1h2a1 1 0 0 0 1-1v-1h1a1 1 0 0 0 1-1v-1h1.59l1.7-1.7A6.5 6.5 0 1 0 15.5 2Zm2 4.25a1.75 1.75 0 1 1 0 3.5 1.75 1.75 0 0 1 0-3.5Z"/>',
    warn: '<path fill="currentColor" fill-rule="evenodd" d="M12 2 1.5 21h21L12 2Zm1 6.5h-2v6h2v-6Zm-1 9.75a1.25 1.25 0 1 0 0-2.5 1.25 1.25 0 0 0 0 2.5Z"/>',
    info: '<path fill="currentColor" fill-rule="evenodd" d="M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20Zm1-14.5v2h-2v-2h2ZM9 11h2.5v5.5H13V18H9v-1.5h1.5V12.5H9V11Z"/>'
  };
  function icon(name, extraClass) {
    var span = document.createElement('span');
    span.className = 'ico' + (extraClass ? ' ' + extraClass : '');
    span.setAttribute('aria-hidden', 'true');
    span.innerHTML = '<svg viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">' +
      ICONS[name] + '</svg>';
    return span;
  }

  // --------------------------------------------------------------------
  // Formatação BR (data dd/mm/aaaa, fuso America/Sao_Paulo, BRL quando couber)
  // --------------------------------------------------------------------
  var TZ = 'America/Sao_Paulo';
  function pad2(n) { return n < 10 ? '0' + n : '' + n; }
  function fmtDate(tsOrStr) {
    if (!tsOrStr) return '—';
    var d;
    if (typeof tsOrStr === 'number') d = new Date(tsOrStr * 1000);
    else d = new Date(tsOrStr);
    if (isNaN(d.getTime())) return '—';
    try {
      var parts = new Intl.DateTimeFormat('pt-BR', {
        timeZone: TZ, day: '2-digit', month: '2-digit', year: 'numeric',
        hour: '2-digit', minute: '2-digit'
      }).formatToParts(d);
      var o = {};
      parts.forEach(function (p) { o[p.type] = p.value; });
      return o.day + '/' + o.month + '/' + o.year + ' ' + o.hour + ':' + o.minute;
    } catch (_) {
      return pad2(d.getDate()) + '/' + pad2(d.getMonth() + 1) + '/' + d.getFullYear() +
        ' ' + pad2(d.getHours()) + ':' + pad2(d.getMinutes());
    }
  }
  function fmtShort(ts) {
    var full = fmtDate(ts);
    if (full === '—') return full;
    var parts = full.split(' ');
    var ymd = parts[0].split('/');
    return ymd.length === 3 ? ymd[0] + '/' + ymd[1] + ' ' + parts[1] : full;
  }
  function fmtAge(ts) {
    if (!ts) return '';
    var secs = Math.max(0, Math.floor(Date.now() / 1000 - ts));
    if (secs < 60) return 'agora';
    var min = Math.floor(secs / 60);
    if (min < 60) return min + ' min';
    var h = Math.floor(min / 60);
    if (h < 24) return h + ' h';
    return Math.floor(h / 24) + ' d';
  }
  function fmtDur(secs) {
    if (typeof secs !== 'number' || !isFinite(secs) || secs <= 0) return '';
    var s = Math.round(secs);
    if (s < 60) return s + ' s';
    var m = Math.floor(s / 60), rs = s % 60;
    if (m < 60) return m + ' min' + (rs ? ' ' + rs + ' s' : '');
    var h = Math.floor(m / 60);
    return h + ' h ' + (m % 60) + ' min';
  }
  function fmtBytes(n) {
    if (!n && n !== 0) return '';
    if (n < 1024) return n + ' B';
    if (n < 1024 * 1024) return (n / 1024).toFixed(1).replace('.', ',') + ' KB';
    return (n / (1024 * 1024)).toFixed(1).replace('.', ',') + ' MB';
  }
  function fmtPct(n) {
    return Math.round(n) + '';
  }
  function safe(v) { return v == null ? '' : String(v); }

  // --------------------------------------------------------------------
  // Status (Run / Step) — rótulos em pt-BR, mesma terminologia do desktop.
  // --------------------------------------------------------------------
  var RUN_LABELS = {
    Queued: 'Na fila',
    Running: 'Executando',
    AwaitingApproval: 'Aguardando aprovação',
    Success: 'Sucesso',
    Failed: 'Falhou',
    Cancelled: 'Cancelado'
  };
  var RUN_TONE = {
    Queued: 'queued',
    Running: 'running',
    AwaitingApproval: 'warning',
    Success: 'success',
    Failed: 'danger',
    Cancelled: 'neutral'
  };
  var STEP_LABELS = {
    Pending: 'Pendente',
    WaitingApproval: 'Aguardando aprovação',
    Running: 'Executando',
    Success: 'Sucesso',
    Failed: 'Falhou',
    Skipped: 'Ignorado'
  };
  var STEP_TONE = {
    Pending: 'neutral',
    WaitingApproval: 'warning',
    Running: 'running',
    Success: 'success',
    Failed: 'danger',
    Skipped: 'neutral'
  };

  function pillRun(status) {
    var tone = RUN_TONE[status] || 'neutral';
    var p = el('span', { class: 'pill pill--' + tone });
    if (status === 'Success') p.appendChild(icon('check', 'pill__ico'));
    else if (status === 'Failed' || status === 'Cancelled') p.appendChild(icon('x', 'pill__ico'));
    else if (status === 'AwaitingApproval') p.appendChild(icon('bolt', 'pill__ico'));
    else p.appendChild(el('span', { class: 'pill__dot' }));
    p.appendChild(el('span', { text: RUN_LABELS[status] || (status || '—') }));
    return p;
  }
  function pillStep(status) {
    var tone = STEP_TONE[status] || 'neutral';
    var p = el('span', { class: 'pill pill--' + tone });
    if (status === 'Success') p.appendChild(icon('check', 'pill__ico'));
    else if (status === 'Failed') p.appendChild(icon('x', 'pill__ico'));
    else if (status === 'WaitingApproval') p.appendChild(icon('bolt', 'pill__ico'));
    else p.appendChild(el('span', { class: 'pill__dot' }));
    p.appendChild(el('span', { text: STEP_LABELS[status] || (status || '—') }));
    return p;
  }

  // --------------------------------------------------------------------
  // Progresso derivado dos steps (espelha run_progress_frac do model.rs)
  // --------------------------------------------------------------------
  function stepDone(st) {
    return st.status === 'Success' || st.status === 'Skipped';
  }
  function runProgress(run) {
    var steps = (run && run.steps) || [];
    if (steps.length === 0) return 0;
    var done = steps.filter(stepDone).length;
    var frac = done / steps.length;
    var active = steps.find(function (s) { return s.status === 'Running'; });
    if (active && active.substeps && active.substeps.length > 0) {
      var subDone = active.substeps.filter(stepDone).length;
      frac += (subDone / active.substeps.length) / steps.length;
    }
    return Math.max(0, Math.min(1, frac));
  }
  function progressTone(status) {
    if (status === 'Success') return 'is-success';
    if (status === 'Failed') return 'is-danger';
    if (status === 'AwaitingApproval') return 'is-warning';
    return '';
  }

  // --------------------------------------------------------------------
  // Densidade — gate 9 do §12
  // --------------------------------------------------------------------
  // O aparelho do maestro (Poco X5 / Ubuntu Touch) mede 1080 px físicos com
  // `devicePixelRatio = 1`. Um layout de 393 dp em 1080 px CSS 1:1 vira
  // formiga. A v2 do plano resolve isso declarando a escala no app. Aqui:
  //   fator = physical / 393, arredondado a 2 casas (1080 / 393 ≈ 2.75)
  // e aplicado como `zoom`. O CSS continua com 48 dp de alvo; o zoom faz o
  // resto e `getBoundingClientRect` devolve o retângulo já multiplicado.
  // É idempotente: recalcular com o mesmo ambiente não muda nada.
  var densityApplied = null;
  function applyDensity() {
    var root = document.documentElement;
    if (!root) return null;
    var dpr = window.devicePixelRatio || 1;
    var viewportWidth = window.innerWidth || root.clientWidth || 0;
    var physical = Math.round(viewportWidth * dpr);
    var forced = physical >= DENSITY_MIN_PHYSICAL && dpr < DENSITY_MAX_NATIVE_DPR;
    if (!forced) {
      // Ambiente já entrega DPR alto ou tela pequena: nada a declarar.
      if (densityApplied !== 'native') {
        root.style.zoom = '';
        root.style.setProperty('--density-zoom', '1');
        densityApplied = 'native';
      }
      root.dataset.density = 'native';
      return { mode: 'native', factor: 1, dpr: dpr, physical: physical };
    }
    var factor = Math.round((physical / DENSITY_TARGET_PX) * 100) / 100;
    if (factor < 1) factor = 1;
    // Idempotência: só toca no DOM quando o fator muda.
    if (densityApplied !== factor) {
      root.style.zoom = String(factor);
      root.style.setProperty('--density-zoom', String(factor));
      densityApplied = factor;
    }
    root.dataset.density = 'forced';
    return { mode: 'forced', factor: factor, dpr: dpr, physical: physical };
  }

  // --------------------------------------------------------------------
  // Sessão (localStorage) — token NUNCA aparece no HTML nem no console.
  // --------------------------------------------------------------------
  function readSession() {
    try {
      // Sessão persistida em localStorage; se o navegador bloquear, tenta
      // sessionStorage — em qualquer caso, nunca há token embutido.
      var raw = localStorage.getItem(LS_SESSION) || sessionStorage.getItem(LS_SESSION);
      if (!raw) return null;
      var data = JSON.parse(raw);
      if (!data || typeof data !== 'object' || !data.base || !data.token) return null;
      return data;
    } catch (_) {
      try {
        var s = sessionStorage.getItem(LS_SESSION);
        return s ? JSON.parse(s) : null;
      } catch (__) { return null; }
    }
  }
  function writeSession(base, token, server) {
    var data = { base: base, token: token, server: server || '' };
    try { localStorage.setItem(LS_SESSION, JSON.stringify(data)); }
    catch (_) { try { sessionStorage.setItem(LS_SESSION, JSON.stringify(data)); } catch (__) {} }
    return data;
  }
  function clearSession() {
    try { localStorage.removeItem(LS_SESSION); } catch (_) {}
    try { sessionStorage.removeItem(LS_SESSION); } catch (_) {}
    state.base = '';
    state.token = '';
    state.server = '';
  }
  function loadSession() {
    var s = readSession();
    if (!s) return false;
    state.base = s.base;
    state.token = s.token;
    state.server = s.server || '';
    return true;
  }

  // --------------------------------------------------------------------
  // HTTP — caminho único com timeout, Authorization: Bearer e classificação de erro
  // --------------------------------------------------------------------
  function httpError(kind, status, message, extra) {
    var e = new Error(message || kind);
    e.kind = kind; e.status = status; e.extra = extra || null;
    // Código de máquina do servidor (ex.: `not_awaiting_approval`), usado
    // para escolher a mensagem humana. Nunca é exibido na tela.
    if (extra && typeof extra === 'object' && extra.code) e.code = extra.code;
    return e;
  }

  function url(path) {
    return state.base.replace(/\/+$/, '') + path;
  }

  function fetchJSON(path, opts) {
    opts = opts || {};
    var init = {
      method: opts.method || 'GET',
      headers: Object.assign(
        { 'Accept': 'application/json', 'Authorization': 'Bearer ' + state.token },
        opts.body ? { 'Content-Type': 'application/json' } : {}
      ),
      cache: 'no-store',
      body: opts.body ? JSON.stringify(opts.body) : undefined
    };
    var controller = null;
    var timer = null;
    if (typeof AbortController !== 'undefined') {
      controller = new AbortController();
      init.signal = controller.signal;
      timer = setTimeout(function () { controller.abort(); }, opts.timeout || FETCH_TIMEOUT);
    }
    return fetch(url(path), init).then(function (res) {
      return res.text().then(function (txt) {
        var data = null;
        if (txt) {
          try { data = JSON.parse(txt); } catch (_) { data = null; }
        }
        if (res.ok) return data;
        var msg = (data && (data.error || data.message)) || '';
        var code = (data && typeof data.code === 'string') ? data.code : '';
        if (res.status === 401) throw httpError('unauthorized', 401, 'Token inválido ou expirado.', { code: code });
        if (res.status === 403) throw httpError('forbidden', 403, msg || 'Sem permissão para essa ação.', { code: code });
        if (res.status === 404) throw httpError('notfound', 404, msg || 'Não encontrado no servidor.', { code: code });
        // O portão devolve 409 quando já foi decidido e 400 para decisão
        // inválida; os dois têm texto próprio na tela.
        if (res.status === 409) throw httpError('conflict', 409, msg || '', { code: code });
        if (res.status === 400) throw httpError('invalid', 400, msg || '', { code: code });
        throw httpError('http', res.status, msg || ('Resposta ' + res.status + ' do servidor.'), { code: code });
      });
    }).catch(function (err) {
      if (timer) clearTimeout(timer);
      if (err && err.kind) throw err;
      if (err && err.name === 'AbortError') throw httpError('timeout', 0, 'O servidor demorou para responder.');
      // resposta não-JSON / fetch rejeitado / DNS / rede
      throw httpError('network', 0, 'Sem resposta do servidor. Verifique a rede.');
    }).then(function (v) {
      if (timer) clearTimeout(timer);
      return v;
    });
  }

  // --------------------------------------------------------------------
  // Toast (topo, largura cheia menos 16; 4 s; toque descarta)
  // --------------------------------------------------------------------
  var toastTimers = [];
  function toast(msg, forceTone) {
    var host = $('#toast-host');
    if (!host) return;
    var t = el('div', { class: 'toast' + (forceTone ? ' toast--' + forceTone : ''), text: msg });
    t.setAttribute('role', forceTone === 'error' ? 'alert' : 'status');
    var remove = function () {
      if (!t.parentNode) return;
      t.parentNode.removeChild(t);
    };
    t.addEventListener('click', remove);
    t.addEventListener('touchend', remove);
    host.appendChild(t);
    var id = setTimeout(remove, 4000);
    toastTimers.push(id);
    if (toastTimers.length > 12) {
      clearTimeout(toastTimers.shift());
    }
  }

  // --------------------------------------------------------------------
  // Prompt de confirmação — SEMPRE em sheet (§8.4: aprovar não é 1 toque)
  // --------------------------------------------------------------------
  var activeSheet = null;
  function openSheet(node, opts) {
    opts = opts || {};
    closeSheet();
    var overlay = el('div', { class: 'sheet-overlay' });
    var sheet = el('div', { class: 'sheet', role: 'dialog', 'aria-modal': 'true', 'aria-label': opts.label || 'Ação' });
    sheet.appendChild(el('div', { class: 'sheet__handle', 'aria-hidden': 'true' }));
    sheet.appendChild(node);
    overlay.appendChild(sheet);
    activeSheet = { overlay: overlay, lastFocus: document.activeElement };

    if (opts.dismissible !== false) {
      overlay.addEventListener('click', function (e) {
        if (e.target === overlay) closeSheet();
      });
    }
    document.getElementById('sheet-host').appendChild(overlay);
    document.body.classList.add('is-locked');
    if (opts.onMount) opts.onMount(sheet);
    // foco no primeiro elemento focável dentro do sheet
    var focusable = sheet.querySelector('[data-autofocus], button, input, select, textarea, [href]');
    if (focusable) setTimeout(function () { try { focusable.focus(); } catch (_) {} }, 30);
    return { overlay: overlay, sheet: sheet };
  }
  function closeSheet() {
    if (!activeSheet) return;
    var ov = activeSheet.overlay;
    if (ov && ov.parentNode) ov.parentNode.removeChild(ov);
    document.body.classList.remove('is-locked');
    var back = activeSheet.lastFocus;
    activeSheet = null;
    if (back && back.focus) { try { back.focus(); } catch (_) {} }
  }
  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape' && activeSheet) closeSheet();
  });

  // --------------------------------------------------------------------
  // Ações do runner
  // --------------------------------------------------------------------
  function apiTrigger(body) {
    return fetchJSON('/trigger', { method: 'POST', body: body });
  }
  // Aprovação de portão. O runner aceita
  //   POST /runs/<id>/approve  { "decision": "approve"|"reject", "note": "..." }
  // `empty note` vai como "" — o runner grava o campo e não fica sem valor.
  function apiApprove(id, decision, note) {
    var body = {
      decision: decision === 'reject' ? 'reject' : 'approve',
      note: typeof note === 'string' ? note : ''
    };
    return fetchJSON('/runs/' + encodeURIComponent(id) + '/approve', { method: 'POST', body: body });
  }
  function apiCancel(id) {
    return fetchJSON('/runs/' + encodeURIComponent(id) + '/cancel', { method: 'POST' });
  }

  // --------------------------------------------------------------------
  // Templates de estado
  // --------------------------------------------------------------------
  function stateBlock(kind, title, msg, actions) {
    var wrap = el('div', { class: 'state' + (kind ? ' state--' + kind : '') });
    wrap.appendChild(icon(kind === 'error' ? 'warn' : kind === 'warn' ? 'warn' : 'info', 'state__icon'));
    wrap.appendChild(el('p', { class: 'state__title', text: title }));
    if (msg) wrap.appendChild(el('p', { class: 'state__msg', text: msg }));
    if (actions && actions.length) {
      var box = el('div', { class: 'state__actions' });
      actions.forEach(function (a) {
        var b = el('button', {
          class: 'btn btn--' + (a.variant || 'neutral'),
          type: 'button',
          text: a.label
        });
        b.addEventListener('click', a.onClick);
        box.appendChild(b);
      });
      wrap.appendChild(box);
    }
    return wrap;
  }
  function skeletonList(n) {
    var box = el('div', { class: 'skeletons' });
    for (var i = 0; i < (n || 3); i++) box.appendChild(el('div', { class: 'skeleton' }));
    return box;
  }

  // --------------------------------------------------------------------
  // Header (sticky) — título + ação primária no máximo
  // --------------------------------------------------------------------
  function headerRow(title, opts) {
    opts = opts || {};
    var head = el('div', { class: 'header' });
    var row = el('div', { class: 'header__row' });
    if (opts.back) {
      var back = el('button', {
        class: 'icon-btn',
        type: 'button',
        'aria-label': 'Voltar',
        title: 'Voltar'
      });
      back.appendChild(icon('back'));
      back.addEventListener('click', opts.back);
      row.appendChild(back);
    }
    row.appendChild(el('h1', { class: 'header__title', text: title }));
    if (opts.settings) {
      var set = el('button', {
        class: 'icon-btn',
        type: 'button',
        'aria-label': 'Ajustes e conexão',
        title: 'Ajustes e conexão'
      });
      set.appendChild(icon('dots'));
      set.addEventListener('click', openSettingsSheet);
      row.appendChild(set);
    }
    if (opts.action) {
      var btn = el('button', {
        class: 'icon-btn',
        type: 'button',
        'aria-label': opts.action.label,
        title: opts.action.label
      });
      btn.appendChild(icon(opts.action.icon || 'refresh'));
      if (opts.action.busy) btn.disabled = true;
      btn.addEventListener('click', opts.action.onClick);
      row.appendChild(btn);
    }
    head.appendChild(row);
    if (opts.sub) head.appendChild(el('p', { class: 'header__sub', text: opts.sub }));
    return head;
  }

  // --------------------------------------------------------------------
  // TELA: Execuções
  // --------------------------------------------------------------------
  var FILTERS = [
    { id: 'all', label: 'Todos' },
    { id: 'active', label: 'Ativos' },
    { id: 'failures', label: 'Falhas' }
  ];

  function activeCount(runs) {
    return (runs || []).filter(function (r) {
      return r.status === 'Queued' || r.status === 'Running' || r.status === 'AwaitingApproval';
    }).length;
  }
  function approvalCount(runs) {
    return (runs || []).filter(function (r) { return r.status === 'AwaitingApproval'; }).length;
  }

  function filterRuns(runs, filter) {
    if (!runs) return [];
    if (filter === 'active') return runs.filter(function (r) {
      return r.status === 'Queued' || r.status === 'Running' || r.status === 'AwaitingApproval';
    });
    if (filter === 'failures') return runs.filter(function (r) { return r.status === 'Failed'; });
    return runs;
  }

  function runCardBody(r) {
    var wrap = el('button', { class: 'run-card', type: 'button' });
    var status = r.status || '';
    var head = el('div', { class: 'run-card__head' });
    head.appendChild(pillRun(status));
    var meta = el('div', { class: 'run-card__meta' });
    var rightBits = [];
    if (r.number) rightBits.push('#' + r.number);
    if (r.started_ts) {
      var t = fmtShort(r.started_ts);
      if (t && t !== '—') rightBits.push(t);
    } else if (r.started_at) {
      rightBits.push(r.started_at);
    } else {
      rightBits.push('—');
    }
    meta.appendChild(el('span', { text: rightBits.join(' · ') }));
    if (status === 'Running') {
      var pct = Math.round(runProgress(r) * 100);
      meta.appendChild(el('span', { text: pct + '%' }));
    } else if (r.secs) {
      var dur = fmtDur(r.secs);
      if (dur) meta.appendChild(el('span', { class: 'sep', text: '·' }));
      if (dur) meta.appendChild(el('span', { text: dur }));
    }
    head.appendChild(el('span', { style: 'flex:1 1 auto' }));
    wrap.appendChild(head);

    var name = r.workflow_name || r.title || 'Fluxo';
    var title = el('h2', { class: 'run-card__title', text: name });
    title.setAttribute('title', name);
    wrap.appendChild(title);

    var subBits = [];
    if (r.title && r.title !== name) subBits.push(r.title);
    if (r.branch) subBits.push(r.branch);
    if (r.repo_name) subBits.push(r.repo_name);
    if (r.actor && r.actor !== 'workploy-runner' && r.actor !== 'manual') subBits.push(r.actor);
    var sub = el('div', { class: 'run-card__meta' });
    sub.appendChild(el('span', { text: subBits.length ? subBits.join(' · ') : (r.trigger_label || '') || 'Sem detalhes' }));
    wrap.appendChild(sub);

    var prog = el('div', { class: 'progress', 'aria-hidden': 'true' });
    prog.appendChild(el('div', { class: 'progress__fill ' + progressTone(status) }));
    wrap.appendChild(prog);
    // O valor real é escrito após o append (precisa do elemento no DOM para transição).
    wrap._progressPct = Math.round(runProgress(r) * 100);

    wrap.addEventListener('click', function () { openDetail(r.id); });
    wrap.setAttribute('aria-label',
      'Abrir execução ' + name + (r.number ? ' número ' + r.number : '') +
      ', status ' + (RUN_LABELS[status] || status));
    return wrap;
  }

  function renderRuns(screen) {
    var runs = state.lastRuns;
    var blocked = state.lastError;
    var loading = runs === null && !blocked;

    var head = headerRow('Execuções', {
      settings: true,
      sub: lastLoadedLabel()
    });
    screen.appendChild(head);

    var body = el('div', { class: 'screen__body' });

    // Segmented
    var seg = el('div', { class: 'segmented', role: 'tablist', 'aria-label': 'Filtro de execuções' });
    FILTERS.forEach(function (f) {
      var c = el('button', {
        class: 'chip',
        type: 'button',
        role: 'tab',
        'aria-selected': f.id === state.filter ? 'true' : 'false',
        text: f.label
      });
      c.addEventListener('click', function () {
        state.filter = f.id;
        try { localStorage.setItem(LS_LAST_FILTER, f.id); } catch (_) {}
        rerender();
      });
      seg.appendChild(c);
    });
    body.appendChild(seg);

    if (loading) {
      body.appendChild(skeletonList(3));
    } else if (blocked && !runs) {
      body.appendChild(errorBlockFor(blocked));
    } else if (blocked && runs) {
      // Uma faixa só, honesta: "sem conexão" + o horário do último estado,
      // e a idade quando o cache passa de 24 h. Nunca finge dado vivo.
      body.appendChild(cacheBanner(blocked));
    }

    if (runs) {
      var filtered = filterRuns(runs, state.filter);
      if (runs.length === 0) {
        body.appendChild(stateBlock('', 'Nenhuma execução ainda',
          'Quando um fluxo rodar, ele aparece aqui.', [
            { label: 'Disparar fluxo', variant: 'primary', onClick: openTriggerSheet }
          ]));
      } else if (filtered.length === 0) {
        body.appendChild(stateBlock('', 'Nada neste filtro',
          state.filter === 'active' ? 'Nenhuma execução ativa agora.' : 'Nenhuma execução com falha.',
          [{ label: 'Ver todos', onClick: function () { state.filter = 'all'; rerender(); } }]));
      } else {
        var list = el('div', { class: 'run-list stagger' });
        filtered.forEach(function (r) {
          var item = el('div', { class: 'run-item' });
          var card = runCardBody(r);
          item.appendChild(card);
          list.appendChild(item);
        });
        body.appendChild(list);
        // Escreve a barra de progresso depois do append para a transição valer.
        requestAnimationFrame(function () {
          var fills = list.querySelectorAll('.run-card');
          var cards = filtered;
          for (var i = 0; i < fills.length && i < cards.length; i++) {
            var p = fills[i].querySelector('.progress__fill');
            if (p) p.style.width = cards[i] ? Math.round(runProgress(cards[i]) * 100) + '%' : '0%';
          }
        });
      }
    }

    if (!runs && !blocked) {
      body.appendChild(el('p', { class: 't-small muted', text: 'Carregando execuções…' }));
    }

    screen.appendChild(body);
  }

  // Faixa discreta que acompanha a lista quando a tela mostra estado
  // guardado (gate 8 do §12) ou um poll falhou com dados antigos na tela.
  function cacheBanner(err) {
    var banner = el('div', { class: 'cache-banner', role: 'status' });
    banner.appendChild(icon('warn', 'cache-banner__ico'));
    var text = '';
    if (state.fromCache && state.cacheSavedAt) {
      var when = formatCacheWhen(state.cacheSavedAt);
      text = 'Sem conexão. Mostrando o estado de ' + when + '.';
      var age = Date.now() - state.cacheSavedAt;
      if (age > RUNS_CACHE_STALE_MS) {
        text += ' Esse estado tem ' + formatAgeLong(age) + '.';
      }
      if (state.cacheTruncated) {
        text += ' Só as 50 execuções mais recentes ficaram guardadas.';
      }
    } else {
      text = 'Sem atualização agora. Mostrando o último estado.' + suffixForError(err);
    }
    banner.appendChild(el('p', { class: 'cache-banner__text', text: text }));
    return banner;
  }

  // Horário curto dd/mm HH:mm no fuso do maestro — a data por extenso não cabe.
  function formatCacheWhen(epochMs) {
    try {
      var parts = new Intl.DateTimeFormat('pt-BR', {
        timeZone: TZ, day: '2-digit', month: '2-digit', hour: '2-digit', minute: '2-digit'
      }).formatToParts(new Date(epochMs));
      var o = {};
      parts.forEach(function (p) { o[p.type] = p.value; });
      return o.day + '/' + o.month + ' ' + o.hour + ':' + o.minute;
    } catch (_) {
      var d = new Date(epochMs);
      return pad2(d.getDate()) + '/' + pad2(d.getMonth() + 1) + ' ' + pad2(d.getHours()) + ':' + pad2(d.getMinutes());
    }
  }

  // Idade em linguagem natural, para o cache muito velho não mentir.
  function formatAgeLong(ms) {
    var hours = Math.floor(ms / (60 * 60 * 1000));
    if (hours < 1) return 'menos de 1 hora';
    if (hours < 48) return hours + (hours === 1 ? ' hora' : ' horas');
    var days = Math.floor(hours / 24);
    return days + (days === 1 ? ' dia' : ' dias');
  }

  function lastLoadedLabel() {
    if (state.fromCache && state.cacheSavedAt) {
      return 'Estado guardado de ' + formatCacheWhen(state.cacheSavedAt);
    }
    if (!state.lastFetchedAt) return 'Conectado — ainda sem dados';
    var age = Math.floor((Date.now() - state.lastFetchedAt) / 1000);
    if (age < 10) return 'Atualizado agora';
    if (age < 60) return 'Atualizado há ' + age + ' s';
    return 'Atualizado há ' + Math.floor(age / 60) + ' min';
  }

  function suffixForError(e) {
    if (!e) return '';
    if (e.kind === 'unauthorized') return ' Token inválido — reconecte.';
    if (e.kind === 'network') return ' Sem resposta do servidor.';
    if (e.kind === 'timeout') return ' Servidor demorou para responder.';
    return '';
  }

  function errorBlockFor(e) {
    if (!e) return stateBlock('', 'Sem dados');
    if (e.kind === 'unauthorized') {
      return stateBlock('warn', 'Token inválido',
        'O servidor recusou o token. Reconecte com o token do runner.', [
          { label: 'Reconectar', variant: 'primary', onClick: openSettingsSheet }
        ]);
    }
    if (e.kind === 'notfound') {
      return stateBlock('error', 'Rota não encontrada',
        'O servidor respondeu 404. Verifique se o runner é uma versão que expõe esta rota.', [
          { label: 'Tentar de novo', variant: 'neutral', onClick: refreshNow },
          { label: 'Ajustes', variant: 'ghost', onClick: openSettingsSheet }
        ]);
    }
    if (e.kind === 'network') {
      return stateBlock('error', 'Sem conexão',
        'Não foi possível falar com o servidor. Verifique a rede e o endereço em Ajustes.', [
          { label: 'Tentar de novo', variant: 'primary', onClick: refreshNow },
          { label: 'Ajustes', variant: 'ghost', onClick: openSettingsSheet }
        ]);
    }
    if (e.kind === 'timeout') {
      return stateBlock('error', 'Servidor demorou demais',
        'A resposta não chegou a tempo. Tente de novo em instantes.', [
          { label: 'Tentar de novo', variant: 'primary', onClick: refreshNow }
        ]);
    }
    return stateBlock('error', 'Erro ao carregar',
      e.message || 'O servidor respondeu com erro. Tente de novo.', [
        { label: 'Tentar de novo', variant: 'primary', onClick: refreshNow }
      ]);
  }

  // --------------------------------------------------------------------
  // TELA: Aprovações
  // --------------------------------------------------------------------
  function renderApprovals(screen) {
    var runs = state.lastRuns;
    var blocked = state.lastError;

    screen.appendChild(headerRow('Aprovações', {
      settings: true,
      sub: runs ? (approvalCount(runs) + (approvalCount(runs) === 1 ? ' aguardando liberação' : ' aguardando liberação')) : 'Carregando…'
    }));

    var body = el('div', { class: 'screen__body' });
    if (runs === null) {
      body.appendChild(blocked ? errorBlockFor(blocked) : skeletonList(2));
    } else {
      var pending = runs.filter(function (r) { return r.status === 'AwaitingApproval'; });
      if (pending.length === 0) {
        body.appendChild(stateBlock('', 'Nenhuma aprovação pendente',
          'Quando um fluxo parar num portão, ele aparece aqui.'));
      } else {
        body.appendChild(el('p', { class: 't-small secondary', text: 'Aprovar um portão libera a execução para o próximo passo. A confirmação é feita em folha, com o resumo do que vai rodar.' }));
        pending.forEach(function (r) { body.appendChild(approvalCard(r)); });
      }
      if (blocked && state.lastRuns) {
        body.insertBefore(cacheBanner(blocked), body.firstChild);
      }
    }
    screen.appendChild(body);
  }

  function approvalCard(r) {
    var card = el('div', { class: 'card approval-card' });
    var head = el('div', { class: 'run-card__head' });
    head.appendChild(pillRun('AwaitingApproval'));
    head.appendChild(el('span', { style: 'flex:1 1 auto' }));
    if (r.number) head.appendChild(el('span', { class: 't-mono muted', text: '#' + r.number }));
    card.appendChild(head);

    var name = r.workflow_name || 'Fluxo';
    card.appendChild(el('h2', { class: 'run-card__title', text: name }));

    var bits = [];
    if (r.title && r.title !== name) bits.push(r.title);
    if (r.branch) bits.push(r.branch);
    if (r.repo_name) bits.push(r.repo_name);
    if (r.actor) bits.push('por ' + r.actor);
    card.appendChild(el('p', { class: 't-small', text: bits.join(' · ') || 'Aguardando liberação manual.' }));

    // Passo retido, quando a Run traz steps[] com WaitingApproval
    var held = (r.steps || []).filter(function (s) { return s.status === 'WaitingApproval'; })[0];
    if (held) {
      card.appendChild(el('p', { class: 't-small', text: 'Passo retido: ' + held.name }));
      if (held.log) {
        card.appendChild(el('pre', {
          class: 't-mono',
          style: 'background:var(--surface-code);padding:10px 12px;border-radius:8px;overflow:auto;max-height:140px;color:#d4d4d4;margin:0;white-space:pre',
          text: held.log.slice(-800)
        }));
      }
    }
    if (r.started_ts) {
      card.appendChild(el('p', { class: 't-small muted', text: 'Iniciada em ' + fmtDate(r.started_ts) + ' · ' + fmtAge(r.started_ts) }));
    }

    var actions = el('div', { class: 'approval-card__actions' });
    var approve = el('button', { class: 'btn btn--primary', type: 'button' });
    approve.appendChild(icon('check'));
    approve.appendChild(el('span', { class: 'btn__label', text: 'Aprovar' }));
    approve.addEventListener('click', function () { confirmApproval(r, true, approve); });
    var reject = el('button', { class: 'btn btn--ghost', type: 'button' });
    reject.appendChild(icon('x'));
    reject.appendChild(el('span', { class: 'btn__label', text: 'Recusar' }));
    reject.addEventListener('click', function () { confirmApproval(r, false, reject); });
    var busy = !!state.busyActions['approve:' + r.id] || !!state.busyActions['reject:' + r.id];
    if (busy) {
      approve.disabled = true;
      reject.disabled = true;
    }
    actions.appendChild(approve);
    actions.appendChild(reject);
    card.appendChild(actions);
    return card;
  }

  // Confirmação em sheet (§8.4). Aprovar e recusar são decisões do portão,
  // enviadas com `{"decision": "...", "note": "..."}` para a rota do runner.
  // A nota é opcional e o campo não recebe foco (teclado não pula na cara).
  function confirmApproval(run, isApprove, originBtn) {
    var name = run.workflow_name || run.title || 'Fluxo';
    var step = (run.steps || []).filter(function (s) { return s.status === 'WaitingApproval'; })[0];
    var sheetBody = el('div', {});

    var head = el('div', { class: 'sheet__head' });
    head.appendChild(el('h2', {
      class: 'sheet__title',
      text: isApprove ? 'Aprovar e continuar' : 'Recusar e cancelar'
    }));
    sheetBody.appendChild(head);

    var content = el('div', { class: 'sheet__body' });
    var card = el('div', { class: 'card' });
    card.appendChild(el('p', { class: 't-h3', text: name }));
    var metaBits = [];
    if (run.number) metaBits.push('#' + run.number);
    if (run.branch) metaBits.push(run.branch);
    if (run.repo_name) metaBits.push(run.repo_name);
    if (run.actor) metaBits.push('por ' + run.actor);
    card.appendChild(el('p', { class: 't-small', text: metaBits.join(' · ') || 'Execução sem metadados adicionais.' }));
    if (step) card.appendChild(el('p', { class: 't-small', text: 'Passo retido: ' + step.name }));
    content.appendChild(card);

    // Texto diferente por decisão: recusar CANCELA o run (o runner grava
    // `cancel_requested`), e isso precisa estar claro antes do toque.
    var consequence = el('div', {
      class: 'card ' + (isApprove ? '' : 'card--warn')
    });
    if (isApprove) {
      consequence.appendChild(el('p', {
        class: 't-small secondary',
        text: 'Ao aprovar, o Workploy segue para o próximo passo. A aprovação vale só para esta execução.'
      }));
      if (run.repo_name || run.branch || run.workflow_id) {
        consequence.appendChild(el('p', {
          class: 't-small secondary',
          text: 'Destino: ' + (run.repo_name || 'runner') + (run.branch ? ' · ' + run.branch : '') +
            (run.workflow_id ? ' · ' + run.workflow_id : '')
        }));
      }
    } else {
      var warnRow = el('div', { class: 'warning-row' });
      warnRow.appendChild(icon('warn', 'warning-row__ico'));
      warnRow.appendChild(el('p', {
        class: 't-small',
        text: 'Recusar cancela esta execução. O runner interrompe o fluxo no portão e marca a execução como cancelada; os passos já executados não são desfeitos.'
      }));
      consequence.appendChild(warnRow);
    }
    content.appendChild(consequence);

    var noteField = el('div', { class: 'field' });
    var noteId = 'gate-note-' + (isApprove ? 'approve' : 'reject');
    noteField.appendChild(el('label', { class: 'field__label', for: noteId, text: 'Nota (opcional)' }));
    var noteInput = el('textarea', {
      class: 'field__textarea',
      id: noteId,
      rows: '2',
      maxlength: '500',
      placeholder: isApprove ? 'ex.: liberar só o rodapé' : 'ex.: motivo da recusa',
      'aria-label': 'Nota da decisão (opcional)',
      autocomplete: 'off'
    });
    // Sem `data-autofocus` de propósito: o teclado só aparece se o maestro
    // tocar no campo. O foco do sheet fica no botão de confirmar.
    noteInput.value = '';
    noteField.appendChild(noteInput);
    noteField.appendChild(el('p', {
      class: 'field__hint',
      text: 'A nota vai junto com a decisão e fica registrada na execução.'
    }));
    content.appendChild(noteField);

    var errBox = el('p', { class: 't-small sheet-error', role: 'alert', hidden: 'hidden' });
    content.appendChild(errBox);
    sheetBody.appendChild(content);

    var foot = el('div', { class: 'sheet__foot' });
    var confirmBtn = el('button', {
      class: 'btn btn--primary btn--block',
      type: 'button',
      'data-autofocus': 'true'
    });
    confirmBtn.appendChild(el('span', {
      class: 'btn__label',
      text: isApprove ? 'Aprovar e continuar' : 'Recusar e cancelar'
    }));
    var cancelBtn = el('button', { class: 'btn btn--ghost btn--block', type: 'button', text: 'Voltar' });

    function showSheetError(msg) {
      errBox.textContent = msg;
      errBox.hidden = false;
    }
    function clearSheetError() {
      errBox.textContent = '';
      errBox.hidden = true;
    }

    cancelBtn.addEventListener('click', closeSheet);
    foot.appendChild(confirmBtn);
    foot.appendChild(cancelBtn);
    sheetBody.appendChild(foot);

    openSheet(sheetBody, { label: isApprove ? 'Confirmar aprovação' : 'Confirmar recusa' });

    confirmBtn.addEventListener('click', function () {
      clearSheetError();
      submitGateDecision(run, isApprove, noteInput.value, confirmBtn, cancelBtn, showSheetError);
    });
  }

  // Rótulo das mensagens de erro do portão — humano, sem código HTTP.
  function gateErrorText(err, isApprove) {
    var verb = isApprove ? 'aprovar' : 'recusar';
    if (!err) return 'Não foi possível ' + verb + ' agora.';
    if (err.status === 409) return 'Este portão já foi decidido.';
    if (err.status === 404) return 'Essa execução não existe mais.';
    if (err.status === 400) return 'A decisão não foi aceita.';
    if (err.status === 403) return 'Este token não pode ' + verb + ' este portão.';
    if (err.kind === 'network') return 'Sem resposta do servidor. Tente de novo.';
    if (err.kind === 'timeout') return 'O servidor demorou demais. Tente de novo.';
    return 'Não foi possível ' + verb + ' agora. Tente de novo.';
  }

  function submitGateDecision(run, isApprove, note, confirmBtn, cancelBtn, showError) {
    var key = (isApprove ? 'approve:' : 'reject:') + run.id;
    state.busyActions[key] = true;
    var original = confirmBtn.querySelector('.btn__label').textContent;
    confirmBtn.disabled = true;
    cancelBtn.disabled = true;
    confirmBtn.querySelector('.btn__label').textContent = 'Enviando…';

    function unlock() {
      delete state.busyActions[key];
      confirmBtn.disabled = false;
      cancelBtn.disabled = false;
      confirmBtn.querySelector('.btn__label').textContent = original;
    }

    apiApprove(run.id, isApprove ? 'approve' : 'reject', note)
      .then(function () {
        closeSheet();
        toast(isApprove ? 'Aprovação enviada.' : 'Recusa enviada. A execução foi cancelada.', 'ok');
        if (state.view === 'detail') refreshDetail();
        refreshNow();
      })
      .catch(function (err) {
        if (err && err.kind === 'unauthorized') {
          // Token inválido: fecha o sheet e devolve para a tela de conexão.
          unlock();
          closeSheet();
          state.lastRuns = null;
          state.fromCache = false;
          state.cacheSavedAt = 0;
          state.lastError = err;
          state.view = 'runs';
          state.detail = null;
          state.log = null;
          render();
          openSettingsSheet();
          toast('Token inválido. Reconecte para continuar.', 'error');
          return;
        }
        unlock();
        if (err && (err.status === 409 || err.status === 404)) {
          // Decisão já tomada / execução sumiu: a tela precisa refletir o
          // servidor, não insistir.
          closeSheet();
          toast(gateErrorText(err, isApprove), 'error');
          refreshNow();
          if (state.view === 'detail') refreshDetail();
          return;
        }
        if (err && err.kind === 'invalid') {
          showError(gateErrorText(err, isApprove));
          return;
        }
        showError(gateErrorText(err, isApprove));
      });
  }

  function errorText(err, prefix) {
    if (!err) return prefix;
    if (err.kind === 'unauthorized') return prefix + ' Token inválido — reconecte em Ajustes.';
    if (err.kind === 'forbidden') return prefix + ' O servidor não permite essa ação para este token.';
    if (err.kind === 'notfound') return prefix + ' Execução não encontrada no servidor.';
    if (err.kind === 'network') return prefix + ' Sem resposta do servidor.';
    if (err.kind === 'timeout') return prefix + ' O servidor demorou demais.';
    return prefix + ' ' + (err.message || '');
  }

  // --------------------------------------------------------------------
  // TELA: Projetos (derivada dos runs — agrupamento por workflow_id/name)
  // --------------------------------------------------------------------
  function renderProjects(screen) {
    var runs = state.lastRuns;
    var blocked = state.lastError;

    screen.appendChild(headerRow('Projetos', { settings: true, sub: 'Agrupado por receita' }));

    var body = el('div', { class: 'screen__body' });
    if (runs === null) {
      body.appendChild(blocked ? errorBlockFor(blocked) : skeletonList(3));
      screen.appendChild(body);
      return;
    }

    var groups = {};
    var order = [];
    runs.forEach(function (r) {
      var key = r.workflow_id || r.workflow_name || 'sem-id';
      if (!groups[key]) { groups[key] = { key: key, name: r.workflow_name || r.workflow_id || 'Fluxo', runs: [] }; order.push(key); }
      groups[key].runs.push(r);
    });

    if (order.length === 0) {
      body.appendChild(stateBlock('', 'Nenhum projeto ainda',
        'Quando um fluxo rodar, o projeto aparece aqui.'));
      screen.appendChild(body);
      return;
    }

    var totalRuns = 0, totalSuccess = 0, hasFailure = false, anyActive = false;
    runs.forEach(function (r) {
      totalRuns++;
      if (r.status === 'Success') totalSuccess++;
      if (r.status === 'Failed') hasFailure = true;
      if (r.status === 'Running' || r.status === 'Queued' || r.status === 'AwaitingApproval') anyActive = true;
    });

    var kpis = el('div', { class: 'kpis' });
    kpis.appendChild(kpi('Execuções', String(totalRuns)));
    kpis.appendChild(kpi('Sucesso', totalRuns ? fmtPct(totalSuccess / totalRuns * 100) + '%' : '—'));
    kpis.appendChild(kpi('Projetos', String(order.length)));
    kpis.appendChild(kpi('Ativas', String(activeCount(runs))));
    body.appendChild(kpis);

    var list = el('ul', { class: 'rows stagger' });
    order.forEach(function (key) {
      var g = groups[key];
      var recent = g.runs.slice().sort(function (a, b) { return (b.started_ts || 0) - (a.started_ts || 0); })[0];
      var failures = g.runs.filter(function (r) { return r.status === 'Failed'; }).length;
      var item = el('li');
      var card = el('button', { class: 'row-btn', type: 'button', 'aria-label': 'Ver execuções do projeto ' + g.name });
      var dot = el('span', { class: 'dot-health ' + (failures ? 'dot-health--bad' : (recent && recent.status === 'Running' ? 'dot-health--warn' : (recent && recent.status === 'Success' ? 'dot-health--ok' : 'dot-health--idle'))) });
      dot.setAttribute('aria-hidden', 'true');
      card.appendChild(dot);
      var main = el('div', { class: 'row-btn__main' });
      main.appendChild(el('span', { class: 'row-btn__title', text: g.name }));
      main.appendChild(el('span', {
        class: 'row-btn__sub',
        text: g.runs.length + (g.runs.length === 1 ? ' execução' : ' execuções') +
          (recent ? ' · último ' + (RUN_LABELS[recent.status] || recent.status) : '')
      }));
      card.appendChild(main);
      card.appendChild(icon('chev', 'row-btn__chev'));
      card.addEventListener('click', function () {
        state.filter = 'all';
        state.view = 'runs';
        state.lastRuns = runs; // apenas recorta o filtro
        state.projectFilter = g.key;
        render();
        state.projectFilter = null;
      });
      item.appendChild(card);
      list.appendChild(item);
    });
    body.appendChild(list);
    screen.appendChild(body);
  }

  function kpi(label, value) {
    var c = el('div', { class: 'card' });
    c.appendChild(el('p', { class: 't-cap', text: label }));
    c.appendChild(el('p', { class: 'kpi__value', text: value }));
    return c;
  }

  // --------------------------------------------------------------------
  // TELA: Conexões (leitura honesta do que o runner devolve para /status)
  // --------------------------------------------------------------------
  function renderConnections(screen) {
    screen.appendChild(headerRow('Conexões', { settings: true }));

    var body = el('div', { class: 'screen__body' });
    var card = el('div', { class: 'card' });
    card.appendChild(el('p', { class: 't-cap', text: 'Servidor' }));
    card.appendChild(el('p', { class: 't-h3', text: state.server || state.base || 'Não configurado' }));
    card.appendChild(el('p', { class: 't-small', text: state.lastFetchedAt ? 'Último contato: ' + fmtDate(Math.floor(state.lastFetchedAt / 1000)) : 'Ainda sem contato bem-sucedido' }));
    if (state.lastError) {
      card.appendChild(el('p', { class: 't-small', text: 'Último erro: ' + state.lastError.message }));
    }
    body.appendChild(card);

    body.appendChild(stateBlock('',
      'Saúde da VPS não aparece nesta versão',
      'O runner não expõe KPIs de SSH/GitHub nem estado do par para esta página. Abra a versão desktop para diagnóstico completo.'));

    var actions = el('div', { class: 'state__actions' });
    var test = el('button', { class: 'btn btn--primary', type: 'button', text: 'Testar conexão' });
    test.addEventListener('click', function () {
      test.disabled = true;
      test.textContent = 'Testando…';
      fetchJSON('/status').then(function (data) {
        test.disabled = false;
        test.textContent = 'Testar conexão';
        if (data && data.ok) toast('Servidor respondeu OK.', 'ok');
        else toast('Resposta inesperada do servidor.', 'error');
        refreshNow();
      }).catch(function (err) {
        test.disabled = false;
        test.textContent = 'Testar conexão';
        toast(errorText(err, 'Conexão falhou.'), 'error');
      });
    });
    actions.appendChild(test);
    body.appendChild(actions);

    var setup = el('div', { class: 'state__actions' });
    var btnSetup = el('button', { class: 'btn btn--neutral', type: 'button', text: 'Editar conexão' });
    btnSetup.addEventListener('click', openSettingsSheet);
    setup.appendChild(btnSetup);
    body.appendChild(setup);

    screen.appendChild(body);
  }

  // --------------------------------------------------------------------
  // TELA: Detalhe do run
  // --------------------------------------------------------------------
  function renderDetail(screen) {
    var d = state.detail;
    var run = d && d.run;
    var label = run ? (run.workflow_name || run.title || 'Execução') : 'Execução';

    screen.appendChild(headerRow(label, {
      back: function () { state.view = 'runs'; state.detail = null; state.log = null; stopPoll(); render(); },
      action: { label: 'Atualizar execução', icon: 'refresh', onClick: function () { refreshDetail(); } }
    }));

    if (!d) {
      screen.appendChild(el('div', { class: 'screen__body' }, skeletonList(2)));
      return;
    }
    if (d.error && !run) {
      screen.appendChild(el('div', { class: 'screen__body' }, errorBlockFor(d.error)));
      return;
    }
    if (!run) {
      screen.appendChild(el('div', { class: 'screen__body' }, skeletonList(2)));
      return;
    }

    var body = el('div', { class: 'screen__body' });

    var head = el('div', { class: 'detail-head' });
    var headRow = el('div', { class: 'run-card__head' });
    headRow.appendChild(pillRun(run.status));
    if (run.number) headRow.appendChild(el('span', { class: 't-mono muted', text: '#' + run.number }));
    headRow.appendChild(el('span', { style: 'flex:1 1 auto' }));
    if (run.started_ts) headRow.appendChild(el('span', { class: 't-small muted', text: fmtAge(run.started_ts) }));
    head.appendChild(headRow);
    var title = el('h2', { class: 'detail-head__title', text: run.workflow_name || run.title || 'Execução' });
    head.appendChild(title);
    var meta = el('div', { class: 'detail-head__meta' });
    if (run.started_ts) meta.appendChild(el('span', { text: fmtDate(run.started_ts) }));
    else if (run.started_at) meta.appendChild(el('span', { text: run.started_at }));
    if (run.secs) meta.appendChild(el('span', { text: fmtDur(run.secs) }));
    if (run.branch) meta.appendChild(el('span', { text: run.branch }));
    if (run.repo_name) meta.appendChild(el('span', { text: run.repo_name }));
    if (run.actor) meta.appendChild(el('span', { text: 'por ' + run.actor }));
    head.appendChild(meta);
    var prog = el('div', { class: 'progress', 'aria-hidden': 'true' });
    var fill = el('div', { class: 'progress__fill ' + progressTone(run.status) });
    fill.style.width = Math.round(runProgress(run) * 100) + '%';
    prog.appendChild(fill);
    head.appendChild(prog);
    body.appendChild(head);

    if (run.status === 'AwaitingApproval') {
      var gate = el('div', { class: 'card approval-card' });
      gate.appendChild(el('p', { class: 't-h3', text: 'Aguardando sua aprovação' }));
      var held = (run.steps || []).filter(function (s) { return s.status === 'WaitingApproval'; })[0];
      gate.appendChild(el('p', { class: 't-small', text: held ? 'Passo retido: ' + held.name : 'A execução está parada num portão.' }));
      var acts = el('div', { class: 'approval-card__actions' });
      var ap = el('button', { class: 'btn btn--primary', type: 'button' });
      ap.appendChild(icon('check'));
      ap.appendChild(el('span', { class: 'btn__label', text: 'Aprovar' }));
      ap.addEventListener('click', function () { confirmApproval(run, true, ap); });
      var rj = el('button', { class: 'btn btn--ghost', type: 'button' });
      rj.appendChild(icon('x'));
      rj.appendChild(el('span', { class: 'btn__label', text: 'Recusar' }));
      rj.addEventListener('click', function () { confirmApproval(run, false, rj); });
      acts.appendChild(ap);
      acts.appendChild(rj);
      gate.appendChild(acts);
      body.appendChild(gate);
    }

    if (run.cancel_requested && run.status === 'Running') {
      body.appendChild(stateBlock('warn', 'Cancelamento solicitado',
        'O runner recebeu o pedido de cancelamento e está encerrando a execução.'));
    }

    // Segmented Resumo | Logs (§8.3)
    var modes = [{ id: 'summary', label: 'Resumo' }, { id: 'logs', label: 'Logs' }];
    var seg = el('div', { class: 'segmented', role: 'tablist', 'aria-label': 'Modo do detalhe' });
    seg.dataset.role = 'mode-tabs';
    // Normaliza uma única vez: se o modo é inválido (ex.: troca de run),
    // cai no Resumo. Depois disso, o clique do chip é a única fonte.
    if (state.detailMode !== 'logs') state.detailMode = 'summary';
    var mode = state.detailMode;
    modes.forEach(function (m) {
      var c = el('button', {
        class: 'chip', type: 'button', role: 'tab',
        'aria-selected': m.id === mode ? 'true' : 'false'
      });
      c.textContent = m.label;
      c.addEventListener('click', function () {
        state.detailMode = m.id;
        if (m.id === 'logs') ensureLogOpen(run);
        else { closeLogPoll(); }
        rerender();
      });
      seg.appendChild(c);
    });
    body.appendChild(seg);

    if (mode === 'summary') {
      body.appendChild(stepsSection(run));
      if (run.summary) {
        var sc = el('div', { class: 'card' });
        sc.appendChild(el('p', { class: 't-cap', text: 'Resumo' }));
        sc.appendChild(el('p', { class: 't-small', text: run.summary }));
        body.appendChild(sc);
      }
      if (run.commit) {
        var cc = el('div', { class: 'card' });
        cc.appendChild(el('p', { class: 't-cap', text: 'Commit' }));
        var sha = el('p', { class: 't-mono', text: run.commit.slice(0, 12), style: 'margin:0' });
        attachLongPressCopy(sha, run.commit, 'Caminho do commit copiado');
        cc.appendChild(sha);
        body.appendChild(cc);
      }
      if (run.finished_at) {
        body.appendChild(el('p', { class: 't-small muted', text: 'Concluída em ' + run.finished_at }));
      }
      var actions = el('div', { class: 'state__actions' });
      if (run.status === 'Running' || run.status === 'Queued') {
        var cancel = el('button', { class: 'btn btn--danger', type: 'button', text: 'Cancelar execução' });
        cancel.addEventListener('click', function () { confirmCancel(run); });
        actions.appendChild(cancel);
      }
      if (run.status === 'Failed' || run.status === 'Cancelled' || run.status === 'Success') {
        var rerun = el('button', { class: 'btn btn--neutral', type: 'button', text: 'Reexecutar fluxo' });
        rerun.addEventListener('click', function () {
          rerun.disabled = true;
          apiTrigger({
            workflow_id: run.workflow_id,
            trigger_label: 'reexecução pelo celular',
            repo: run.repo_name || '',
            branch: run.branch || 'main',
            actor: 'maestro (mobile)'
          }).then(function (res) {
            toast('Fluxo disparado.', 'ok');
            if (res && res.run_id) openDetail(res.run_id);
            else refreshNow();
          }).catch(function (err) {
            rerun.disabled = false;
            toast(errorText(err, 'Não foi possível reexecutar.'), 'error');
          });
        });
        actions.appendChild(rerun);
      }
      if (actions.childNodes.length) body.appendChild(actions);
    } else {
      body.appendChild(logSection(run));
    }

    if (d.error && run) {
      body.insertBefore(el('div', { class: 'card card--info' },
        [el('p', { class: 't-small', text: 'Sem atualização agora. Mostrando o último estado.' + suffixForError(d.error) })]), body.firstChild);
    }

    screen.appendChild(body);
  }

  function stepsSection(run) {
    var wrap = el('div', { class: 'card' });
    var top = el('div', { class: 'run-card__head' });
    top.appendChild(el('p', { class: 't-cap', text: 'Passos' }));
    top.appendChild(el('span', { style: 'flex:1 1 auto' }));
    top.appendChild(el('p', { class: 't-small muted', text: (run.steps || []).length + '' }));
    wrap.appendChild(top);

    var list = el('ul', { class: 'step-list' });
    (run.steps || []).forEach(function (st, idx) {
      var li = el('li');
      var btn = el('button', { class: 'step' + (state.logStep === idx && state.view === 'detail' && state.detailMode === 'logs' ? ' is-active' : ''), type: 'button' });
      btn.appendChild(el('span', { class: 'step__index', text: String(idx + 1) }));
      btn.appendChild(el('span', { class: 'step__name', text: st.name || 'Passo ' + (idx + 1) }));
      if (st.secs) btn.appendChild(el('span', { class: 'step__secs', text: fmtDur(st.secs) }));
      btn.appendChild(pillStep(st.status));
      btn.setAttribute('aria-label', 'Abrir log do passo ' + (st.name || idx + 1) + ', status ' + (STEP_LABELS[st.status] || st.status));
      btn.addEventListener('click', function () {
        state.detailMode = 'logs';
        state.logStep = idx;
        ensureLogOpen(run);
        rerender();
      });
      li.appendChild(btn);
      if (st.substeps && st.substeps.length) {
        var ul = el('ul', { class: 'step__children' });
        st.substeps.forEach(function (ss, si) {
          var sli = el('li');
          var sb = el('button', {
            class: 'step__child' + (state.logStep === idx && state.logSubstep === si ? ' is-active' : ''),
            type: 'button'
          });
          sb.appendChild(el('span', { class: 'step__name', text: (ss.name || ('Subetapa ' + (si + 1))) }));
          sb.appendChild(pillStep(ss.status));
          sb.addEventListener('click', function () {
            state.detailMode = 'logs';
            state.logStep = idx;
            state.logSubstep = si;
            ensureLogOpen(run);
            rerender();
          });
          sli.appendChild(sb);
          ul.appendChild(sli);
        });
        li.appendChild(ul);
      }
      list.appendChild(li);
    });
    wrap.appendChild(list);
    return wrap;
  }

  function confirmCancel(run) {
    var body = el('div', {});
    var head = el('div', { class: 'sheet__head' });
    head.appendChild(el('h2', { class: 'sheet__title', text: 'Cancelar execução' }));
    body.appendChild(head);
    var content = el('div', { class: 'sheet__body' });
    content.appendChild(el('p', { class: 't-small', text: run.workflow_name || run.title || 'Execução' }));
    content.appendChild(el('p', { class: 't-small secondary', text: 'Os passos já executados não são desfeitos.' }));
    body.appendChild(content);
    var foot = el('div', { class: 'sheet__foot' });
    var ok = el('button', { class: 'btn btn--danger-solid btn--block', type: 'button', text: 'Cancelar execução' });
    var no = el('button', { class: 'btn btn--ghost btn--block', type: 'button', text: 'Voltar' });
    no.addEventListener('click', closeSheet);
    foot.appendChild(ok);
    foot.appendChild(no);
    body.appendChild(foot);
    openSheet(body, { label: 'Cancelar execução' });
    ok.addEventListener('click', function () {
      ok.disabled = true;
      ok.textContent = 'Enviando…';
      apiCancel(run.id).then(function () {
        closeSheet();
        toast('Cancelamento solicitado.', 'ok');
        refreshDetail();
      }).catch(function (err) {
        ok.disabled = false;
        ok.textContent = 'Cancelar execução';
        toast(errorText(err, 'Não foi possível cancelar.'), 'error');
      });
    });
  }

  // --------------------------------------------------------------------
  // Log — poll incremental com offset, auto-follow opcional, "ir para o fim"
  // --------------------------------------------------------------------
  function stepIndexOf(run, stateObj) {
    var idx = state.logStep;
    if (idx == null || idx < 0) {
      // Começa no primeiro passo ativo ou no último que tem log.
      var active = (run.steps || []).findIndex(function (s) { return s.status === 'Running' || s.status === 'WaitingApproval'; });
      if (active >= 0) return active;
      var lastWithLog = -1;
      (run.steps || []).forEach(function (s, i) { if (s.log) lastWithLog = i; });
      return lastWithLog >= 0 ? lastWithLog : 0;
    }
    return idx;
  }

  // offset e size são caracteres, não bytes: ver contrato do runner.
  // O runner calcula `len(full[offset:offset+tail])` sobre a string Python
  // concatenada dos passos. "Caractere" ali é CODE POINT, e `String.length`
  // do JS conta unidades UTF-16 (emoji ocupa 2) — por isso o cursor usa
  // `Array.from(chunk).length`, nunca `.length` cru nem TextEncoder.
  // `tail=200000` é o teto do próprio runner.
  var LOG_TAIL = 200000;
  // Converte o índice de code point em índice UTF-16 na string acumulada.
  function utf16IndexOf(text, codePoints) {
    if (codePoints <= 0) return 0;
    if (codePoints >= text.length) return text.length;
    // Percorre code points sem alocar array inteiro a cada poll.
    var idx = 0;
    var cp = 0;
    while (idx < text.length && cp < codePoints) {
      var code = text.charCodeAt(idx);
      var step = code >= 0xd800 && code <= 0xdbff ? 2 : 1;
      idx += step;
      cp += 1;
    }
    return idx;
  }
  function countCodePoints(s) {
    if (!s) return 0;
    // Soma 1 por code point; pares substitutos contam como 1.
    var cp = 0;
    for (var i = 0; i < s.length; i++) {
      var code = s.charCodeAt(i);
      if (code >= 0xd800 && code <= 0xdbff && i + 1 < s.length) i++;
      cp += 1;
    }
    return cp;
  }
  function buildLogPath(runId, idx, offset) {
    var p = '/run/' + encodeURIComponent(runId) + '/log?offset=' + Math.max(0, offset || 0) +
      '&tail=' + LOG_TAIL;
    if (idx != null && idx >= 0) p += '&step=' + idx;
    return p;
  }

  function ensureLogOpen(run) {
    if (!run) return;
    var idx = stepIndexOf(run, state);
    if (state.log && state.log.id === run.id && state.log.step === idx && state.log.substep === state.logSubstep) {
      // Já há log para este passo: religa o timer e completa o que falta.
      // Não vira um render() em loop — a primeira chamada fica aqui e o
      // timer cuida do resto.
      startLogPoll();
      return;
    }
    state.log = {
      id: run.id, step: idx, substep: state.logSubstep || null,
      // offset em caracteres (string JS), não bytes.
      // `offset` e `size` em CODE POINTS (contrato do runner), não UTF-16.
      text: '', offset: 0, size: 0, truncated: false, history: false,
      error: null, loadingFrom: 0, visibleFrom: 0
    };
    startLogPoll();
    pollLogOnce();
  }

  //: single-flight do log: dois gatilhos (timer + resume/visibility) não
  // podem derramar o MESMO offset duas vezes em L.text.
  var logPollInFlight = false;
  function pollLogOnce() {
    var L = state.log;
    if (!L) return Promise.resolve();
    if (!state.visible) return Promise.resolve();
    if (logPollInFlight) return Promise.resolve();
    // `offset` vem em caracteres: é o cursor de L.text.
    var requestedFrom = L.offset;
    if (L.loadingFrom != null) L.loadingFrom = requestedFrom;
    logPollInFlight = true;
    return fetchJSON(buildLogPath(L.id, L.step, requestedFrom))
      .then(function (data) {
        logPollInFlight = false;
        if (!state.log || state.log !== L) return;
        if (!data || typeof data !== 'object') {
          L.error = httpError('http', 0, 'Resposta inesperada do servidor.');
          return;
        }
        var chunk = typeof data.log === 'string' ? data.log : '';
        var size = typeof data.size === 'number' ? data.size : (requestedFrom + countCodePoints(chunk));
        // Resposta de um offset já coberto (retry/duplicata): ignora o
        // conteúdo em vez de colar duas vezes o mesmo trecho na tela.
        var serverOffset = typeof data.offset === 'number' ? data.offset : requestedFrom;
        if (L.offset > serverOffset + 0) {
          L.size = size;
          L.error = null;
          updateLogPane();
          return;
        }
        // O servidor pode devolver offset além do fim (log reciclado): nesse
        // caso payload.log vem vazio e truncated=true; rebobinamos.
        if (typeof data.offset === 'number' && data.offset !== requestedFrom) {
          if (data.offset < requestedFrom) {
            // O runner voltou atrás → descarta o buffer e recomeça.
            L.text = '';
            L.offset = data.offset;
            L.history = false;
            L.loadingFrom = data.offset;
          }
          // Se data.offset > requestedFrom, mantém o cursor — o próximo poll
          // continua daqui em vez de pular caracteres.
        }
        // Poll sem novidade: chunk vazio e size > offset significa "nada
        // novo ainda". Não apaga a tela — o conteúdo já renderizado fica.
        if (chunk) {
          L.text += chunk;
          L.offset += countCodePoints(chunk);  // code points, não bytes nem UTF-16
        } else if (size > L.offset) {
          // sem chunk mas o total é maior: o runner limitou por tail; o
          // próximo poll pega o trecho seguinte.
        }
        L.size = size;
        L.truncated = !!data.truncated;
        if (!L.truncated) L.loadingFrom = 0;
        L.error = null;
        updateLogPane();
      })
      .catch(function (err) {
        logPollInFlight = false;
        if (!state.log || state.log !== L) return;
        L.error = err;
        if (err && err.kind === 'notfound') {
          // log ainda inexistente é esperado num run recém-criado.
          L.error = httpError('notfound', 404, 'Log ainda não disponível.');
        }
        updateLogPane();
      });
  }

  function logSection(run) {
    var wrap = el('div', {});
    var idx = stepIndexOf(run, state);
    var steps = run.steps || [];
    var meta = steps[idx];

    if (steps.length > 1) {
      var chips = el('div', { class: 'chips', role: 'tablist', 'aria-label': 'Passo do log' });
      steps.forEach(function (s, i) {
        var c = el('button', { class: 'chip', type: 'button', role: 'tab', 'aria-selected': i === idx ? 'true' : 'false' });
        c.textContent = (i + 1) + '. ' + (s.name || 'Passo');
        c.addEventListener('click', function () {
          state.logStep = i;
          state.logSubstep = null;
          state.log = null;
          rerender();
          ensureLogOpen(state.detail && state.detail.run);
        });
        chips.appendChild(c);
      });
      wrap.appendChild(chips);
    }

    if (state.log && state.log.id !== run.id) state.log = null;
    if (!state.log) ensureLogOpen(run);

    var L = state.log;

    // Aviso discreto de recorte: "carregado a partir de…" (§7: este é o
    // único lugar com rolagem horizontal; o aviso fica no topo do bloco).
    if (L && L.truncated) {
      var banner = el('div', { class: 'log-banner', role: 'status' });
      banner.appendChild(icon('info', 'log-banner__ico'));
      var bannerText = el('p', { class: 'log-banner__text' });
      bannerText.appendChild(document.createTextNode('Carregado a partir do caractere '));
      bannerText.appendChild(el('span', { class: 't-mono', text: String(L.loadingFrom || 0) }));
      bannerText.appendChild(document.createTextNode(' de ' + L.size + '. Trechos anteriores não estão na tela.'));
      banner.appendChild(bannerText);
      var fromTop = el('button', { class: 'chip log-banner__btn', type: 'button' });
      fromTop.appendChild(el('span', { text: 'Recarregar do início' }));
      fromTop.addEventListener('click', reloadLogFromStart);
      banner.appendChild(fromTop);
      wrap.appendChild(banner);
    }

    var box = el('div', { class: 'log-wrap' });
    var logHead = el('div', { class: 'log-head' });
    var title = meta ? ((idx + 1) + '/' + steps.length + ' · ' + (meta.name || 'Passo')) : 'Log';
    logHead.appendChild(el('p', { class: 'log-head__title', text: title }));
    var live = el('span', { class: 'log-live' });
    var dot = el('span', { class: 'log-live__dot' + (state.visible ? '' : ' is-paused') });
    live.appendChild(dot);
    live.appendChild(el('span', { text: state.visible ? 'ao vivo' : 'pausado' }));
    logHead.appendChild(live);
    box.appendChild(logHead);

    var scroll = el('div', { class: 'log-scroll', id: 'log-scroll' });
    var pre = el('pre', {
      class: 'log-pre' + (state.logWrap ? ' is-wrapped' : ''),
      id: 'log-pre',
      'aria-label': 'Log da execução',
      tabindex: '0',
      role: 'log'
    });
    pre.textContent = (L && L.text) || '';
    if (!pre.textContent) {
      var empty = el('span', { class: 'log-empty', text: (L && L.error)
        ? 'Sem log ainda (' + L.error.message + ')'
        : 'Aguardando saída do passo…' });
      pre.appendChild(empty);
    }
    attachLongPressCopy(pre, null, 'Log copiado');
    scroll.appendChild(pre);
    box.appendChild(scroll);
    // O botão "ir para o fim" só aparece quando o auto-scroll está desligado.
    wrap.appendChild(box);

    var follow = el('button', { class: 'log-follow', type: 'button' });
    follow.id = 'log-follow';
    follow.appendChild(icon('arrowDown'));
    follow.appendChild(el('span', { text: 'Ir para o fim' }));
    follow.hidden = state.autoFollow !== false;
    follow.addEventListener('click', function () {
      var sc = document.getElementById('log-scroll');
      if (sc) sc.scrollTop = sc.scrollHeight;
      state.autoFollow = true;
      follow.hidden = true;
      try { localStorage.setItem(LS_PREFS, JSON.stringify({ logWrap: !!state.logWrap })); } catch (_) {}
    });
    wrap.appendChild(follow);

    var toolbar = el('div', { class: 'log-toolbar' });
    var wrapBtn = el('button', { class: 'chip', type: 'button', 'aria-pressed': state.logWrap ? 'true' : 'false' });
    wrapBtn.appendChild(icon('wrap', ''));
    wrapBtn.appendChild(el('span', { text: 'Quebrar linha' }));
    wrapBtn.addEventListener('click', function () {
      state.logWrap = !state.logWrap;
      var p = document.getElementById('log-pre');
      if (p) p.classList.toggle('is-wrapped', !!state.logWrap);
      wrapBtn.setAttribute('aria-pressed', state.logWrap ? 'true' : 'false');
      try { localStorage.setItem(LS_PREFS, JSON.stringify({ logWrap: !!state.logWrap })); } catch (_) {}
    });
    var copyBtn = el('button', { class: 'chip', type: 'button' });
    copyBtn.appendChild(icon('copy', ''));
    copyBtn.appendChild(el('span', { text: 'Copiar log' }));
    copyBtn.addEventListener('click', function () { copyLog(); });
    toolbar.appendChild(wrapBtn);
    toolbar.appendChild(copyBtn);
    if (L && L.truncated) {
      var more = el('button', { class: 'chip', type: 'button' });
      more.appendChild(el('span', { text: 'Carregar anterior' }));
      more.addEventListener('click', reloadLogFromStart);
      toolbar.appendChild(more);
    }
    wrap.appendChild(toolbar);

    // scroll handler: quando o usuário sobe, o auto-follow é desligado e o
    // botão "ir para o fim" aparece.
    setTimeout(function () {
      var sc = document.getElementById('log-scroll');
      if (!sc) return;
      sc.addEventListener('scroll', function () {
        var nearBottom = sc.scrollHeight - sc.scrollTop - sc.clientHeight < 32;
        state.autoFollow = nearBottom;
        var btn = document.getElementById('log-follow');
        if (btn) btn.hidden = nearBottom;
      }, { passive: true });
      if (state.log) {
        if (state.autoFollow === false) {
          var btn = document.getElementById('log-follow');
          if (btn) btn.hidden = false;
        } else {
          sc.scrollTop = sc.scrollHeight;
        }
      }
    }, 0);

    return wrap;
  }

  function updateLogPane() {
    var pre = document.getElementById('log-pre');
    var sc = document.getElementById('log-scroll');
    if (!pre) return;
    var L = state.log;
    if (!L) return;
    // Atualiza o banner de recorte sem remontar a tela.
    var banner = document.querySelector('.log-banner');
    if (banner) {
      var txt = banner.querySelector('.log-banner__text');
      if (txt) {
        txt.textContent = 'Carregado a partir do caractere ' + (L.loadingFrom || 0) + ' de ' + L.size + '. Trechos anteriores não estão na tela.';
      }
    }
    pre.textContent = L.text || '';
    if (!L.text) {
      var empty = document.createElement('span');
      empty.className = 'log-empty';
      empty.textContent = L.error ? 'Sem log ainda (' + L.error.message + ')' : 'Aguardando saída do passo…';
      pre.appendChild(empty);
    }
    if (sc && state.autoFollow !== false) {
      sc.scrollTop = sc.scrollHeight;
      var btn = document.getElementById('log-follow');
      if (btn) btn.hidden = true;
    }
  }

  function copyLog() {
    var L = state.log;
    if (!L || !L.text) { toast('Nada para copiar ainda.'); return; }
    copyText(L.text, 'Log copiado');
  }

  function copyText(text, msg) {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).then(function () { toast(msg || 'Copiado.', 'ok'); }, fallbackCopy);
    } else fallbackCopy();
    function fallbackCopy() {
      var ta = document.createElement('textarea');
      ta.value = text;
      ta.setAttribute('readonly', '');
      ta.style.position = 'fixed';
      ta.style.opacity = '0';
      document.body.appendChild(ta);
      ta.select();
      try { document.execCommand('copy'); toast(msg || 'Copiado.', 'ok'); }
      catch (_) { toast('Não foi possível copiar.', 'error'); }
      document.body.removeChild(ta);
    }
  }

  function reloadLogFromStart() {
    var L = state.log;
    if (!L) return;
    // Recarregar do início: offset volta a 0 em caracteres.
    L.text = '';
    L.offset = 0;
    L.loadingFrom = 0;
    L.truncated = false;
    L.error = null;
    pollLogOnce().then(function () { rerender(); });
  }

  // Toque longo copia (sem hover). Fallback: duplo toque, se o contexto não cooperar.
  function attachLongPressCopy(node, explicitText, msg) {
    if (!node) return;
    var timer = null;
    var LONG = 550;
    function getText() {
      if (explicitText != null) return explicitText;
      var sel = window.getSelection ? String(window.getSelection()) : '';
      return sel && sel.trim() ? sel : (node.textContent || '');
    }
    function start(e) {
      clearTimeout(timer);
      timer = setTimeout(function () {
        var t = getText();
        if (t && t.length) copyText(t, msg || 'Copiado');
      }, LONG);
    }
    function cancel() { clearTimeout(timer); }
    node.addEventListener('pointerdown', start);
    node.addEventListener('pointerup', cancel);
    node.addEventListener('pointercancel', cancel);
    node.addEventListener('pointerleave', cancel);
  }

  // --------------------------------------------------------------------
  // Poll (lista e log), com pausa quando aba fica em segundo plano
  // --------------------------------------------------------------------
  var pollHandle = null;
  var detailPollHandle = null;
  var logPollHandle = null;

  function stopPoll() {
    if (pollHandle) { clearInterval(pollHandle); pollHandle = null; }
    if (detailPollHandle) { clearInterval(detailPollHandle); detailPollHandle = null; }
    if (logPollHandle) { clearInterval(logPollHandle); logPollHandle = null; }
  }
  function closeLogPoll() {
    if (logPollHandle) { clearInterval(logPollHandle); logPollHandle = null; }
    state.log = null;
  }
  function startListPoll() {
    if (pollHandle) return;
    if (!state.visible) return;
    pollHandle = setInterval(function () {
      if (!state.visible) return;
      refreshRunsSilent();
    }, POLL_MS);
  }
  function startDetailPoll() {
    if (detailPollHandle) return;
    if (!state.visible) return;
    detailPollHandle = setInterval(function () {
      if (!state.visible) return;
      refreshDetail(true);
    }, POLL_MS);
  }
  function startLogPoll() {
    if (logPollHandle) return;
    if (!state.visible) return;
    logPollHandle = setInterval(function () {
      if (!state.visible) return;
      pollLogOnce();
    }, POLL_MS);
  }

  // --------------------------------------------------------------------
  // Cache offline do último estado conhecido (gate 8 do §12)
  // --------------------------------------------------------------------
  // Sem rede e com o app reaberto, o último `GET /runs` bem-sucedido continua
  // na tela. O cache é do maestro e do aparelho; é explicitamente marcado na
  // UI como "sem conexão", nunca faz passar por dado vivo.
  // A chave é versionada (`workploy.cache.runs.v1`): um schema novo pode
  // gravar v2 e ignorar a v1 sem migração complicada.
  function readRunsCache() {
    try {
      var raw = localStorage.getItem(LS_RUNS_CACHE);
      if (!raw) return null;
      var data = JSON.parse(raw);
      if (!data || !Array.isArray(data.runs) || !data.savedAt) return null;
      return data;
    } catch (_) { return null; }
  }

  // Persiste somente quando o conjunto mudou de verdade; o poll de 2 s não
  // deve reescrever localStorage a cada ciclo. Remove antes de medir para o
  // tamanho refletir o que ficará gravado. Se passar do teto, guarda os
  // `RUNS_CACHE_TRIM_RUNS` mais recentes em vez de falhar.
  function writeRunsCache(runs) {
    if (!Array.isArray(runs)) return false;
    var savedAt = Date.now();
    var payload = { savedAt: savedAt, runs: runs.slice(0, RUNS_CACHE_MAX_RUNS) };
    var serialized;
    try { serialized = JSON.stringify(payload); } catch (_) { return false; }
    if (serialized.length > RUNS_CACHE_MAX_BYTES) {
      var recent = runs.slice(0, RUNS_CACHE_TRIM_RUNS);
      // Recalcula mantendo os mais recentes; a lista já vem em ordem do
      // runner (id mais novo primeiro).
      var trimmed = { savedAt: savedAt, runs: recent, truncated: true, dropped: runs.length - recent.length };
      try { serialized = JSON.stringify(trimmed); } catch (_) { return false; }
    }
    try {
      localStorage.setItem(LS_RUNS_CACHE, serialized);
      return true;
    } catch (_) { return false; }
  }

  // Diferença de "topo da lista" — suficiente para decidir se o cache
  // precisa ser reescrito. O conteúdo de cada run muda a todo poll (steps),
  // então comparamos `id`, `status` e `finished_at` do topo.
  function runsCacheSignature(runs) {
    if (!Array.isArray(runs) || runs.length === 0) return '';
    var head = runs[0] || {};
    return [
      runs.length,
      head.id || '',
      head.status || '',
      head.finished_at || ''
    ].join('|');
  }
  var lastCacheSignature = null;

  function maybeWriteRunsCache(runs) {
    var sig = runsCacheSignature(runs);
    if (sig === lastCacheSignature) return;
    lastCacheSignature = sig;
    writeRunsCache(runs);
  }

  // --------------------------------------------------------------------
  // Fetch de dados
  // --------------------------------------------------------------------
  // Um refresh que chega DEPOIS de o usuário abrir um run não pode repintar
  // a lista por cima do detalhe: era exatamente o bug de tela voltando para
  // "Execuções" enquanto o log abria. O render só roda se a tela ainda for a
  // mesma e o run não foi fechado no meio.
  var listEpoch = 0;
  function refreshRunsSilent() {
    if (!state.token) return;
    if (state.view !== 'runs' && state.view !== 'projects' && state.view !== 'approvals') return;
    var myEpoch = ++listEpoch;
    return fetchJSON('/runs?limit=100')
      .then(function (data) {
        if (!Array.isArray(data)) throw httpError('http', 0, 'Resposta inesperada do servidor.');
        state.lastRuns = data;
        state.fromCache = false;
        state.lastError = null;
        state.lastFetchedAt = Date.now();
        maybeWriteRunsCache(data);
        if (myEpoch !== listEpoch) return; // resposta velha: só atualiza dados, não pinta
        if (state.view === 'detail') return;
        rerender();
      })
      .catch(function (err) {
        // Erros de poll chegam silenciosamente; o estado de cache honesto
        // aparece na tela somente se o usuário for reconhecer a falha.
        if (myEpoch !== listEpoch) return;
        if (state.view === 'detail') return;
        state.lastError = err;
        if (!isAuthError(err) && state.lastRuns === null) applyCachedRuns(err);
        if (state.lastRuns === null) ensureSessionOrAuth(err);
        rerender();
      });
  }

  function refreshNow() {
    if (!state.token) { openSettingsSheet(); return; }
    var myEpoch = ++listEpoch;
    return fetchJSON('/runs?limit=100')
      .then(function (data) {
        if (!Array.isArray(data)) throw httpError('http', 0, 'Resposta inesperada do servidor.');
        state.lastRuns = data;
        state.fromCache = false;
        state.lastError = null;
        state.lastFetchedAt = Date.now();
        maybeWriteRunsCache(data);
        if (myEpoch !== listEpoch) return;
        if (state.view === 'detail') return;
        rerender();
      })
      .catch(function (err) {
        if (myEpoch !== listEpoch) return;
        if (state.view === 'detail') return;
        state.lastError = err;
        // Sem rede e sem dados vivos: cai no cache persistido (gate 8) em
        // vez de mostrar lista vazia. 401 tem caminho próprio (reconectar).
        if (!isAuthError(err) && state.lastRuns === null) {
          applyCachedRuns(err);
        }
        ensureSessionOrAuth(err);
        rerender();
      });
  }

  function isAuthError(err) {
    return !!(err && (err.kind === 'unauthorized' || err.status === 401));
  }

  // Aplica o cache como último estado conhecido quando o servidor não responde.
  // A lista só é considerada "de cache" se estava vazia: um erro de poll com
  // dados frescos na tela não deve fingir que está em cache.
  function applyCachedRuns(err) {
    var cached = readRunsCache();
    if (!cached) return false;
    state.lastRuns = cached.runs;
    state.fromCache = true;
    state.cacheSavedAt = cached.savedAt;
    state.cacheTruncated = !!cached.truncated;
    state.lastFetchedAt = 0;
    state.lastError = err || httpError('network', 0, 'Sem conexão.');
    return true;
  }

  function ensureSessionOrAuth(err) {
    if (err && err.kind === 'unauthorized') {
      // 401: não apaga automaticamente o token (o maestro confere antes).
      // A tela de erro mostra o botão "Reconectar".
    }
  }

  function refreshDetail(silent) {
    var d = state.detail;
    if (!d || !d.id) return;
    // Enquanto a tela não tem o primeiro Run (resposta em voo), o poll não
    // dispara — evita corrida entre o /run do openDetail e o /run do timer.
    if (!d.run) return;
    var id = d.id;
    return fetchJSON('/run/' + encodeURIComponent(id))
      .then(function (run) {
        if (!run || typeof run !== 'object') throw httpError('http', 0, 'Resposta inesperada do servidor.');
        // O usuário pode ter voltado para a lista no meio do poll.
        if (!state.detail || state.detail.id !== id) return;
        // O servidor pode devolver null/lista em vez de um Run quando a rota
        // não existe ou o dado corrompeu: preserva o último Run válido.
        if (Array.isArray(run) || !run.id) throw httpError('http', 0, 'Resposta inesperada do servidor.');
        state.detail.run = run;
        state.detail.error = null;
        if (state.view === 'detail') rerender();
      })
      .catch(function (err) {
        if (!state.detail || state.detail.id !== id) return;
        state.detail.error = err;
        if (state.view === 'detail') rerender();
      });
  }

  function openDetail(id) {
    // Reabrir o MESMO run em carregamento (evento repetido, retorno de
    // aba, re-render) não pode resetar o modo nem recriar o log.
    if (state.view === 'detail' && state.detail && state.detail.id === id) {
      startDetailPoll();
      return;
    }
    stopPoll();
    listEpoch++; // invalida qualquer /runs em voo
    state.view = 'detail';
    state.detail = { id: id, run: null, error: null };
    state.detailMode = 'summary';
    state.log = null;
    state.logStep = null;
    state.logSubstep = null;
    state.logWrap = readPrefs().logWrap || false;
    state.autoFollow = true;
    render();
    // O render inicial mostra o cabeçalho com o id. O /run real chega em
    // seguida; se a resposta for inválida, o estado de erro aparece sem
    // derrubar a tela para a lista.
    fetchJSON('/run/' + encodeURIComponent(id))
      .then(function (run) {
        if (!state.detail || state.detail.id !== id) return;
        // Rota ausente/corrompida: NÃO apaga o Run que já está na tela.
        if (!run || typeof run !== 'object' || Array.isArray(run) || !run.id) {
          state.detail.error = httpError('http', 0, 'Execução não reconhecida pelo servidor.');
          render();
          return;
        }
        state.detail.run = run;
        state.detail.error = null;
        // Repinta só a tela do run: um render() completo poderia ser
        // capturado por outro caminho (nav, visibility) no meio da leitura.
        if (state.view === 'detail' && state.detail.id === id) rerender();
        startDetailPoll();
      })
      .catch(function (err) {
        if (!state.detail || state.detail.id !== id) return;
        state.detail.error = err;
        render();
        // 404 é definitivo; demais erros seguem tentando no próximo manual
      });
  }

  function readPrefs() {
    try { return JSON.parse(localStorage.getItem(LS_PREFS) || '{}') || {}; }
    catch (_) { return {}; }
  }

  // --------------------------------------------------------------------
  // Sheets de ação: disparar fluxo, conexão, reexecutar
  // --------------------------------------------------------------------
  function openTriggerSheet(preset) {
    if (!state.token) { openSettingsSheet(); return; }
    var body = el('div', {});
    var head = el('div', { class: 'sheet__head' });
    head.appendChild(el('h2', { class: 'sheet__title', text: 'Disparar fluxo' }));
    body.appendChild(head);

    var content = el('div', { class: 'sheet__body' });
    var form = el('form', { id: 'trigger-form' });
    form.setAttribute('novalidate', 'true');

    var wfField = el('div', { class: 'field' });
    wfField.appendChild(el('label', { class: 'field__label', for: 'trigger-wf', text: 'Fluxo' }));
    var wfSel = el('select', { class: 'field__select', id: 'trigger-wf', 'data-autofocus': 'true' });
    var wfKnown = knownWorkflows();
    if (wfKnown.length === 0) {
      var optEmpty = el('option', { value: '', text: 'Digite o identificador do fluxo abaixo' });
      wfSel.appendChild(optEmpty);
    } else {
      wfKnown.forEach(function (w) {
        var o = el('option', { value: w.id, text: w.name + ' (' + w.id + ')' });
        wfSel.appendChild(o);
      });
    }
    wfField.appendChild(wfSel);
    wfField.appendChild(el('p', { class: 'field__hint', text: 'A lista vem das execuções recentes. Se o fluxo ainda não rodou, informe o identificador no campo abaixo.' }));
    form.appendChild(wfField);

    var manualField = el('div', { class: 'field' });
    manualField.appendChild(el('label', { class: 'field__label', for: 'trigger-wf-manual', text: 'Identificador do fluxo (opcional)' }));
    var wfInput = el('input', { class: 'field__input', id: 'trigger-wf-manual', type: 'text', placeholder: 'ex.: deploy-api' });
    wfInput.autocomplete = 'off';
    wfInput.spellcheck = false;
    manualField.appendChild(wfInput);
    form.appendChild(manualField);

    var repoField = el('div', { class: 'field' });
    repoField.appendChild(el('label', { class: 'field__label', for: 'trigger-repo', text: 'Projeto (opcional)' }));
    var repoInput = el('input', { class: 'field__input', id: 'trigger-repo', type: 'text', placeholder: 'ex.: usuario/repo', value: preset && preset.repo ? preset.repo : '' });
    repoInput.autocomplete = 'off';
    repoField.appendChild(repoInput);
    form.appendChild(repoField);

    var brField = el('div', { class: 'field' });
    brField.appendChild(el('label', { class: 'field__label', for: 'trigger-branch', text: 'Branch' }));
    var brInput = el('input', { class: 'field__input', id: 'trigger-branch', type: 'text', value: (preset && preset.branch) || 'main' });
    brInput.autocomplete = 'off';
    brField.appendChild(brInput);
    form.appendChild(brField);

    var errBox = el('p', { class: 't-small', style: 'color:var(--danger);display:none' });
    form.appendChild(errBox);
    content.appendChild(form);
    body.appendChild(content);

    var foot = el('div', { class: 'sheet__foot' });
    var submit = el('button', { class: 'btn btn--primary btn--block', type: 'submit', form: 'trigger-form' });
    submit.appendChild(icon('play'));
    submit.appendChild(el('span', { class: 'btn__label', text: 'Disparar' }));
    var cancel = el('button', { class: 'btn btn--ghost btn--block', type: 'button', text: 'Cancelar' });
    cancel.addEventListener('click', closeSheet);
    foot.appendChild(submit);
    foot.appendChild(cancel);
    body.appendChild(foot);

    var opened = openSheet(body, { label: 'Disparar fluxo' });
    submit.addEventListener('click', function (e) { e.preventDefault(); submitTrigger(); });
    form.addEventListener('submit', function (e) { e.preventDefault(); submitTrigger(); });

    function submitTrigger() {
      var wf = (wfInput.value || '').trim() || (wfSel.value || '').trim();
      if (!wf) {
        errBox.textContent = 'Informe o identificador do fluxo.';
        errBox.style.display = '';
        wfInput.focus();
        return;
      }
      submit.disabled = true;
      cancel.disabled = true;
      submit.querySelector('.btn__label').textContent = 'Disparando…';
      var payload = {
        workflow_id: wf,
        trigger_label: 'disparado pelo celular',
        repo: (repoInput.value || '').trim(),
        branch: (brInput.value || 'main').trim() || 'main',
        actor: 'maestro (mobile)'
      };
      apiTrigger(payload).then(function (res) {
        closeSheet();
        toast('Fluxo disparado.', 'ok');
        if (res && res.run_id) {
          setTimeout(function () { openDetail(res.run_id); }, 200);
        } else {
          refreshNow();
        }
      }).catch(function (err) {
        submit.disabled = false;
        cancel.disabled = false;
        submit.querySelector('.btn__label').textContent = 'Disparar';
        if (err && err.kind === 'notfound') {
          errBox.textContent = 'O servidor não reconhece esse fluxo. Confira o identificador.';
        } else if (err && err.kind === 'forbidden') {
          errBox.textContent = 'O servidor recusou: esse token não pode disparar este fluxo.';
        } else {
          errBox.textContent = errorText(err, 'Não foi possível disparar.');
        }
        errBox.style.display = '';
      });
    }
  }

  function knownWorkflows() {
    var map = {};
    (state.lastRuns || []).forEach(function (r) {
      if (!r.workflow_id) return;
      if (!map[r.workflow_id]) map[r.workflow_id] = { id: r.workflow_id, name: r.workflow_name || r.workflow_id };
    });
    return Object.keys(map).map(function (k) { return map[k]; });
  }

  // Conexão / Ajustes — endereço + token (armazenado em localStorage; nunca
  // no HTML nem logado no console).
  function openSettingsSheet() {
    var body = el('div', {});
    var head = el('div', { class: 'sheet__head' });
    head.appendChild(el('h2', { class: 'sheet__title', text: 'Conexão' }));
    body.appendChild(head);

    var content = el('div', { class: 'sheet__body' });
    var info = el('div', { class: 'card card--info' });
    info.appendChild(el('p', { class: 't-small', text: 'Informe o endereço do runner da VPS e o token de acesso. O token fica salvo apenas no seu aparelho.' }));
    content.appendChild(info);

    var form = el('form', { id: 'settings-form' });
    var baseField = el('div', { class: 'field' });
    baseField.appendChild(el('label', { class: 'field__label', for: 'settings-base', text: 'Endereço do runner' }));
    var baseInput = el('input', {
      class: 'field__input', id: 'settings-base', type: 'url', inputmode: 'url',
      placeholder: 'http://10.0.0.5:8787', value: state.base || '', 'data-autofocus': 'true'
    });
    baseInput.autocomplete = 'url';
    baseInput.spellcheck = false;
    baseField.appendChild(baseInput);
    baseField.appendChild(el('p', { class: 'field__hint', text: 'Ex.: http://192.168.0.10:8787 ou a URL do túnel.' }));
    form.appendChild(baseField);

    var tokField = el('div', { class: 'field' });
    tokField.appendChild(el('label', { class: 'field__label', for: 'settings-token', text: 'Token do runner' }));
    var tokInput = el('input', {
      class: 'field__input', id: 'settings-token', type: 'password',
      placeholder: 'cole o token aqui', autocomplete: 'off', autocapitalize: 'off', spellcheck: 'false'
    });
    tokField.appendChild(tokInput);
    tokField.appendChild(el('p', { class: 'field__hint', text: state.token ? 'Token salvo. Deixe em branco para manter o atual.' : 'Encontre em /etc/workploy/token na VPS.' }));
    form.appendChild(tokField);

    var srvField = el('div', { class: 'field' });
    srvField.appendChild(el('label', { class: 'field__label', for: 'settings-name', text: 'Nome da VPS (opcional)' }));
    var srvInput = el('input', { class: 'field__input', id: 'settings-name', type: 'text', value: state.server || '', placeholder: 'VPS principal' });
    srvInput.autocomplete = 'off';
    srvField.appendChild(srvInput);
    form.appendChild(srvField);

    var errBox = el('p', { class: 't-small', style: 'color:var(--danger);display:none' });
    form.appendChild(errBox);
    content.appendChild(form);
    body.appendChild(content);

    var foot = el('div', { class: 'sheet__foot' });
    var save = el('button', { class: 'btn btn--primary btn--block', type: 'submit', form: 'settings-form', text: 'Conectar' });
    var close = el('button', { class: 'btn btn--ghost btn--block', type: 'button', text: 'Fechar' });
    close.addEventListener('click', closeSheet);
    var forget = el('button', { class: 'btn btn--ghost btn--block', type: 'button', text: 'Esquecer token deste aparelho' });
    forget.addEventListener('click', function () {
      clearSession();
      closeSheet();
      state.view = 'runs';
      state.lastRuns = null;
      state.lastError = null;
      render();
      openSettingsSheet();
    });
    foot.appendChild(save);
    foot.appendChild(close);
    foot.appendChild(forget);
    body.appendChild(foot);

    openSheet(body, { label: 'Conexão com o runner' });
    save.addEventListener('click', function (e) {
      e.preventDefault();
      var base = (baseInput.value || '').trim();
      var tok = (tokInput.value || '').trim();
      var srv = (srvInput.value || '').trim();
      if (!/^https?:\/\//i.test(base)) {
        errBox.textContent = 'O endereço deve começar com http:// ou https://.';
        errBox.style.display = '';
        return;
      }
      if (!tok && !state.token) {
        errBox.textContent = 'Informe o token do runner.';
        errBox.style.display = '';
        return;
      }
      save.disabled = true;
      save.textContent = 'Testando…';
      var oldBase = state.base, oldToken = state.token, oldServer = state.server;
      state.base = base.replace(/\/+$/, '');
      state.token = tok || state.token;
      state.server = srv;
      fetchJSON('/status').then(function () {
        var sess = writeSession(state.base, state.token, state.server);
        state.token = sess.token;
        closeSheet();
        toast('Conectado ao runner.', 'ok');
        state.lastRuns = null;
        state.fromCache = false;
        state.cacheSavedAt = 0;
        state.lastError = null;
        state.view = 'runs';
        render();
        refreshNow();
      }).catch(function (err) {
        // Reverte a sessão em memória se a tentativa falhou antes de salvar.
        state.base = oldBase; state.token = oldToken; state.server = oldServer;
        save.disabled = false;
        save.textContent = 'Conectar';
        if (err && err.kind === 'unauthorized') errBox.textContent = 'O servidor recusou este token.';
        else if (err && err.kind === 'network') errBox.textContent = 'Não foi possível alcançar o endereço.';
        else if (err && err.kind === 'notfound') errBox.textContent = 'O endereço respondeu, mas não é o runner do Workploy.';
        else if (err && err.kind === 'timeout') errBox.textContent = 'O servidor demorou para responder.';
        else errBox.textContent = errorText(err, 'Falha ao conectar.');
        errBox.style.display = '';
        if (oldBase && oldToken) {
          // mantém a antiga sessão em memória intacta
        }
      });
    });
  }

  // --------------------------------------------------------------------
  // Navegação inferior (4 itens, badge)
  // --------------------------------------------------------------------
  function renderNav() {
    var nav = $('#nav');
    var runs = state.lastRuns;
    var active = runs ? activeCount(runs) : 0;
    var pending = runs ? approvalCount(runs) : 0;
    var allRuns = runs || [];
    var failing = allRuns.filter(function (r) { return r.status === 'Failed'; }).length;
    var live = allRuns.filter(function (r) { return r.status === 'Running' || r.status === 'Queued'; }).length;

    var items = [
      { id: 'runs', label: 'Execuções', icon: 'runs', badge: active ? String(active) : null },
      { id: 'projects', label: 'Projetos', icon: 'projects', badge: failing ? '' : null, badgeClass: failing ? 'nav-badge--danger nav-badge__dot' : '' },
      { id: 'approvals', label: 'Aprovações', icon: 'approvals', badge: pending ? String(pending) : null, badgeClass: pending ? 'nav-badge--alert' : '' },
      { id: 'connections', label: 'Conexões', icon: 'connections', badge: live ? '' : null, badgeClass: 'nav-badge--ok nav-badge__dot' }
    ];

    nav.innerHTML = '';
    items.forEach(function (it) {
      var btn = el('button', { class: 'nav-item', type: 'button' });
      btn.setAttribute('aria-label', it.label + (it.badge ? ' (' + it.badge + ')' : ''));
      if (state.view === it.id || (state.view === 'detail' && it.id === 'runs')) {
        btn.setAttribute('aria-current', 'page');
      }
      var ic = el('span', { class: 'nav-item__icon' });
      ic.appendChild(icon(it.icon));
      btn.appendChild(ic);
      btn.appendChild(el('span', { class: 'nav-item__label', text: it.label }));
      if (it.badge) {
        var b = el('span', { class: 'nav-badge ' + (it.badgeClass || ''), text: it.badge });
        if (!it.badge) b.textContent = '';
        btn.appendChild(b);
      }
      btn.addEventListener('click', function () {
        stopPoll();
        closeSheet();
        state.view = it.id;
        state.detail = null;
        state.log = null;
        render();
        if (it.id === 'runs' || it.id === 'projects' || it.id === 'approvals') {
          startListPoll();
          refreshRunsSilent();
        }
      });
      nav.appendChild(btn);
    });
  }

  // --------------------------------------------------------------------
  // Render raiz
  // --------------------------------------------------------------------
  function rerender() {
    render();
  }

  function render() {
    var screen = $('#screen');
    if (!screen) return;
    screen.innerHTML = '';
    screen.className = 'screen';

    if (!state.token) {
      renderConnect(screen);
      renderNav();
      setFab(false);
      stopPoll();
      return;
    }

    // Um render assíncrono nunca pode pintar "detail" sem detalhe: se não há
    // run carregado, volta para a lista em vez de mostrar tela vazia com
    // cabeçalho genérico.
    if (state.view === 'detail' && (!state.detail || !state.detail.id)) {
      state.view = 'runs';
      state.log = null;
    }
    // O log só existe dentro de um run. Sair do detalhe por QUALQUER caminho
    // (voltar, nav, Escape) limpa o cursor; senão um render futuro herda um
    // log órfão e a tela de Logs pinta em branco.
    if (state.view !== 'detail') {
      state.log = null;
    }

    if (state.view === 'runs') renderRuns(screen);
    else if (state.view === 'projects') renderProjects(screen);
    else if (state.view === 'approvals') renderApprovals(screen);
    else if (state.view === 'connections') renderConnections(screen);
    else if (state.view === 'detail') renderDetail(screen);
    else renderRuns(screen);

    renderNav();
    setFab(state.view === 'runs' || state.view === 'projects' || state.view === 'approvals');

    if (state.view === 'runs' || state.view === 'projects' || state.view === 'approvals') {
      startListPoll();
    }
    if (state.view === 'detail') {
      startDetailPoll();
      if (state.detailMode === 'logs') startLogPoll();
    }
  }

  function setFab(show) {
    var fab = $('#fab');
    if (!fab) return;
    fab.hidden = !show;
  }

  // Primeira tela sem sessão: explain, don't fake.
  function renderConnect(screen) {
    screen.appendChild(headerRow('Workploy', { settings: false }));
    var body = el('div', { class: 'screen__body' });
    var intro = el('div', { class: 'card' });
    intro.appendChild(el('p', { class: 't-cap', text: 'Conectar ao runner' }));
    intro.appendChild(el('p', { class: 't-body', text: 'O Workploy Mobile lê as execuções do runner da VPS. Informe o endereço do servidor e o token de acesso para começar.' }));
    body.appendChild(intro);
    var acts = el('div', { class: 'state__actions' });
    var open = el('button', { class: 'btn btn--primary', type: 'button', text: 'Conectar' });
    open.addEventListener('click', openSettingsSheet);
    acts.appendChild(open);
    body.appendChild(acts);
    body.appendChild(el('p', { class: 't-small muted', text: 'O token fica salvo neste aparelho, nunca embutido na página.' }));
    screen.appendChild(body);
  }

  // --------------------------------------------------------------------
  // Eventos globais: visibilidade, FAB, atalho de retorno
  // --------------------------------------------------------------------
  document.addEventListener('visibilitychange', function () {
    state.visible = !document.hidden;
    if (state.visible) resume();
    else {
      stopPoll();
      var d = document.querySelector('.log-live__dot');
      if (d) d.classList.add('is-paused');
    }
  });

  // Retoma o poll ao voltar para a aba/serviço. Idempotente: start* ignoram
  // se já existe timer, então dispara em todos os eventos que podem indicar
  // retorno à tela (visibilitychange, focus, pageshow, online).
  function resume() {
    if (document.hidden) return;
    state.visible = true;
    if (!state.token) return;
    if (state.view === 'runs' || state.view === 'projects' || state.view === 'approvals') {
      startListPoll();
      refreshRunsSilent();
    } else if (state.view === 'detail') {
      startDetailPoll();
      refreshDetail(true);
      if (state.detailMode === 'logs') { startLogPoll(); pollLogOnce(); }
    }
    var dot = document.querySelector('.log-live__dot');
    if (dot) dot.classList.remove('is-paused');
  }

  // Reobservação de foco/rede: um celular com tela bloqueada pode não
  // disparar visibilitychange ao despertar.
  window.addEventListener('focus', resume);
  window.addEventListener('pageshow', resume);
  window.addEventListener('online', resume);

  // Teclado: Escape fecha sheet (ver closeSheet já registrado) e retorna tela
  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape' && !activeSheet && state.view === 'detail') {
      state.view = 'runs'; state.detail = null; state.log = null; stopPoll(); render();
    }
  });

  window.addEventListener('popstate', function () {
    if (state.view === 'detail') {
      state.view = 'runs';
      state.detail = null;
      state.log = null;
      render();
    }
  });

  // --------------------------------------------------------------------
  // Boot
  // --------------------------------------------------------------------

  var booted = false;
  function boot() {
    // bindApp dispara na DOMContentLoaded e, no harness, também é chamado
    // direto: um boot duplo reabriria o detalhe e resetaria o modo de visão.
    if (booted) return;
    booted = true;
    // Gate 9 do §12: densidade ANTES da primeira tela para nada piscar em
    // 1:1 e depois "saltar" para 2,75.
    applyDensity();
    window.addEventListener('resize', applyDensity);
    window.addEventListener('orientationchange', applyDensity);
    var prefs = readPrefs();
    state.logWrap = !!prefs.logWrap;
    var fab = $('#fab');
    if (fab) fab.addEventListener('click', function () { openTriggerSheet(); });
    try {
      var f = localStorage.getItem(LS_LAST_FILTER);
      if (f === 'all' || f === 'active' || f === 'failures') state.filter = f;
    } catch (_) {}

    if (loadSession()) {
      // Respeita uma view pré-definida (drill-down por deep link ou harness).
      if (state.view == null) state.view = 'runs';
      if (state.view === 'detail' && state.detail && state.detail.id) {
        // Já há um run aberto: só repinta e religa o poll, sem resetar.
        render();
        startDetailPoll();
      } else {
        // Primeiro render: se houver cache, mostra já a lista guardada com a
        // faixa "sem conexão" enquanto o servidor responde. O refreshNow
        // substitui pelo estado vivo, ou mantém o cache se a rede falhar.
        if (state.lastRuns === null) applyCachedRuns(null);
        render();
        refreshNow();
      }
    } else {
      render();
      // não abre o sheet automaticamente: o botão "Conectar" é explícito.
    }
  }

  // Expõe para o harness de teste injetar dados sem rede.
  window.WorkployMobile = {
    version: '1.0.0',
    state: state,
    render: render,
    boot: boot,
    _test: {
      setSession: function (base, token, server) { writeSession(base, token, server); loadSession(); },
      setDetailRun: function (run) { if (state.detail) state.detail.run = run; },
      stopPolls: function () { stopPoll(); },
      setRuns: function (arr) { state.lastRuns = arr; state.lastError = null; state.lastFetchedAt = Date.now(); },
      applyDensity: applyDensity,
      readRunsCache: readRunsCache,
      writeRunsCache: writeRunsCache,
      setError: function (err) { state.lastError = err; },
      setView: function (v) { state.view = v; },
      // Transição de detalhe atômica: guarda a tela que o render deve mostrar
      // e zera log/modos. Antes, um refresh assíncrono que chamava render()
      // com `view=detail` e `detail=null` pintava o cabeçalho genérico
      // ("Execução"/"Execuções") e o usuário perdia a tela.
      gotoDetail: function (run) {
        state.view = 'detail';
        listEpoch++;
        state.detail = { id: run.id, run: run, error: null };
        // gotoDetail preserva o modo; se for inválido, renderDetail normaliza
        // para Resumo. Só um novo openDetail(id) volta a 'summary'.
        if (state.detailMode !== 'logs') state.detailMode = 'summary';
        state.log = null;
        state.logStep = null;
        state.logSubstep = null;
        state.logWrap = false;
        state.autoFollow = true;
        render();
        startDetailPoll();
      },
      openDetailWith: function (run) {
        this.gotoDetail(run);
      },
      setLog: function (log) { state.log = log; },
      setVisible: function (v) { state.visible = !!v; },
      setDetailMode: function (m) { state.detailMode = m; },
      pollLog: function () { return pollLogOnce(); },
      reloadLog: function () { reloadLogFromStart(); },
      renderNow: function () { render(); },
      // Testes: simula o "wiring" que o jsdom não entrega (visibilitychange
      // real, focus do window). Chama o handler de resume do produto.
    }
  };

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', boot);
  } else {
    boot();
  }
})();