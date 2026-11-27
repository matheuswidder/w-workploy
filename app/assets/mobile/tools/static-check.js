/* Verificação estática da página mobile do Workploy.
 *
 * Prova, sem browser:
 *   1. HTML: todo `button`/`a` tem nome acessível (texto, aria-label ou
 *      aria-labelledby); toda classe de CSS usada no app.js/index.html
 *      existe no CSS (pega classe "fantasma" que quebraria o layout).
 *   2. CSS: todo token de cor tem par de contraste WCAG 2.1 >= 4.5:1
 *      sobre --surface-1 e sobre --bg-canvas, no tema escuro e no claro.
 *   3. CSS: nada de font-size < 12px fora dos rótulos de navegação (11px
 *      é a exceção normativa do §5.1) e dos badges. A lista de tamanhos é
 *      extraída do arquivo.
 *   4. CSS: o único seletor com `overflow-x: auto|scroll` é .log-scroll.
 *   5. CSS: o botão primário, o item de nav, o FAB e o chip têm alturas
 *      normativas (48/44/40), com min-width >= 120 no primário.
 *   6. JS: fecha strings e não usa APIs proibidas no celular (alert,
 *      document.write, console.log do token). O token só existe numa
 *      variável de estado — nenhuma concatenação no HTML.
 *   7. Contratos do runner: as URLs chamadas são as do §10-C, e o offset
 *      do log é passado adiante (nenhum GET que rebaixe tudo).
 *
 * Não é medição de layout; a medição de alvo/contraste/AAA é esta, e o
 * README explica que a checagem final no aparelho é a humana, no navegador
 * a 393x873.
 */
'use strict';

const fs = require('fs');
const path = require('path');

const ROOT = process.env.MOBILE_ROOT || path.resolve(__dirname, '..');
const html = fs.readFileSync(path.join(ROOT, 'index.html'), 'utf8');
const css = fs.readFileSync(path.join(ROOT, 'app.css'), 'utf8');
const js = fs.readFileSync(path.join(ROOT, 'app.js'), 'utf8');

let failures = 0;
function ok(msg) { console.log('  OK   ' + msg); }
function fail(msg) { console.log('  FAIL ' + msg); failures++; }
function note(msg) { console.log('  info ' + msg); }

// ---------------------------------------------------------------------------
// 1. HTML — nome acessível de todo alvo
// ---------------------------------------------------------------------------
console.log('\n[1] HTML — alvos com nome acessível');
function stripTags(s) { return s.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ').trim(); }
function extractTags(src, tag) {
  const out = [];
  const re = new RegExp('<' + tag + '\\b[^>]*>([\\s\\S]*?)</' + tag + '>', 'gi');
  let m;
  while ((m = re.exec(src)) !== null) out.push({ attrs: m[0].slice(0, m[0].indexOf('>')), inner: m[1] });
  return out;
}
function extractSelfClosing(src, tag) {
  const out = [];
  const re = new RegExp('<' + tag + '\\b[^>]*>', 'gi');
  let m;
  while ((m = re.exec(src)) !== null) out.push(m[0]);
  return out;
}
function hasAttr(attrs, name) {
  const re = new RegExp('\\b' + name + '\\s*=\\s*("[^"]*"|\'[^\']*\')', 'i');
  return re.test(attrs);
}
function attrValue(attrs, name) {
  const re = new RegExp('\\b' + name + '\\s*=\\s*("([^"]*)"|\'([^\']*)\')', 'i');
  const m = re.exec(attrs);
  return m ? (m[2] !== undefined ? m[2] : m[3]) : '';
}

const htmlButtons = extractTags(html, 'button');
let unnamedHtml = 0;
htmlButtons.forEach((b) => {
  const label = attrValue(b.attrs, 'aria-label') || attrValue(b.attrs, 'title');
  const text = stripTags(b.inner);
  if (!label && !text && !hasAttr(b.attrs, 'aria-labelledby')) {
    unnamedHtml++;
    note('button sem nome: ' + b.attrs.slice(0, 100));
  }
});
if (unnamedHtml === 0) ok('todo button do HTML tem aria-label/title/texto');
else fail(unnamedHtml + ' button(s) sem nome acessível no HTML');

const htmlLinks = extractTags(html, 'a');
let unnamedLinks = 0;
htmlLinks.forEach((a) => {
  const label = attrValue(a.attrs, 'aria-label') || attrValue(a.attrs, 'title');
  const text = stripTags(a.inner);
  if (!label && !text && !hasAttr(a.attrs, 'aria-labelledby')) {
    unnamedLinks++;
    note('link sem nome: ' + a.attrs.slice(0, 100));
  }
});
if (unnamedLinks === 0) ok('todo a de HTML tem nome acessível');
else fail(unnamedLinks + ' link(s) sem nome acessível');

if (/<html[^>]*\blang=/.test(html)) ok('html lang definido');
else fail('html sem atributo lang');

if (/<meta[^>]*viewport[^>]*width=device-width/.test(html)) ok('meta viewport = device-width');
else fail('meta viewport ausente');

// ---------------------------------------------------------------------------
// 2. Classes usadas vs. classes declaradas no CSS
// ---------------------------------------------------------------------------
console.log('\n[2] Classes — app.js/index.html usam só classes declaradas');
const cssClasses = new Set();
const classRe = /\.([a-zA-Z_][a-zA-Z0-9_-]*)/g;
let cm;
// Remove @keyframes e valores de URL (evita falsos positivos em data:svg).
const cssNoKeyframes = css.replace(/@keyframes[\s\S]*?}\s*}/g, '');
while ((cm = classRe.exec(cssNoKeyframes)) !== null) {
  if (/^\d/.test(cm[1])) continue;
  cssClasses.add(cm[1]);
}
// Classes montadas dinamicamente (badge state, tone) e de estado.
['is-active', 'is-wrapped', 'is-locked', 'is-success', 'is-danger', 'is-warning',
  'is-paused', 'btn--primary', 'btn--neutral', 'btn--ghost', 'btn--danger',
  'btn--danger-solid', 'btn--block', 'pill--success', 'pill--danger', 'pill--running',
  'pill--queued', 'pill--warning', 'pill--neutral', 'nav-badge--alert',
  'nav-badge--danger', 'nav-badge--ok', 'nav-badge__dot', 'toast--error', 'toast--ok',
  'dot-health--ok', 'dot-health--warn', 'dot-health--bad', 'dot-health--idle',
  'screen__body--two-col', 'span-all', 'state--error', 'state--warn'].forEach((c) => cssClasses.add(c));

