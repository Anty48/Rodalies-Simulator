// ============================================================================================
// CREADOR DE HORARIOS CADENCIADOS (/horarios).
//
// Relojes por estación de referencia (Sants, Cerdanyola Universitat, La Garriga, Tarragona,
// Lleida), uno por SENTIDO en cada conjunto de vías principales. Se arrastra el logo de una línea
// al minuto en que su tren debe pasar por la referencia; el marcador se repite cada hora (o cada N
// horas) desde la apertura del sistema (04:30). Al pulsar un marcador se elige el RECORRIDO entero
// (origen → destino ligados, sacados del GTFS), la frecuencia y una variante opcional cada N horas.
//
// El catálogo (recorridos, sentidos, tiempos de cabecera a referencia) lo da el servidor
// (`/api/cadence/catalog`, src/cadence.rs) y la malla se genera allí (`/api/cadence/load`): este
// fichero solo edita la especificación y la guarda en el navegador.
//
// Formato guardado (lo leen también el simulador visual y Simulaciones encadenadas rápidas):
//   localStorage "rod-cadences" = [{ id, name, updated, spec: { name,
//     markers: [{ line, pat, min, every, alt: { pat, every } | null }],
//     groups:  [{ name, ref, g, d, items: [pat…], mins: [min…], every }] } }]
// Exportado a fichero: { format: "rodalies-cadence", version: 1, name, markers, groups }.
// GRUPOS: una secuencia de recorridos de un mismo reloj que se reparte, rotando, los minutos
// marcados (regionales de França cada media hora, cada vez de una línea distinta).
// ============================================================================================
(function () {
  "use strict";
  const $ = (id) => document.getElementById(id);
  const KEY = "rod-cadences";
  const T = (s) => (window.I18N ? I18N.t(s) : s);
  const esc = (x) => String(x).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
  const hhmm = (s) => { s = ((Math.round(s) % 86400) + 86400) % 86400; return String(Math.floor(s / 3600)).padStart(2, "0") + ":" + String(Math.floor(s % 3600 / 60)).padStart(2, "0"); };
  const mm = (m) => ":" + String(m).padStart(2, "0");

  // Logos oficiales de cada línea (mismo mapa que app.js).
  const LINE_IMG = {
    R1: "R1_barcelona.svg.webp", R2: "R2_barcelona.svg.webp", R2N: "R2_Nord_Rodalies.png",
    R2S: "R2_Sud_Rodalies.png", R3: "R3.jpg", R3a: "R3.jpg", R4: "R4.jpg", R7: "R7.png",
    R8: "R8_barcelona.svg.webp", R11: "R11.jpg", R13: "R13_Rodalies_de_Catalunya.svg.png",
    R14: "R14.jpg", R15: "R15_Rodalies_de_Catalunya.svg.png", R16: "R16_Rodalies_de_Catalunya.svg.png",
    R17: "R17_Rodalies_de_Catalunya.svg.webp", RG1: "RG1_rodalies.png", RL3: "RL3.png",
    RL4: "RL4_Rodalies_de_Catalunya.svg.png", RT1: "RT1_tarragona.svg.webp", RT2: "RT2_tarragona.svg.webp",
  };
  const logoUrl = (l) => (LINE_IMG[l] ? "/game/lines/" + LINE_IMG[l] : null);
  const logo = (l, color) => logoUrl(l) ? `<img src="${logoUrl(l)}" alt="${esc(l)}">` : `<span class="sw" style="background:${color || "#888"}"></span>`;
  // Orden de líneas: R, RG, RL, RT y por número (como la leyenda del simulador).
  function lineKey(name) {
    const m = /^(R[GLT]?)(\d*)([NS]?)/i.exec(name) || [];
    return [{ R: 0, RG: 1, RL: 2, RT: 3 }[(m[1] || "R").toUpperCase()] ?? 9, parseInt(m[2] || "0", 10) || 0, m[3] || ""];
  }
  const byLine = (a, b) => { const x = lineKey(a), y = lineKey(b); return x[0] - y[0] || x[1] - y[1] || x[2].localeCompare(y[2]); };

  // Frecuencias ofrecidas (h). Regionales: por defecto cada 2 h (pasan menos de una vez por hora).
  const EVERY = [1, 2, 3, 4, 6];
  const ALT_EVERY = [2, 3, 4, 6];

  // ---- Estado ----------------------------------------------------------------------------------
  let CAT = null;                   // catálogo del servidor
  const PAT = new Map();            // id de recorrido → patrón
  let doc = nuevoDoc();             // horario en edición
  let dirty = false;
  let tab = null;                   // referencia visible
  let sel = null;                   // uid del marcador abierto en el editor
  let selG = null;                  // uid del grupo abierto en su editor
  let uidSeq = 1;

  function nuevoDoc() { return { id: null, name: "", markers: [], groups: [] }; }
  function lista() { try { const v = JSON.parse(localStorage.getItem(KEY)); return Array.isArray(v) ? v : []; } catch (e) { return []; } }
  function guardarLista(v) { try { localStorage.setItem(KEY, JSON.stringify(v)); return true; } catch (e) { status(T("No se pudo guardar en este navegador (almacenamiento bloqueado)."), "err"); return false; } }
  // Marcador limpio para guardar/enviar (sin el uid interno).
  const limpio = (m) => ({ line: m.line, pat: m.pat, min: m.min, every: m.every, alt: m.alt ? { pat: m.alt.pat, every: m.alt.every } : null });
  const limpioG = (g) => ({ name: g.name, ref: g.ref, g: g.g, d: g.d, items: g.items.slice(), mins: g.mins.slice(), every: g.every });
  function conUidG(groups) {
    return (groups || []).filter((g) => g && Array.isArray(g.items) && Array.isArray(g.mins)).map((g) => ({
      uid: uidSeq++, name: String(g.name || "Grupo"), ref: g.ref, g: +g.g || 0, d: +g.d || 0,
      items: g.items.filter((x) => typeof x === "string"),
      mins: [...new Set(g.mins.map((m) => ((Math.round(+m) % 60) + 60) % 60).filter(Number.isFinite))].sort((a, b) => a - b),
      every: Math.max(1, Math.round(+g.every || 1)),
    }));
  }
  // Cuerpo que entiende el servidor (/api/cadence/load) y que se guarda/exporta.
  const especificacion = () => ({ name: doc.name, markers: doc.markers.map(limpio), groups: doc.groups.map(limpioG) });
  const vacio = () => !doc.markers.length && !doc.groups.some((g) => g.items.length && g.mins.length);
  function conUid(markers) {
    return (markers || []).filter((m) => m && typeof m.pat === "string" && Number.isFinite(+m.min)).map((m) => ({
      uid: uidSeq++, line: m.line || (m.pat.split("|")[0]), pat: m.pat,
      min: ((Math.round(+m.min) % 60) + 60) % 60, every: Math.max(1, Math.round(+m.every || 1)),
      alt: m.alt && typeof m.alt.pat === "string" ? { pat: m.alt.pat, every: Math.max(1, Math.round(+m.alt.every || 2)) } : null,
    }));
  }

  function status(txt, cls) { const el = $("cadStatus"); el.textContent = txt || ""; el.className = "cad-status" + (cls ? " " + cls : ""); }
  function marcarCambio() { dirty = true; programarPrevia(); }

  // ---- Cálculo de pasos (igual que src/cadence.rs::slots, para la vista del editor) -------------
  function slots(min, every, off) {
    let t = min * 60; const out = [];
    while (t < CAT.open + off) t += 3600;
    while (t - off <= CAT.last_dep) { out.push(t); t += Math.max(1, every) * 3600; }
    return out;
  }

  // Marcadores que se pintan en el reloj (ref, grupo, sentido): editables + calculados (de otra referencia).
  function marcadoresDe(ref, g, d) {
    const edit = [], ghost = [];
    for (const m of doc.markers) {
      const p = PAT.get(m.pat); if (!p) continue;
      if (p.ref_key === ref && p.group === g && p.dir === d) edit.push({ m, p, min: m.min });
      // El mismo tren visto desde otra referencia por la que pasa: minuto calculado, sin editar.
      for (const q of [p, m.alt && PAT.get(m.alt.pat)].filter(Boolean)) {
        for (const a of q.also || []) {
          if (a.ref_key !== ref || a.group !== g || a.dir !== d) continue;
          const sec = m.min * 60 + a.off - q.ref_off;
          ghost.push({ m, p: q, min: Math.round((((sec % 3600) + 3600) % 3600) / 60) % 60, from: q.ref_key });
        }
      }
    }
    // Pasos de los GRUPOS de este reloj (editables) y, de cualquier grupo, los trenes que pasan
    // también por esta referencia (calculados: un fantasma por línea y minuto distintos).
    const gmk = [];
    for (const gr of doc.groups) {
      if (gr.ref === ref && gr.g === g && gr.d === d) gr.mins.forEach((min, mi) => gmk.push({ gr, min, mi }));
      const vistos = new Set();
      for (const { t, p } of expandirGrupo(gr)) {
        for (const a of p.also || []) {
          if (a.ref_key !== ref || a.group !== g || a.dir !== d) continue;
          const sec = t + a.off - p.ref_off, min = Math.round((((sec % 3600) + 3600) % 3600) / 60) % 60;
          if (vistos.has(p.id + min)) continue;
          vistos.add(p.id + min);
          ghost.push({ m: null, p, min, from: gr.ref, grp: gr.name });
        }
      }
    }
    return { edit, ghost, gmk };
  }
  const refName = (k) => { const r = CAT.refs.find((x) => x.key === k); return r ? r.name : k; };

  // ---- Render ----------------------------------------------------------------------------------
  function render() { pintarGuardados(); pintarTabs(); pintarCuerpo(); }

  function pintarGuardados() {
    const s = $("cadSaved"), l = lista();
    s.innerHTML = `<option value="">${esc(T("— Horario nuevo (sin guardar) —"))}</option>` +
      l.map((c) => `<option value="${esc(c.id)}"${c.id === doc.id ? " selected" : ""}>${esc(c.name || T("Sin nombre"))}</option>`).join("");
    $("cadName").value = doc.name;
    $("cadDelete").disabled = !doc.id;
  }

  function pintarTabs() {
    const n = (k) => doc.markers.filter((m) => { const p = PAT.get(m.pat); return p && p.ref_key === k; }).length + doc.groups.filter((g) => g.ref === k).length;
    $("cadTabs").innerHTML = CAT.refs.map((r) =>
      `<button type="button" role="tab" data-k="${r.key}" class="${r.key === tab ? "active" : ""}" aria-selected="${r.key === tab}">${esc(r.name)}${n(r.key) ? `<span class="cnt">${n(r.key)}</span>` : ""}</button>`).join("");
    $("cadTabs").querySelectorAll("button").forEach((b) => b.onclick = () => { tab = b.dataset.k; sel = null; selG = null; pintarTabs(); pintarCuerpo(); });
  }

  function pintarCuerpo() {
    const r = CAT.refs.find((x) => x.key === tab); if (!r) { $("cadBody").innerHTML = ""; return; }
    $("cadBody").innerHTML = r.groups.map((g, gi) => `<section class="cad-group"><h3>${esc(g.label)}</h3><div class="cad-pair">` +
      g.dirs.map((d, di) => `<div class="clock-card" id="card-${gi}-${di}" data-g="${gi}" data-d="${di}"></div>`).join("") + `</div></section>`).join("");
    r.groups.forEach((g, gi) => g.dirs.forEach((d, di) => pintarTarjeta(gi, di)));
  }

  // Recorridos editables de una línea en un reloj, del más habitual al menos.
  const recorridos = (line, ref, g, d) => CAT.patterns.filter((p) => p.line === line && p.ref_key === ref && p.group === g && p.dir === d).sort((a, b) => b.n - a.n);
  const nombreRec = (p) => `${p.from_name} → ${p.to_name}`;

  // Rótulo del reloj: «<referencia> dirección <siguiente parada>» (o llegadas si acaban allí).
  function tituloReloj(r, dir) {
    return dir.terminal
      ? `${esc(r.name)} <span class="dir">· ${esc(T("llegadas (final de recorrido)"))}</span>`
      : `${esc(r.name)} <span class="dir">${esc(T("dirección"))}</span> ${esc(dir.label)}`;
  }
  // Abreviatura de un grupo para el marcador del reloj (3 letras).
  const sigla = (gr) => (gr.name || "G").replace(/[^\p{L}\p{N}]/gu, "").slice(0, 3).toUpperCase() || "G";
  const COLOR_GRUPO = "#830065";   // morado Renfe (acento secundario Andana) = marcador de grupo
  const gruposDe = (ref, g, d) => doc.groups.filter((x) => x.ref === ref && x.g === g && x.d === d);
  const secuenciaTxt = (gr) => gr.items.map((p) => (PAT.get(p) || { line: "?" }).line).join(" → ") || T("secuencia vacía");

  function pintarTarjeta(g, d) {
    const card = $(`card-${g}-${d}`); if (!card) return;
    const r = CAT.refs.find((x) => x.key === tab);
    const lines = [...new Set(CAT.patterns.filter((p) => p.ref_key === tab && p.group === g && p.dir === d).map((p) => p.line))].sort(byLine);
    const { edit, ghost, gmk } = marcadoresDe(tab, g, d);
    const items = [...edit.map((x) => ({ ...x, kind: "m" })), ...gmk.map((x) => ({ ...x, kind: "g" })), ...ghost.map((x) => ({ ...x, kind: "ghost", ghost: true }))].sort((a, b) => a.min - b.min);
    const color = (l) => (CAT.patterns.find((p) => p.line === l) || {}).color || "#888";
    const grupos = gruposDe(tab, g, d);
    const fila = (x) => {
      if (x.kind === "g") {
        return `<li class="grp${x.gr.uid === selG ? " sel" : ""}" data-gid="${x.gr.uid}" title="${esc(secuenciaTxt(x.gr))}">
          <span class="gtag" style="background:${COLOR_GRUPO}">${esc(sigla(x.gr))}</span><span class="t">${mm(x.min)}</span><span class="d">«${esc(x.gr.name)}» · ${esc(secuenciaTxt(x.gr))}</span></li>`;
      }
      const det = x.ghost ? T("calculado desde") + " " + refName(x.from) + (x.grp ? " («" + x.grp + "»)" : "")
        : (T("cada") + " " + x.m.every + " h" + (x.m.alt ? " · " + T("variante") : ""));
      return `<li class="${x.ghost ? "ghost" : ""}${!x.ghost && x.m.uid === sel ? " sel" : ""}" ${x.ghost ? "" : `data-uid="${x.m.uid}"`} title="${esc(nombreRec(x.p))}">
        ${logo(x.p.line, x.p.color)}<span class="t">${mm(x.min)}</span><span class="d">→ ${esc(x.p.to_name)} · ${esc(det)}</span></li>`;
    };
    card.innerHTML = `<h4>${tituloReloj(r, r.groups[g].dirs[d])}</h4>
      <div class="palette">${lines.map((l) => `<span class="pal-chip" data-line="${esc(l)}" title="${esc(T("Arrastra al reloj"))}">${logo(l, color(l))}${esc(l)}</span>`).join("")}
        ${grupos.map((gr) => `<span class="pal-chip grp" data-gid="${gr.uid}" title="${esc(T("Arrastra al reloj para añadir un paso del grupo"))}: ${esc(secuenciaTxt(gr))}"><span class="gtag" style="background:${COLOR_GRUPO}">${esc(sigla(gr))}</span>${esc(gr.name)}</span>`).join("")}
        ${lines.length ? `<button type="button" class="pal-add" title="${esc(T("Agrupa varias líneas en una secuencia que se reparte los minutos marcados"))}">+ ${esc(T("Grupo"))}</button>`
          : `<span class="pal-empty">${esc(T("Sin líneas que se editen aquí"))}</span>`}</div>
      <div class="clock-row">${relojSvg(items, g, d)}
        <ul class="mk-list">${items.length ? items.map(fila).join("") : `<li class="mk-empty">${esc(T("Arrastra una línea al reloj para que pase a ese minuto."))}</li>`}</ul>
      </div>`;
    const ed = edit.find((x) => x.m.uid === sel);
    if (ed) card.appendChild(editor(ed.m, g, d));
    const eg = grupos.find((x) => x.uid === selG);
    if (eg) card.appendChild(editorGrupo(eg, g, d));
    card.querySelectorAll(".pal-chip[data-line]").forEach((c) => c.addEventListener("pointerdown", (e) => arrastrarPaleta(e, { line: c.dataset.line }, g, d)));
    card.querySelectorAll(".pal-chip[data-gid]").forEach((c) => c.addEventListener("pointerdown", (e) => arrastrarPaleta(e, { gid: +c.dataset.gid }, g, d)));
    const add = card.querySelector(".pal-add");
    if (add) add.onclick = () => nuevoGrupo(g, d);
    card.querySelectorAll(".mk-list li[data-uid]").forEach((li) => li.onclick = () => { sel = +li.dataset.uid; selG = null; pintarTarjetas(); });
    card.querySelectorAll(".mk-list li[data-gid]").forEach((li) => li.onclick = () => { selG = +li.dataset.gid; sel = null; pintarTarjetas(); });
    card.querySelectorAll(".clock-svg .mk[data-uid], .clock-svg .mk[data-gid]").forEach((el) => el.addEventListener("pointerdown", (e) => arrastrarMarcador(e, el)));
  }
  // Repinta todas las tarjetas de la pestaña (al abrir/cerrar el editor en otra).
  function pintarTarjetas() { const r = CAT.refs.find((x) => x.key === tab); r.groups.forEach((gr, gi) => gr.dirs.forEach((_, di) => pintarTarjeta(gi, di))); }

  // ---- Reloj SVG -------------------------------------------------------------------------------
  const C = 125, R = 112, LANES = [74, 52, 30];
  const polar = (min, r) => { const a = (min / 60) * 2 * Math.PI - Math.PI / 2; return [C + r * Math.cos(a), C + r * Math.sin(a)]; };
  // Atributos que identifican un marcador editable (suelto: uid; paso de grupo: gid + índice).
  const idAttrs = (x) => x.kind === "m" ? `data-uid="${x.m.uid}"` : x.kind === "g" ? `data-gid="${x.gr.uid}" data-mi="${x.mi}"` : "";
  const handKey = (x) => x.kind === "m" ? "m" + x.m.uid : x.kind === "g" ? "g" + x.gr.uid + "-" + x.mi : "";
  function relojSvg(items, g, d) {
    let s = `<svg class="clock-svg" viewBox="0 0 250 250" data-g="${g}" data-d="${d}" role="img" aria-label="${esc(T("Reloj de pasos"))}">`;
    s += `<circle class="face" cx="${C}" cy="${C}" r="${R}"/><circle class="drop" cx="${C}" cy="${C}" r="${R}"/>`;
    for (let m = 0; m < 60; m++) {
      const big = m % 5 === 0, [x1, y1] = polar(m, R - (big ? 12 : 6)), [x2, y2] = polar(m, R - 2);
      s += `<line class="tick${big ? " h" : ""}" x1="${x1.toFixed(1)}" y1="${y1.toFixed(1)}" x2="${x2.toFixed(1)}" y2="${y2.toFixed(1)}"/>`;
      if (big) { const [lx, ly] = polar(m, R - 21); s += `<text class="lbl" x="${lx.toFixed(1)}" y="${ly.toFixed(1)}">${String(m).padStart(2, "0")}</text>`; }
    }
    // Carriles: marcadores cercanos (±3 min) se apilan hacia el centro para no taparse.
    const placed = [];
    for (const x of items) {
      let lane = 0;
      while (lane < LANES.length - 1 && placed.some((p) => p.lane === lane && Math.min(Math.abs(p.min - x.min), 60 - Math.abs(p.min - x.min)) < 4)) lane++;
      placed.push({ ...x, lane });
    }
    for (const x of placed) {
      const [hx, hy] = polar(x.min, LANES[x.lane] - 13);
      const col = x.kind === "g" ? COLOR_GRUPO : x.p.color;
      s += `<line class="hand" x1="${C}" y1="${C}" x2="${hx.toFixed(1)}" y2="${hy.toFixed(1)}" stroke="${esc(col)}"${x.ghost ? ' stroke-dasharray="3 3"' : ""} data-hand="${handKey(x)}"/>`;
    }
    for (const x of placed) s += marcadorSvg(x, LANES[x.lane]);
    s += `<circle class="hub" cx="${C}" cy="${C}" r="4"/></svg>`;
    return s;
  }
  function marcadorSvg(x, r) {
    const [mx, my] = polar(x.min, r);
    const pos = `data-r="${r}" data-hk="${handKey(x)}" transform="translate(${mx.toFixed(1)},${my.toFixed(1)})"`;
    if (x.kind === "g") {
      return `<g class="mk grp${x.gr.uid === selG ? " sel" : ""}" ${idAttrs(x)} ${pos}><title>${esc(`«${x.gr.name}» ${mm(x.min)} · ${secuenciaTxt(x.gr)}`)}</title>
        <circle r="13" fill="${COLOR_GRUPO}"/><text>${esc(sigla(x.gr))}</text></g>`;
    }
    const u = logoUrl(x.p.line);
    const tip = `${x.p.line} ${mm(x.min)} · ${nombreRec(x.p)}` + (x.ghost ? ` (${T("calculado desde")} ${refName(x.from)})` : "");
    return `<g class="mk${x.ghost ? " ghost" : ""}${x.kind === "m" && x.m.uid === sel ? " sel" : ""}" ${idAttrs(x)} ${pos}>
      <title>${esc(tip)}</title><circle r="13" fill="${esc(x.p.color)}"/>` +
      (u ? `<image href="${u}" x="-9.5" y="-9.5" width="19" height="19" preserveAspectRatio="xMidYMid meet"/>` : `<text>${esc(x.p.line)}</text>`) + `</g>`;
  }
  // Minuto bajo el puntero en un reloj (ángulo desde las 12, en sentido horario).
  function minutoEn(svg, e) {
    const b = svg.getBoundingClientRect();
    const x = (e.clientX - b.left) / b.width * 250 - C, y = (e.clientY - b.top) / b.height * 250 - C;
    let a = Math.atan2(y, x) + Math.PI / 2; if (a < 0) a += 2 * Math.PI;
    return Math.round(a / (2 * Math.PI) * 60) % 60;
  }

  // ---- Arrastrar desde la paleta (línea o grupo → minuto del reloj) ---------------------------
  function arrastrarPaleta(e, what, g, d) {
    e.preventDefault();
    const x0 = e.clientX, y0 = e.clientY;
    const gr = what.gid != null ? doc.groups.find((x) => x.uid === what.gid) : null;
    let moved = false, ghost = null, over = null;
    const target = () => document.querySelector(`#card-${g}-${d} .clock-svg`);
    const move = (ev) => {
      if (!moved && Math.hypot(ev.clientX - x0, ev.clientY - y0) < 5) return;
      if (!moved) {
        moved = true;
        ghost = document.createElement("div"); ghost.className = "drag-ghost";
        ghost.innerHTML = gr ? `<span class="gtag big" style="background:${COLOR_GRUPO}">${esc(sigla(gr))}</span>`
          : logoUrl(what.line) ? `<img src="${logoUrl(what.line)}" alt="">` : `<b>${esc(what.line)}</b>`;
        document.body.appendChild(ghost);
      }
      ghost.style.left = ev.clientX + "px"; ghost.style.top = ev.clientY + "px";
      const el = document.elementFromPoint(ev.clientX, ev.clientY);
      const svg = el && el.closest && el.closest(".clock-svg");
      over = svg === target() ? svg : null;
      const t = target(); if (t) t.classList.toggle("over", !!over);
    };
    const up = (ev) => {
      window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", up); window.removeEventListener("pointercancel", up);
      if (ghost) ghost.remove();
      const t = target(); if (t) t.classList.remove("over");
      if (moved && !over) return;
      if (gr) {
        // Grupo: soltar = añadir ese minuto a sus pasos; un clic abre su editor.
        if (moved) { const m = minutoEn(over, ev); if (!gr.mins.includes(m)) { gr.mins.push(m); gr.mins.sort((a, b) => a - b); marcarCambio(); } }
        selG = gr.uid; sel = null; pintarTabs(); pintarTarjetas();
        return;
      }
      // Línea: soltada en su reloj → minuto bajo el puntero; un clic sin arrastrar la pone a :00.
      añadirMarcador(what.line, g, d, moved ? minutoEn(over, ev) : 0);
    };
    window.addEventListener("pointermove", move); window.addEventListener("pointerup", up); window.addEventListener("pointercancel", up);
  }
  function añadirMarcador(line, g, d, min) {
    const ps = recorridos(line, tab, g, d); if (!ps.length) return;
    const p = ps[0];
    const m = { uid: uidSeq++, line, pat: p.id, min, every: p.regional ? 2 : 1, alt: null };
    doc.markers.push(m); sel = m.uid; selG = null;
    marcarCambio(); pintarTabs(); pintarTarjetas();
  }

  // ---- Arrastrar un marcador (suelto o paso de grupo) por la esfera: cambia su minuto ---------
  function arrastrarMarcador(e, el) {
    e.preventDefault(); e.stopPropagation();
    const svg = el.ownerSVGElement, r = +el.dataset.r;
    const isG = el.dataset.gid != null;
    const m = isG ? null : doc.markers.find((x) => x.uid === +el.dataset.uid);
    const gr = isG ? doc.groups.find((x) => x.uid === +el.dataset.gid) : null;
    const mi = isG ? +el.dataset.mi : 0;
    if (!m && !gr) return;
    const get = () => (gr ? gr.mins[mi] : m.min), put = (v) => { if (gr) gr.mins[mi] = v; else m.min = v; };
    const hand = svg.querySelector(`[data-hand="${el.dataset.hk}"]`);
    const x0 = e.clientX, y0 = e.clientY, min0 = get();
    let moved = false;
    const move = (ev) => {
      if (!moved && Math.hypot(ev.clientX - x0, ev.clientY - y0) < 4) return;
      moved = true;
      const v = minutoEn(svg, ev); put(v);
      const [mx, my] = polar(v, r); el.setAttribute("transform", `translate(${mx.toFixed(1)},${my.toFixed(1)})`);
      if (hand) { const [hx, hy] = polar(v, r - 13); hand.setAttribute("x2", hx.toFixed(1)); hand.setAttribute("y2", hy.toFixed(1)); }
    };
    const up = () => {
      window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", up); window.removeEventListener("pointercancel", up);
      if (gr) { gr.mins = [...new Set(gr.mins)].sort((a, b) => a - b); selG = gr.uid; sel = null; } else { sel = m.uid; selG = null; }
      if (moved && get() !== min0) marcarCambio();
      pintarTarjetas();
    };
    window.addEventListener("pointermove", move); window.addEventListener("pointerup", up); window.addEventListener("pointercancel", up);
  }

  // ---- GRUPOS de líneas: una secuencia que se reparte los minutos marcados -------------------
  // Igual que src/cadence.rs::group_slots: pasos = todos los minutos de cada ciclo, en orden; la
  // rotación empieza en el primer paso cuyo 1er recorrido no sale antes de la apertura.
  function expandirGrupo(gr) {
    const ps = gr.items.map((id) => PAT.get(id)).filter(Boolean);
    if (!ps.length || !gr.mins.length) return [];
    const cycle = Math.max(1, gr.every) * 3600, times = [];
    for (let base = 0; base < 30 * 3600; base += cycle) for (const m of gr.mins) times.push(base + m * 60);
    const ts = [...new Set(times)].sort((a, b) => a - b);
    const k0 = ts.findIndex((t) => t >= CAT.open + ps[0].ref_off);
    if (k0 < 0) return [];
    const out = [];
    ts.slice(k0).forEach((t, k) => { const p = ps[k % ps.length]; if (t >= CAT.open + p.ref_off && t - p.ref_off <= CAT.last_dep) out.push({ t, p }); });
    return out;
  }
  function nuevoGrupo(g, d) {
    const n = doc.groups.filter((x) => x.ref === tab).length + 1;
    const gr = { uid: uidSeq++, name: T("Grupo") + " " + n, ref: tab, g, d, items: [], mins: [], every: 1 };
    doc.groups.push(gr); selG = gr.uid; sel = null;
    marcarCambio(); pintarTabs(); pintarTarjetas();
  }
  function editorGrupo(gr, g, d) {
    const box = document.createElement("div"); box.className = "mk-editor grp-editor";
    const lineas = [...new Set(CAT.patterns.filter((p) => p.ref_key === tab && p.group === g && p.dir === d).map((p) => p.line))].sort(byLine);
    const plan = expandirGrupo(gr);
    const prev = plan.length
      ? `${esc(T("Secuencia del día"))}: ${plan.slice(0, 6).map((x) => `<b>${hhmm(x.t)}</b> ${esc(x.p.line)}`).join(" · ")}${plan.length > 6 ? " · …" : ""} · <b>${plan.length}</b> ${esc(T("trenes al día"))}`
      : esc(T("Añade líneas a la secuencia y arrastra el grupo al reloj (o añade minutos) para generar pasos."));
    box.innerHTML = `<div class="hd"><span class="gtag" style="background:${COLOR_GRUPO}">${esc(sigla(gr))}</span><span>${esc(T("Grupo de líneas"))}</span><button type="button" class="x" aria-label="${esc(T("Cerrar"))}">×</button></div>
      <label class="fld">${esc(T("Nombre del grupo"))}<input type="text" maxlength="40" value="${esc(gr.name)}" data-k="name"></label>
      <div class="fld">${esc(T("Secuencia (en este orden se reparten los pasos)"))}
        <ol class="seq">${gr.items.map((id, i) => {
          const p = PAT.get(id) || { line: "?", color: "#888" };
          return `<li>${logo(p.line, p.color)}<select data-i="${i}">${recorridos(p.line, tab, g, d).map((q) => `<option value="${esc(q.id)}"${q.id === id ? " selected" : ""}>${esc(q.line)} · ${esc(nombreRec(q))}</option>`).join("")}</select>
            <button type="button" data-up="${i}" title="${esc(T("Subir"))}"${i === 0 ? " disabled" : ""}>↑</button><button type="button" data-down="${i}" title="${esc(T("Bajar"))}"${i === gr.items.length - 1 ? " disabled" : ""}>↓</button><button type="button" data-rm="${i}" title="${esc(T("Quitar"))}">✕</button></li>`;
        }).join("") || `<li class="mk-empty">${esc(T("Todavía no hay líneas en la secuencia."))}</li>`}</ol>
      </div>
      <div class="row2 add-row"><select data-k="addline">${lineas.map((l) => `<option value="${esc(l)}">${esc(l)}</option>`).join("")}</select><button type="button" class="btn ghost" data-a="add">+ ${esc(T("Añadir a la secuencia"))}</button></div>
      <div class="fld">${esc(T("Minutos de paso"))}
        <div class="mins">${gr.mins.map((m, i) => `<span class="min-chip">${mm(m)}<button type="button" data-mrm="${i}" aria-label="${esc(T("Quitar"))}">×</button></span>`).join("")}
          <input type="number" min="0" max="59" step="1" placeholder="mm" data-k="addmin"><button type="button" class="btn ghost" data-a="addmin">+</button></div>
      </div>
      <label class="fld">${esc(T("Cada"))}<select data-k="every">${EVERY.map((h) => `<option value="${h}"${h === gr.every ? " selected" : ""}>${h} h</option>`).join("")}</select></label>
      <div class="prev">${prev}</div>
      <div class="acts"><button type="button" class="btn ghost danger" data-a="del">${esc(T("Eliminar grupo"))}</button><button type="button" class="btn" data-a="ok">${esc(T("Listo"))}</button></div>`;
    const set = (fn) => { fn(); marcarCambio(); pintarTabs(); pintarTarjetas(); };
    const cerrar = () => { selG = null; pintarTarjetas(); };
    box.querySelector(".x").onclick = cerrar;
    box.querySelector('[data-a="ok"]').onclick = cerrar;
    box.querySelector('[data-a="del"]').onclick = () => set(() => { doc.groups = doc.groups.filter((x) => x.uid !== gr.uid); selG = null; });
    box.querySelector('[data-k="name"]').onchange = (e) => set(() => { gr.name = e.target.value.trim() || gr.name; });
    box.querySelector('[data-k="every"]').onchange = (e) => set(() => { gr.every = +e.target.value; });
    box.querySelectorAll(".seq select").forEach((s) => s.onchange = () => set(() => { gr.items[+s.dataset.i] = s.value; }));
    box.querySelectorAll("[data-up]").forEach((b) => b.onclick = () => set(() => { const i = +b.dataset.up; [gr.items[i - 1], gr.items[i]] = [gr.items[i], gr.items[i - 1]]; }));
    box.querySelectorAll("[data-down]").forEach((b) => b.onclick = () => set(() => { const i = +b.dataset.down; [gr.items[i + 1], gr.items[i]] = [gr.items[i], gr.items[i + 1]]; }));
    box.querySelectorAll("[data-rm]").forEach((b) => b.onclick = () => set(() => { gr.items.splice(+b.dataset.rm, 1); }));
    box.querySelectorAll("[data-mrm]").forEach((b) => b.onclick = () => set(() => { gr.mins.splice(+b.dataset.mrm, 1); }));
    box.querySelector('[data-a="add"]').onclick = () => {
      const l = box.querySelector('[data-k="addline"]').value, ps = recorridos(l, tab, g, d);
      if (ps.length) set(() => { gr.items.push(ps[0].id); });
    };
    box.querySelector('[data-a="addmin"]').onclick = () => {
      const v = Math.round(+box.querySelector('[data-k="addmin"]').value);
      if (Number.isFinite(v) && v >= 0 && v < 60 && !gr.mins.includes(v)) set(() => { gr.mins.push(v); gr.mins.sort((a, b) => a - b); });
    };
    return box;
  }

  // ---- Editor del marcador (desplegable al lado del reloj) -----------------------------------
  function editor(m, g, d) {
    const p = PAT.get(m.pat), ps = recorridos(m.line, tab, g, d);
    const box = document.createElement("div"); box.className = "mk-editor";
    const opts = (list, cur) => list.map((q) => `<option value="${esc(q.id)}"${q.id === cur ? " selected" : ""}>${esc(nombreRec(q))} · ${q.n}/${esc(T("día"))}</option>`).join("");
    const altList = ps.filter((q) => q.id !== m.pat);
    const ss = slots(m.min, m.every, p.ref_off);
    const prev = ss.length
      ? `${esc(T("Primer tren"))}: ${esc(T("sale de"))} ${esc(p.from_name)} <b>${hhmm(ss[0] - p.ref_off)}</b>, ${esc(T("pasa a las"))} <b>${hhmm(ss[0])}</b> · ${esc(T("último"))} <b>${hhmm(ss[ss.length - 1])}</b> · <b>${ss.length}</b> ${esc(T("trenes al día"))}`
      : esc(T("Con esta hora no cabe ningún tren entre la apertura y la última salida."));
    box.innerHTML = `<div class="hd">${logo(m.line, p.color)}<span>${esc(m.line)} · ${mm(m.min)}</span><button type="button" class="x" aria-label="${esc(T("Cerrar"))}">×</button></div>
      <div class="row2">
        <label class="fld">${esc(T("Minuto"))}<input type="number" min="0" max="59" step="1" value="${m.min}" data-k="min"></label>
        <label class="fld">${esc(T("Cada"))}<select data-k="every">${EVERY.map((h) => `<option value="${h}"${h === m.every ? " selected" : ""}>${h} h</option>`).join("")}</select></label>
      </div>
      <label class="fld">${esc(T("Recorrido"))}<select data-k="pat">${opts(ps, m.pat)}</select></label>
      ${altList.length ? `<div class="alt">
        <label class="chk"><input type="checkbox" data-k="alton"${m.alt ? " checked" : ""}> ${esc(T("Variante: cada N horas va a otro destino"))}</label>
        ${m.alt ? `<div class="row2">
          <label class="fld">${esc(T("Cada"))}<select data-k="altevery">${ALT_EVERY.map((h) => `<option value="${h}"${h === m.alt.every ? " selected" : ""}>${h} h</option>`).join("")}</select></label>
        </div>
        <label class="fld">${esc(T("Recorrido de la variante"))}<select data-k="altpat">${opts(altList, m.alt.pat)}</select></label>` : ""}
      </div>` : ""}
      <div class="prev">${prev}</div>
      <div class="acts"><button type="button" class="btn ghost danger" data-a="del">${esc(T("Eliminar"))}</button><button type="button" class="btn" data-a="ok">${esc(T("Listo"))}</button></div>`;
    const set = (fn) => { fn(); marcarCambio(); pintarTarjetas(); };
    box.querySelector(".x").onclick = () => { sel = null; pintarTarjetas(); };
    box.querySelector('[data-a="ok"]').onclick = () => { sel = null; pintarTarjetas(); };
    box.querySelector('[data-a="del"]').onclick = () => { doc.markers = doc.markers.filter((x) => x.uid !== m.uid); sel = null; marcarCambio(); pintarTabs(); pintarTarjetas(); };
    box.querySelector('[data-k="min"]').onchange = (e) => set(() => { const v = Math.round(+e.target.value); m.min = Number.isFinite(v) ? ((v % 60) + 60) % 60 : m.min; });
    box.querySelector('[data-k="every"]').onchange = (e) => set(() => { m.every = +e.target.value; });
    box.querySelector('[data-k="pat"]').onchange = (e) => set(() => { m.pat = e.target.value; if (m.alt && m.alt.pat === m.pat) m.alt = null; });
    const ao = box.querySelector('[data-k="alton"]');
    if (ao) ao.onchange = (e) => set(() => { m.alt = e.target.checked ? { pat: altList[0].id, every: 3 } : null; });
    const ae = box.querySelector('[data-k="altevery"]');
    if (ae) ae.onchange = (e) => set(() => { m.alt.every = +e.target.value; });
    const ap = box.querySelector('[data-k="altpat"]');
    if (ap) ap.onchange = (e) => set(() => { m.alt.pat = e.target.value; });
    return box;
  }

  // ---- Vista previa (malla generada por el servidor) -----------------------------------------
  let previaTimer = 0, previaSeq = 0;
  function programarPrevia() { clearTimeout(previaTimer); previaTimer = setTimeout(previa, 450); }
  async function previa() {
    const box = $("cadSum");
    if (vacio()) { box.innerHTML = `<span class="sub">${esc(T("Coloca líneas en los relojes para generar la malla."))}</span>`; return; }
    const my = ++previaSeq;
    let r;
    try {
      r = await (await fetch("/api/cadence/load", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(especificacion()) })).json();
    } catch (e) { r = { ok: false, error: String(e) }; }
    if (my !== previaSeq) return;   // llegó tarde: ya hay otra previa en curso
    if (!r.ok) { box.innerHTML = `<span class="warn">${esc(r.error || T("No se pudo generar la malla."))}</span>`; return; }
    const pl = Object.keys(r.per_line || {}).sort(byLine).map((l) => `<span class="pl">${logoUrl(l) ? `<img src="${logoUrl(l)}" alt="${esc(l)}">` : esc(l)} ${r.per_line[l]}</span>`).join("");
    box.innerHTML = `<span><b>${r.n_trains}</b> ${esc(T("trenes al día"))}</span>` +
      (r.first_dep != null ? `<span>${esc(T("primera salida"))} <b>${hhmm(r.first_dep)}</b></span>` : "") + pl +
      (r.warnings || []).map((w) => `<span class="warn">${esc(w)}</span>`).join("");
  }

  // ---- Guardar / nuevo / borrar / importar / exportar ----------------------------------------
  function guardar() {
    const name = $("cadName").value.trim();
    if (!name) { status(T("Ponle un nombre antes de guardarlo."), "err"); $("cadName").focus(); return; }
    if (vacio()) { status(T("El horario está vacío: coloca al menos una línea en un reloj."), "err"); return; }
    doc.name = name;
    const l = lista();
    if (!doc.id) doc.id = "c" + Date.now().toString(36);
    const entry = { id: doc.id, name, updated: Date.now(), spec: especificacion() };
    const i = l.findIndex((x) => x.id === doc.id);
    if (i >= 0) l[i] = entry; else l.push(entry);
    if (!guardarLista(l)) return;
    dirty = false;
    pintarGuardados();
    status(T("Guardado. Ya puedes elegirlo en la pantalla previa del simulador o en Simulaciones encadenadas rápidas."), "ok");
  }
  function descartarCambios() { return !dirty || confirm(T("Hay cambios sin guardar en este horario. ¿Descartarlos?")); }
  function abrir(id) {
    const c = lista().find((x) => x.id === id);
    doc = c ? { id: c.id, name: c.name || "", markers: conUid(c.spec && c.spec.markers), groups: conUidG(c.spec && c.spec.groups) } : nuevoDoc();
    dirty = false; sel = null; selG = null;
    render(); previa(); status("");
  }
  function exportar() {
    if (vacio()) { status(T("No hay nada que exportar: el horario está vacío."), "err"); return; }
    const name = $("cadName").value.trim() || T("horario cadenciado");
    const data = { format: "rodalies-cadence", version: 1, ...especificacion(), name };
    const a = document.createElement("a");
    a.href = URL.createObjectURL(new Blob([JSON.stringify(data, null, 2)], { type: "application/json" }));
    a.download = name.replace(/[^\p{L}\p{N}_-]+/gu, "_").replace(/^_+|_+$/g, "") + ".json";
    document.body.appendChild(a); a.click(); a.remove();
    setTimeout(() => URL.revokeObjectURL(a.href), 2000);
  }
  async function importar(file) {
    let data;
    try { data = JSON.parse(await file.text()); } catch (e) { status(T("El fichero no es un JSON válido."), "err"); return; }
    const markers = conUid(data && data.markers), groups = conUidG(data && data.groups);
    if (!data || data.format !== "rodalies-cadence" || (!markers.length && !groups.length)) { status(T("El fichero no es un horario cadenciado de Rodalies Simulator."), "err"); return; }
    if (!descartarCambios()) return;
    // Se guarda al importar (nombre sin repetir) para poder elegirlo ya en el simulador.
    const base = String(data.name || file.name.replace(/\.json$/i, "")).slice(0, 60), names = new Set(lista().map((c) => c.name));
    let name = base, k = 2; while (names.has(name)) name = `${base} (${k++})`;
    doc = { id: null, name, markers, groups }; sel = null; selG = null;
    $("cadName").value = name;
    guardar(); render(); previa();
    const fuera = markers.filter((m) => !PAT.has(m.pat)).length + groups.reduce((n, g) => n + g.items.filter((x) => !PAT.has(x)).length, 0);
    status(T("Importado y guardado.") + (fuera ? " " + fuera + " " + T("marcadores usan recorridos que no existen en el GTFS cargado (se ignoran).") : ""), fuera ? "err" : "ok");
  }

  $("cadSave").onclick = guardar;
  $("cadNew").onclick = () => { if (descartarCambios()) { doc = nuevoDoc(); dirty = false; sel = null; selG = null; render(); previa(); status(""); } };
  $("cadSaved").onchange = (e) => { const v = e.target.value; if (descartarCambios()) abrir(v); else pintarGuardados(); };
  $("cadDelete").onclick = () => {
    if (!doc.id || !confirm(T("¿Borrar este horario de este navegador?"))) return;
    guardarLista(lista().filter((c) => c.id !== doc.id));
    doc = nuevoDoc(); dirty = false; sel = null; selG = null; render(); previa(); status(T("Horario borrado."), "ok");
  };
  $("cadExport").onclick = exportar;
  $("cadImport").onchange = (e) => { const f = e.target.files[0]; if (f) importar(f); e.target.value = ""; };
  $("cadName").oninput = () => { doc.name = $("cadName").value; dirty = true; };
  window.addEventListener("beforeunload", (e) => { if (dirty) { e.preventDefault(); e.returnValue = ""; } });
  document.addEventListener("keydown", (e) => { if (e.key === "Escape" && (sel != null || selG != null)) { sel = null; selG = null; pintarTarjetas(); } });

  // ---- Arranque --------------------------------------------------------------------------------
  (async function iniciar() {
    try {
      CAT = await (await fetch("/api/cadence/catalog")).json();
    } catch (e) { $("cadSum").innerHTML = `<span class="warn">${esc(T("No se pudo cargar el catálogo de recorridos:"))} ${esc(e)}</span>`; return; }
    for (const p of CAT.patterns) PAT.set(p.id, p);
    tab = (CAT.refs[0] || {}).key;
    // Abre el último horario guardado (o uno nuevo si no hay ninguno).
    const l = lista();
    if (l.length) abrir(l.slice().sort((a, b) => (b.updated || 0) - (a.updated || 0))[0].id); else { render(); previa(); }
  })();
})();
