// ============================================================================================
// IDIOMAS (castellano · català · English) — capa de traducción de la interfaz en el navegador.
//
// La interfaz se escribe en CASTELLANO (idioma fuente) en Rust (report.rs), en el HTML del juego y
// en el JS. En vez de reescribir miles de textos con claves, esta capa traduce el DOM ya pintado:
//   * nodos de texto y atributos (title, aria-label, placeholder, alt) cuyo texto EXACTO está en
//     el diccionario (`window.I18N_DICT`, fichero i18n_dict.js: "texto es" → ["ca", "en"]);
//   * los números dentro de un texto son COMODINES: la clave se busca con cada número cambiado por
//     {n} y luego se reponen en orden («35 incidencias» y «2 incidencias» = una sola entrada);
//   * un MutationObserver traduce lo que el JS pinta después (paneles, modales, gráficos).
// En castellano no hace nada (coste cero). En ca/en, cada nodo que cambia cuesta una búsqueda en
// un Map. Los nombres propios (estaciones, motivos de incidencia del usuario) no están en el
// diccionario y quedan tal cual.
//
// El idioma se guarda en localStorage "rod-lang" (compartido por el menú, las subpáginas y el
// simulador visual); se elige en Configuración (⚙). LOCALE() da el locale para formatear números.
// ============================================================================================
(function () {
  "use strict";
  const LANGS = { es: { idx: -1, locale: "es-ES" }, ca: { idx: 0, locale: "ca-ES" }, en: { idx: 1, locale: "en-GB" } };
  let lang = "es";
  try { const v = localStorage.getItem("rod-lang"); if (LANGS[v]) lang = v; } catch (e) { /* sin storage */ }

  const NUM = /\d+(?:[.,]\d+)*/g;
  const LETTER = /[A-Za-zÀ-ÿ]/;
  // MathML: números (mn), variables (mi: la "y" de una ecuación NO es la conjunción) y operadores (mo)
  // no se traducen; el texto recto (mtext: unidades, "previa"…) sí.
  const SKIP = new Set(["SCRIPT", "STYLE", "CODE", "TEXTAREA", "NOSCRIPT", "MN", "MI", "MO"]);
  const ATTRS = ["title", "aria-label", "placeholder", "alt"];
  let dict = null;          // Map clave → texto traducido (solo el idioma activo)
  const missing = new Set(); // claves sin traducción vistas (depuración: I18N.missing)
  const done = new WeakMap(); // nodo → último valor traducido (no retraducir lo propio)

  let pats = [];            // [regex, reemplazo] del idioma activo (textos con contenido propio)
  let outs = new Set();     // textos YA traducidos: el observador los ve cambiar y no hay que retraducirlos
  function build() {
    dict = new Map();
    const src = window.I18N_DICT || {};
    const i = LANGS[lang].idx;
    for (const k in src) { const v = src[k][i]; if (v != null) { dict.set(k, v); outs.add(v); } }
    pats = (window.I18N_PATTERNS || []).filter((p) => p[1 + i] != null).map((p) => [p[0], p[1 + i]]);
  }

  // Traduce una cadena conservando espacios de los extremos y números. null = sin cambios.
  function tr(s) {
    if (!s || !LETTER.test(s)) return null;
    const m = /^(\s*)([\s\S]*?)(\s*)$/.exec(s);
    const core = m[2].replace(/ /g, " "); // &nbsp; del HTML = espacio normal para la clave
    const nums = core.match(NUM) || [];
    const key = nums.length ? core.replace(NUM, "{n}") : core;
    let t = dict.get(key);
    if (t == null && outs.has(key)) return null; // ya está en el idioma destino
    if (t == null) {
      // Textos que mezclan contenido propio (motivo, tren, estaciones) con texto de la interfaz.
      for (const [re, rep] of pats) if (re.test(core)) return m[1] + core.replace(re, rep) + m[3];
      if (core.length < 300) missing.add(key);
      return null;
    }
    if (nums.length) { let k = 0; t = t.replace(/\{n\}/g, () => (k < nums.length ? nums[k++] : "")); }
    return m[1] + t + m[3];
  }

  function skipEl(el) {
    for (let e = el; e && e.nodeType === 1; e = e.parentElement) {
      if (SKIP.has(e.tagName.toUpperCase()) || (e.hasAttribute && e.hasAttribute("data-no-i18n"))) return true;
    }
    return false;
  }
  function textNode(n) {
    if (done.get(n) === n.nodeValue) return;
    if (skipEl(n.parentElement)) return;
    const t = tr(n.nodeValue);
    if (t != null && t !== n.nodeValue) { n.nodeValue = t; }
    done.set(n, n.nodeValue);
  }
  function attrs(el) {
    for (const a of ATTRS) {
      const v = el.getAttribute(a);
      if (!v) continue;
      const t = tr(v);
      if (t != null && t !== v) el.setAttribute(a, t);
    }
  }
  function walk(root) {
    if (root.nodeType === 3) { textNode(root); return; }
    if (root.nodeType !== 1 || skipEl(root)) return;
    attrs(root);
    const it = document.createTreeWalker(root, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_ELEMENT);
    for (let n = it.nextNode(); n; n = it.nextNode()) {
      if (n.nodeType === 3) textNode(n);
      else if (!SKIP.has(n.tagName.toUpperCase())) attrs(n);
    }
  }

  function start() {
    document.documentElement.lang = lang;
    if (lang === "es") return;
    build();
    const t = tr(document.title); if (t) document.title = t;
    walk(document.body);
    new MutationObserver((muts) => {
      for (const m of muts) {
        if (m.type === "characterData") textNode(m.target);
        else if (m.type === "attributes") { if (m.target.nodeType === 1 && !skipEl(m.target)) attrs(m.target); }
        else for (const n of m.addedNodes) walk(n);
      }
    }).observe(document.body, { childList: true, subtree: true, characterData: true, attributes: true, attributeFilter: ATTRS });
  }
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start);
  else start();

  window.LOCALE = () => LANGS[lang].locale;
  window.I18N = {
    get lang() { return lang; },
    // Cambia de idioma y recarga (la traducción es desde el castellano: volver atrás = repintar).
    set(l) { if (!LANGS[l]) return; try { localStorage.setItem("rod-lang", l); } catch (e) {} location.reload(); },
    // Traducción puntual desde JS (p. ej. textos de confirm()/alert()).
    t(s) { if (lang === "es") return s; if (!dict) build(); return tr(s) ?? s; },
    missing,
  };
})();