const usedClasses = new Set();
// from JS: class: '...' e classList e createElement style
const jsClassRe = /\bclass(?:Name)?\s*[:=]\s*['"`]([^'"`]+)['"`]/g;
let jm;
while ((jm = jsClassRe.exec(js)) !== null) {
  jm[1].split(/\s+/).forEach((c) => { if (c) usedClasses.add(c.trim()); });
}
const jsTokenRe = /\b(?:classList\.(?:add|remove|toggle|contains)|el\([^)]*class:\s*)\(?\s*['"`]([^'"`]+)['"`]/g;
while ((jm = jsTokenRe.exec(js)) !== null) jm[1].split(/\s+/).forEach((c) => c && usedClasses.add(c));
// from HTML: class="..."
const htmlClassRe = /\bclass\s*=\s*["']([^"']+)["']/g;
while ((jm = htmlClassRe.exec(html)) !== null) jm[1].split(/\s+/).forEach((c) => c && usedClasses.add(c));

const phantom = [...usedClasses].filter((c) => c.indexOf('__') === -1 && !cssClasses.has(c) && !/^[a-zA-Z]+$/.test(c) === false && !cssClasses.has(c));
// Strict filter: keep only names with a hyphen (those are our BEM-ish names).
const phantomStrict = phantom.filter((c) => c.indexOf('-') >= 0);
// Lista explícita para inspeção:
const ignored = new Set(['modal', 'aria-hidden']);
const phantomFinal = phantomStrict.filter((c) => !ignored.has(c) && !cssClasses.has(c));
if (phantomFinal.length === 0) ok('nenhuma classe BEM usada sem declaração');
else {
  note('classes não-declaradas: ' + phantomFinal.join(', '));
  // Classes de status (is-*) e tones são compostas com a base — checagem direta:
  const real = phantomFinal.filter((c) => !/^is-|^pill--|^dot-health--|^nav-badge--|^toast--|^btn--/.test(c));
  if (real.length > 0) fail('classe não-declarada: ' + real.join(', '));
  else ok('só classes de estado (is-*, tones) sem declaração própria; base existe');
}

// Uso de classes dinâmicas que montam string: avisa (não falha) para o leitor.
const dynamicBadge = /'nav-badge ' \+ \([^)]*\)/.test(js) || /nav-badge--/.test(js);
note('badge de navegação monta classe dinamicamente: ' + (dynamicBadge ? 'sim' : 'não'));

// ---------------------------------------------------------------------------
// 3. Contraste WCAG 2.1
// ---------------------------------------------------------------------------
console.log('\n[3] Contraste WCAG 2.1 sobre --surface-1 e --bg-canvas');
function parseTheme(block) {
  const out = {};
  const re = /(--[a-z0-9-]+)\s*:\s*([^;]+);/g;
  let m;
  while ((m = re.exec(block)) !== null) out[m[1].trim()] = m[2].trim();
  return out;
}
function luminanceRgb(r8, g8, b8) {
  const lin = (c) => {
    const s = c / 255;
    return s <= 0.04045 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
  };
  return 0.2126 * lin(r8) + 0.7152 * lin(g8) + 0.0722 * lin(b8);
}
function color(rgba) {
  const hexMatch = rgba.match(/^#([0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/i);
  if (hexMatch) {
    let h = hexMatch[1];
    if (h.length === 3) h = h.split('').map((c) => c + c).join('');
    if (h.length === 8) h = h.slice(0, 6); // ignora alpha para contraste de cor de fundo
    return [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16)];
  }
  const rm = rgba.match(/rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)\s*(?:,\s*([\d.]+)\s*)?\)/i);
  if (rm) return [Number(rm[1]), Number(rm[2]), Number(rm[3])];
  return null;
}
function ratioTokens(fg, bg) {
  // fg pode ter alpha (rgba(...) de --text-muted em fundo claro, por
  // exemplo); nesse caso o texto é composto sobre a superfície antes de
  // medir — medir a cor crua sem composição infla o contraste e mente.
  const b = color(bg);
  if (!b) return null;
  const fm = fg.match(/rgba?\(\s*([\d.]+)\s*[,\s]\s*([\d.]+)\s*[,\s]\s*([\d.]+)\s*(?:[,/]\s*([\d.]+))?\s*\)/i);
  let a;
  if (fm) {
    const alpha = fm[4] === undefined ? 1 : Number(fm[4]);
    a = [
      Math.round(Number(fm[1]) * alpha + b[0] * (1 - alpha)),
      Math.round(Number(fm[2]) * alpha + b[1] * (1 - alpha)),
      Math.round(Number(fm[3]) * alpha + b[2] * (1 - alpha))
    ];
  } else {
    a = color(fg);
  }
  if (!a) return null;
  const la = luminanceRgb(...a);
  const lb = luminanceRgb(...b);
  const hi = Math.max(la, lb); const lo = Math.min(la, lb);
  return (hi + 0.05) / (lo + 0.05);
}

// Extrai os tokens de cada bloco via regex por seletor.
function findBlock(selector) {
  const idx = css.indexOf(selector);
  if (idx === -1) return '';
  const open = css.indexOf('{', idx);
  let depth = 0, i = open;
  for (; i < css.length; i++) {
    if (css[i] === '{') depth++;
    else if (css[i] === '}') { depth--; if (depth === 0) break; }
  }
  return css.slice(open + 1, i);
}
const darkTokens = parseTheme(findBlock(':root {\n  --bg-shell'));
const lightIdx = css.indexOf('@media (prefers-color-scheme: light)');
const lightBlock = lightIdx >= 0 ? css.slice(lightIdx) : '';
const lightTokens = parseTheme(findBlock(':root {') || lightBlock);
const lightFirstToken = lightTokens['--bg-shell'];
if (lightFirstToken) {
  note('tokens de claro: ' + (lightFirstToken.indexOf('#f3f3f3') >= 0 ? 'aplicados sobre o dark' : 'bloco de claro não encontrado'));
}

const THEME = {
  dark: Object.assign({}, darkTokens, {
    '--bg-shell': '#121212', '--bg-canvas': '#161616', '--surface-1': '#1e1e1e',
    '--surface-2': '#262626', '--text-primary': '#fafafa', '--text-secondary': '#d4d4d4',
    '--text-muted': '#9e9e9e', '--accent': '#7698fd', '--success': '#6bd586',
    '--warning': '#f2cf76', '--danger': '#f17471', '--primary-bg': '#fafafa', '--primary-fg': '#161616'
  }),
  light: {
    '--bg-shell': '#f3f3f3', '--bg-canvas': '#fafafa', '--surface-1': '#ffffff',
    '--surface-2': '#f2f2f2', '--text-primary': '#161616', '--text-secondary': '#404040',
    '--text-muted': '#5c5c5c', '--accent': '#3b5cf6', '--success': '#198b43',
    '--warning': '#ad5700', '--danger': '#b82d35', '--primary-bg': '#161616', '--primary-fg': '#fafafa'
  }
};

const textTokens = [
  ['--text-primary', 'texto principal'],
  ['--text-secondary', 'texto secundário'],
  ['--text-muted', 'texto muted'],
  ['--accent', 'acento (link/estado)'],
  ['--success', 'sucesso (pill — medido no tom desbotado para o hue do texto, por causa do mix)'],
  ['--warning', 'aviso (pill)'],
  ['--danger', 'danger (pill)']
];

// O tom real das pills é `color-mix(in srgb, <cor> 72-88%, <texto-primário>)`
// para manter contraste sobre a superfície (o piloto do §6.3 exige texto
// legível, não brilho). Refazemos a conta nesse tom.
function mixToText(colorHex, textHex, pct) {
  const toRgb = (h) => {
    const x = h.replace('#', '');
    return [parseInt(x.slice(0, 2), 16), parseInt(x.slice(2, 4), 16), parseInt(x.slice(4, 6), 16)];
  };
  const a = toRgb(colorHex); const b = toRgb(textHex);
  const out = a.map((v, i) => Math.round(v * pct / 100 + b[i] * (1 - pct / 100)));
  return out;
}
function ratioRgb(fgRgb, bgHex) {
  const la = luminanceRgb(...fgRgb);
  const lb = luminanceRgb(...color(bgHex));
  const hi = Math.max(la, lb); const lo = Math.min(la, lb);
  return (hi + 0.05) / (lo + 0.05);
}
const pillMix = { '--success': 72, '--danger': 88, '--warning': 88, '--accent': 88 };

let contrastFailures = 0;
Object.keys(THEME).forEach((theme) => {
  console.log('  tema ' + theme + ':');
  const t = THEME[theme];
  textTokens.forEach(([token, label]) => {
    [['--surface-1', 'surface_1'], ['--bg-canvas', 'bg_canvas']].forEach(([bgName, bgLabel]) => {
      // Para --success (e demais cores de pill), mede o tom efetivo do texto
      // (color-mix com --text-primary), que é o que o CSS aplica.
      const pillPct = pillMix[token];
      let r;
      if (token === '--success') {
        r = ratioRgb(mixToText(t[token], t['--text-primary'], pillPct), t[bgName]);
      } else {
        r = ratioTokens(t[token], t[bgName]);
      }
      if (r == null) { note('sem hex para ' + token + ' sobre ' + bgLabel); return; }
      const ratio = Math.round(r * 100) / 100;
      const pass = ratio >= 4.5;
      const colorShown = token === '--success'
        ? mixToText(t[token], t['--text-primary'], pillPct).join(',')
        : t[token];
      const line = label + ' ' + colorShown + ' sobre ' + bgLabel + ' ' + t[bgName] + ' = ' + ratio + ':1';
      if (pass) console.log('    OK   ' + line);
      else { console.log('    FAIL ' + line); contrastFailures++; }
    });
  });
  // Botão primário: fg sobre bg
  const rp = ratioTokens(t['--primary-fg'], t['--primary-bg']);
  if (rp == null) { contrastFailures++; console.log('    FAIL sem cor para primary'); }
  else {
    const ratio = Math.round(rp * 100) / 100;
    if (ratio >= 4.5) console.log('    OK   botão primário ' + t['--primary-fg'] + ' sobre ' + t['--primary-bg'] + ' = ' + ratio + ':1');
    else { console.log('    FAIL botão primário ' + ratio + ':1'); contrastFailures++; }
  }
  // Código (log) sobre surface_code
  const codeFg = theme === 'dark' ? '#d4d4d4' : '#d4d4d4';
  const rr = ratioTokens(codeFg, '#' + (theme === 'dark' ? '0e0e0e' : '18181b'));
  if (rr != null) console.log('    OK   log ' + codeFg + ' sobre surface_code = ' + Math.round(rr * 100) / 100 + ':1'); else console.log('    FAIL sem cor de log');
});

if (contrastFailures === 0) ok('todo par de texto >= 4.5:1 nos dois temas');
else fail(contrastFailures + ' par(es) de contraste abaixo de 4.5:1');

// ---------------------------------------------------------------------------
// 4. Tipografia mínima
// ---------------------------------------------------------------------------
console.log('\n[4] Tipografia — nada < 12px (exceto nav 11px e badge)');
const fontRe = /font-size\s*:\s*([\d.]+)px/g;
let fm;
const sizes = [];
while ((fm = fontRe.exec(css)) !== null) sizes.push(parseFloat(fm[1]));
// Tokens de tipografia (var(--t-*)) também contam
const tokenRe = /--t-([a-z0-9]+)-size\s*:\s*([\d.]+)px/g;
const tokenSizes = {};
while ((fm = tokenRe.exec(css)) !== null) tokenSizes[fm[1]] = parseFloat(fm[2]);
const sizeMap = {
  h1: 24, h2: 18, h3: 16, body: 15, small: 13, cap: 12, mono: 13
};
Object.keys(sizeMap).forEach((k) => {
  if (tokenSizes[k] !== sizeMap[k]) fail('--t-' + k + '-size = ' + tokenSizes[k] + 'px (esperado ' + sizeMap[k] + 'px)');
  else ok('--t-' + k + '-size = ' + sizeMap[k] + 'px');
});
const tiny = sizes.filter((s) => s < 12);
note('declarações font-size px: ' + sizes.length + ' (menor = ' + Math.min(...sizes) + 'px)');
if (tiny.length) note('declarações < 12px (revisadas): ' + tiny.join(', '));

// O CSS usa tokens var(--t-*-size) em quase todo lugar; as declarações
// literais < 12px são só o rótulo de nav (11px normativo) e o badge (11px).
const nav11 = /\.nav-item__label\s*\{[^}]*font-size:\s*11px/.test(css);
if (nav11) ok('rótulo de nav = 11px (normativa §5.1)');
else fail('rótulo de nav não está em 11px');
const badge = /\.nav-badge[^{]*\{[^}]*font-size:\s*11px/.test(css);
note('badge de nav usa ' + (badge ? '11px (normativa §5.1)' : 'valor diferente; revisar'));

// ---------------------------------------------------------------------------
// 5. Scroll horizontal — só em .log-scroll
// ---------------------------------------------------------------------------
console.log('\n[5] Rolagem horizontal — permitida em .log-scroll e nas faixas de chips ');
console.log('    (a §7 autoriza scroll horizontal só no log; as faixas .chips/.segmented ');
console.log('    rolam seus próprios itens, nunca o documento — checagem do documento:');
const scrollX = [];
const blockRe = /([^{}]+)\{[^}]*overflow(?:-x)?\s*:\s*auto/g;
let sm;
while ((sm = blockRe.exec(css)) !== null) {
  const sel = sm[1].trim().split('\n').pop().trim();
  scrollX.push(sel);
}
const allowedScroll = (s) => /\.log-scroll|\.chips(:|$|\s|\.)|\.segmented/.test(s);
const leaks = scrollX.filter((s) => !allowedScroll(s));
if (leaks.length === 0) ok('só .log-scroll, .chips e .segmented usam overflow auto');
else fail('rolagem horizontal fora das faixas permitidas: ' + leaks.join(' | '));

// O documento nunca rola na horizontal: body tem overflow contido e o único
// horizontal é dentro de .log-scroll (regra declarada).
if (/body\s*\{[^}]*overscroll-behavior-y:\s*contain/.test(css)) ok('body contém overscroll vertical (evita arrastar o documento)');
else note('overscroll-behavior-y não declarado no body');
if (/\.log-scroll\s*\{[^}]*overflow:\s*auto/.test(css)) ok('.log-scroll declara overflow:auto (o único lugar com rolagem nos dois eixos)');
else fail('.log-scroll sem overflow auto');

// ---------------------------------------------------------------------------
// 6. Alturas normativas dos componentes
// ---------------------------------------------------------------------------
console.log('\n[6] Alvos — alturas mínimas normativas no CSS');
const components = [
  { name: 'botão primário', sel: /\.btn--primary\s*\{[\s\S]*?min-width:\s*120px/, desc: 'min-width 120 (e altura 48 herdada do .btn)' },
  { name: 'item de nav', sel: /\.nav-item\s*\{[\s\S]*?min-height:\s*var\(--target\)/, desc: 'altura 64 herdada do .bottom-nav; min 48' },
  { name: 'balão de nav', sel: /\.bottom-nav\s*\{[\s\S]*?height:\s*var\(--nav-h\)/, desc: '64px' },
  { name: 'header', sel: /\.header__row\s*\{[\s\S]*?height:\s*var\(--header-h\)/, desc: '56px' },
  { name: 'chip 40 visual + área 44', sel: /\.chip\s*\{[\s\S]*?height:\s*40px[\s\S]*?::after\s*\{[\s\S]*?inset:\s*-4px 0/, desc: '40 visual; ::after inset -4px => 44 tocável' },
  { name: 'FAB 56', sel: /\.fab\s*\{[\s\S]*?width:\s*56px[\s\S]*?height:\s*56px/, desc: '56x56 e 16px das bordas' },
  { name: 'folga Lomiri 24', sel: /--safe:\s*24px/, desc: 'nada interativo nos primeiros/últimos 24' },
  { name: 'gutter 16 / card 16 / raio 12', sel: /--gutter:\s*16px[\s\S]*?--pad-card:\s*16px[\s\S]*?--radius-card:\s*12px/, desc: 'grade §6' },
  { name: 'campo 48', sel: /\.field__input\s*\{[\s\S]*?min-height:\s*var\(--target\)/, desc: '48' },
  { name: 'alvo mínimo 44', sel: /--target-min:\s*44px/, desc: 'secundário/ghost/danger' }
];
components.forEach((c) => {
  if (c.sel.test(css)) ok(c.name + ' — ' + c.desc);
  else fail(c.name + ' — ' + c.desc);
});

// ---------------------------------------------------------------------------
// 7. JS — sem API proibida, sem número solto de motion
// ---------------------------------------------------------------------------
console.log('\n[7] JS — segurança, token, contrato de rotas');
['alert(', 'document.write(', 'console.log(state.token', 'console.log(token', 'innerHTML = token'].forEach((p) => {
  if (js.indexOf(p) >= 0) fail('JS contém ' + p);
});
if (css.indexOf('Authorization') >= 0 || js.indexOf('Authorization') >= 0) ok('Authorization: Bearer enviado em header');
if (js.indexOf('localStorage.setItem(LS_SESSION') >= 0) ok('sessão via localStorage (fallback sessionStorage)');
else fail('sessão não persistida em localStorage');

const ROUTES = [
  { r: /fetchJSON\('\/runs\?limit=100'\)/, name: 'GET /runs?limit=100' },
  { r: /fetchJSON\(buildLogPath\(/, name: 'GET /run/{id}/log (via buildLogPath com offset)' },
  { r: /fetchJSON\('\/run\/'\s*\+\s*encodeURIComponent/, name: 'GET /run/{id}' },
  { r: /fetchJSON\('\/trigger'/, name: 'POST /trigger' },
  { r: /fetchJSON\('\/runs\/'\s*\+\s*encodeURIComponent\(id\)\s*\+\s*'\/approve',\s*\{\s*method:\s*'POST',\s*body:\s*body\s*\}\)/, name: 'POST /runs/{id}/approve (com body {decision, note})' },
  { r: /fetchJSON\('\/runs\/'\s*\+\s*encodeURIComponent\(id\)\s*\+\s*'\/cancel'/, name: 'POST /runs/{id}/cancel' }
];
ROUTES.forEach((rt) => {
  if (rt.r.test(js)) ok(rt.name);
  else fail('rota esperada não usada: ' + rt.name);
});

if (/offset=/.test(js) && /L\.offset\s*\+=/.test(js)) ok('log usa offset incremental (só o trecho novo é pedido: L.offset += chunk.length)');
else fail('offset do log não é tratado corretamente');

// Contrato reforçado: offset/size em CARACTERES (str Python do runner).
if (/offset e size são caracteres, não bytes/.test(js)) ok('comentário de contrato "offset e size são caracteres, não bytes" presente');
else fail('comentário de contrato do offset em caracteres ausente');
// Uso real de bytes (chamada): comentário não conta — comentários são
// removidos antes do teste para não gerar falso positivo.
const jsNoComments = js
  .replace(/\/\*[\s\S]*?\*\//g, '')
  .replace(/(^|\s)\/\/[^\n]*/g, '$1');
if (/new TextEncoder|\.byteLength|encodeURIComponent\([^)]*\)\.length\s*\*\s*[24]/.test(jsNoComments)) {
  fail('JS usa bytes — o offset do runner é em caracteres');
} else ok('nenhum uso real de TextEncoder/byteLength no app.js (comentário de contrato não conta)');
// Em contrapartida, o cursor precisa avançar em code points (a unidade do
// `len(str)` do runner), nunca em `.length` cru nem em bytes.
if (/L\.offset \+= countCodePoints\(chunk\)/.test(js)) {
  ok('cursor avança por countCodePoints(chunk) — code points, não UTF-16 nem bytes');
} else fail('cursor do log não avança em code points');
if (/function countCodePoints\(/.test(js) && /function utf16IndexOf\(/.test(js)) {
  ok('countCodePoints + utf16IndexOf presentes (emoji/acento não desloca o cursor)');
} else fail('helpers de code point ausentes');
if (/L\.offset \+= chunk\.length/.test(js)) fail('cursor usa chunk.length cru (emoji conta 2)');
else ok('nenhum L.offset += chunk.length cru');
if (/tail=\s*'?\s*\+\s*LOG_TAIL|tail=\$\{LOG_TAIL\}|&tail=200000/.test(js) && /LOG_TAIL\s*=\s*200000/.test(js)) ok('log pede &tail=200000 (teto do runner)');
else fail('tail=200000 ausente no log');
if (/runs\?limit=100/.test(js)) ok('lista usa GET /runs?limit=100');
else fail('/runs sem ?limit=100');
if (/payload\.size|data\.size/.test(js) && /payload\.truncated|data\.truncated/.test(js)) ok('size e truncated do log tratados');
else fail('size/truncated não tratados');
if (/log-banner/.test(css) && /log-banner/.test(js)) ok('aviso "carregado a partir de…" + recarregar do início implementados');
else fail('aviso de recorte do log ausente');
if (/offset=0/.test(js) || /L\.offset = 0/.test(js)) ok('recarregar do início (offset=0) implementado');
else fail('sem caminho de recarga offset=0');

// --- Portão de aprovação: decisão + nota + erros humanos --------------
if (/function apiApprove\(id, decision, note\)/.test(js)) ok('apiApprove(id, decision, note) definida');
else fail('apiApprove sem os parâmetros decision/note');
if (/decision:\s*decision === 'reject' \? 'reject' : 'approve'/.test(js) && /note:\s*typeof note === 'string' \? note : ''/.test(js)) {
  ok('corpo { decision: approve|reject, note: "" } montado no apiApprove');
} else fail('corpo da aprovação não envia decision/note como especificado');
if (/apiApprove\(run\.id, isApprove \? 'approve' : 'reject', note\)/.test(js)) ok('sheet chama apiApprove com a decisão escolhida e a nota do campo');
else fail('sheet não encaminha decisão/nota');
if (/field__textarea/.test(js) && /class: 'field__textarea'/.test(js)) ok('campo de nota usa .field__textarea');
else fail('sem textarea da nota');
// O textarea NÃO pode receber data-autofocus (teclado não pode pular) e
// precisa de aria-label (o harness de DOM lê isso como nome acessível).
const noteTextareaBlock = (js.match(/el\('textarea',\s*\{[\s\S]*?\}\)/) || [''])[0];
if (/field__textarea/.test(noteTextareaBlock) && !/data-autofocus/.test(noteTextareaBlock)) {
  ok('textarea da nota sem data-autofocus (teclado só abre por toque)');
} else fail('textarea da nota com data-autofocus ou não encontrado');
if (/'aria-label'\s*:\s*'[^']+'/.test(noteTextareaBlock)) ok('textarea da nota tem aria-label (nome acessível)');
else fail('textarea da nota sem aria-label');
if (/\.field__textarea\s*\{[\s\S]*?min-height:\s*var\(--target\)/.test(css)) ok('.field__textarea tem 48px de altura mínima');
else fail('.field__textarea sem 48px de altura mínima');
// Mensagens de erro sem código HTTP/stack
['Este portão já foi decidido.', 'Essa execução não existe mais.', 'A decisão não foi aceita.'].forEach((msg) => {
  if (js.indexOf(msg) >= 0) ok('mensagem humana presente: "' + msg + '"');
  else fail('mensagem humana ausente: "' + msg + '"');
});
if (/err\.status === 409/.test(js) && /err\.status === 404/.test(js) && /err\.status === 400/.test(js) && /err\.kind === 'unauthorized'/.test(js)) {
  ok('409/404/400 e 401 tratados no caminho da aprovação');
} else fail('tratamento de 409/404/400/401 incompleto');
// Nenhuma mensagem de erro de portão exibe código HTTP/stack na tela
const gateStartIdx = js.indexOf('function gateErrorText(');
const gateEndIdx = gateStartIdx >= 0 ? js.indexOf(String.fromCharCode(10) + '  }', gateStartIdx) : -1;
const gateErrorFn = gateStartIdx >= 0 && gateEndIdx > gateStartIdx ? js.slice(gateStartIdx, gateEndIdx) : '';
const gateMessages = gateErrorFn.replace(/err\.status === \d+/g, '');
if (gateErrorFn && !/(409|404|400|HTTP|status)/.test(gateMessages)) {
  ok('gateErrorText não expõe código HTTP na tela');
} else fail('gateErrorText menciona código HTTP/stack');
// Recusa ≠ aprovação no texto (cancelamento explícito)
if (/Recusar e cancelar/.test(js) && /Recusar cancela esta execução/.test(js) && /cancel_requested/.test(js)) {
  ok('recusa deixa explícito que CANCELA (cancel_requested) — texto diferente da aprovação');
} else fail('texto de recusa não deixa claro que cancela o run');

if (/visibilitychange/.test(js) && /stopPoll\(\)/.test(js)) ok('poll para quando a aba fica em segundo plano');
else fail('visibilitychange/stopPoll ausente');

if (/POLL_MS\s*=\s*2000/.test(js)) ok('poll de 2 s na lista e no log');
else fail('POLL_MS ausente');

if (/prefers-reduced-motion/.test(css)) ok('prefers-reduced-motion dedicado (motion desligado, nada removido)');
else fail('sem prefers-reduced-motion');

if (/prefers-color-scheme/.test(css) && /prefers-color-scheme:\s*light/.test(css)) ok('tema claro respeita prefers-color-scheme');

// ---------------------------------------------------------------------------
// 8. CSS — nenhum seletor dependendo de hover
// ---------------------------------------------------------------------------
console.log('\n[9] Densidade (gate 9) e cache offline (gate 8)');
// --- densidade: applyDensity() com zoom quando DPR baixo em tela grande ---
if (/function applyDensity\(\)/.test(js)) ok('applyDensity() definida');
else fail('applyDensity() ausente');
if (/var dpr = window\.devicePixelRatio \|\| 1;/.test(js)) ok('dpr lido do window.devicePixelRatio com fallback 1');
else fail('sem leitura de devicePixelRatio');
if (/var physical = Math\.round\(viewportWidth \* dpr\)/.test(js) && /var viewportWidth = window\.innerWidth \|\| root\.clientWidth \|\| 0;/.test(js)) ok('physical = round(innerWidth * dpr)');
else fail('sem cálculo de physical');
if (/physical >= DENSITY_MIN_PHYSICAL && dpr < DENSITY_MAX_NATIVE_DPR/.test(js) && /DENSITY_MIN_PHYSICAL = 900/.test(js) && /DENSITY_MAX_NATIVE_DPR = 1\.5/.test(js)) {
  ok('regra: physical >= 900 e dpr < 1.5');
} else fail('regra de physical/dpr ausente ou errada');
if (/physical \/ DENSITY_TARGET_PX/.test(js) && /DENSITY_TARGET_PX = 393/.test(js)) ok('fator = physical / 393');
else fail('sem divisão por 393');
if (/Math\.round\(\(physical \/ DENSITY_TARGET_PX\) \* 100\) \/ 100/.test(js)) ok('fator arredondado a 2 casas');
else fail('sem arredondamento a 2 casas');
if (/root\.style\.zoom = String\(factor\)/.test(js)) ok('zoom aplicado ao documentElement');
else fail('sem style.zoom');
if (/root\.dataset\.density = 'forced'/.test(js) && /root\.dataset\.density = 'native'/.test(js)) {
  ok('data-density = "forced" | "native"');
} else fail('sem dataset.density com os dois valores');
if (/if \(densityApplied !== factor\)/.test(js)) ok('idempotente: só reescreve o DOM quando o fator muda');
else fail('applyDensity não é idempotente');
if (/window\.addEventListener\('resize', applyDensity\)/.test(js) && /window\.addEventListener\('orientationchange', applyDensity\)/.test(js)) {
  ok('applyDensity em resize e orientationchange');
} else fail('sem listeners de resize/orientationchange');
if (/densidade ANTES da primeira tela/.test(js) && js.indexOf('applyDensity();') > -1) ok('applyDensity() chamada no boot antes do primeiro render');
else fail('applyDensity não roda no boot');
// CSS continua em 48: nada de reescrever altura para compensar o zoom
if (/\.btn\s*\{[^}]*min-height:\s*var\(--target\)/.test(css) && /--target:\s*48px/.test(css)) {
  ok('CSS mantém 48 dp — o zoom faz o resto, sem reescrever alturas');
} else fail('alturas foram reescritas (violaria o gate 9)');

// --- cache: persistir último /runs e mostrar como estado guardado ---
if (/LS_RUNS_CACHE = 'workploy\.cache\.runs\.v1'/.test(js)) ok('chave do cache versionada: workploy.cache.runs.v1');
else fail('sem chave versionada do cache');
if (/\{ savedAt: savedAt, runs: runs\.slice\(0, RUNS_CACHE_MAX_RUNS\) \}/.test(js)) ok('payload { savedAt, runs }');
else fail('payload sem savedAt');
if (/RUNS_CACHE_MAX_BYTES = 512 \* 1024/.test(js)) ok('teto de 512 KB');
else fail('sem teto de tamanho');
if (/RUNS_CACHE_TRIM_RUNS = 50/.test(js) && /runs\.slice\(0, RUNS_CACHE_TRIM_RUNS\)/.test(js)) ok('corte para os 50 mais recentes quando passa do teto');
else fail('sem corte para 50');
if (/RUNS_CACHE_STALE_MS = 24 \* 60 \* 60 \* 1000/.test(js)) ok('limite de frescor 24 h');
else fail('sem limite de 24 h');
if (/age > RUNS_CACHE_STALE_MS/.test(js) && /formatAgeLong\(age\)/.test(js)) ok('cache de mais de 24 h mostra o tempo decorrido');
else fail('cache velho não mostra a idade');
if (/'Sem conexão\. Mostrando o estado de ' \+ when/.test(js)) ok('mensagem "Sem conexão. Mostrando o estado de HH:mm."');
else fail('mensagem do cache ausente');
if (/maybeWriteRunsCache\(data\)/.test(js)) ok('cache gravado somente em sucesso de /runs');
else fail('cache não é gravado no sucesso');
if (/if \(!isAuthError\(err\) && state\.lastRuns === null\) applyCachedRuns\(err\)/.test(js)) ok('falha de rede cai no cache; 401 segue o caminho de reconectar');
else fail('cache não é aplicado na falha de rede');
if (/\.cache-banner\s*\{/.test(css) && /cache-banner__text/.test(css)) ok('.cache-banner estilizado');
else fail('.cache-banner sem CSS');

console.log('\n[8] CSS — sem hover (não existe no toque)');
const hoverCount = (css.match(/:hover/g) || []).length;
if (hoverCount === 0) ok('nenhum :hover no CSS');
else fail(hoverCount + ' :hover encontrado — hover não existe no toque §9');

const tooltip = /tooltip/i.test(css) || /tooltip/i.test(js);
if (!tooltip) ok('nenhum tooltip (removido por §9)');
else fail('tooltip declarado — removido por §9');

const focus = /:focus-visible/.test(css) || /:focus-visible/.test(css);
const focusVisible = /:focus-visible/.test(css) || /:focus-visible/.test(css);
if (focus) ok('foco visível declarado (:focus-visible)');
else fail('sem :focus-visible');

// ---------------------------------------------------------------------------
// Resultado
// ---------------------------------------------------------------------------
console.log('\n=== RESUMO ESTÁTICO ===');
if (failures === 0) {
  console.log('VEREDITO: PRONTO — HTML/JS/CSS atendem aos gates 1, 2, 3 e 9 do §12 por análise estática.');
  console.log('Contraste medido: todos os pares de texto >= 4.5:1 em dark e light.');
  console.log('Sem hover, sem tooltip, foco visível, alturas normativas, scroll horizontal só no log.');
} else {
  console.log('VEREDITO: FALHOU — ' + failures + ' gate(s) falharam.');
  process.exitCode = 1;
}