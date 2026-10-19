// Recolector GTFS-RT de Renfe (Rodalies) en Cloudflare Workers — plan GRATUITO.
//
// Un Durable Object (SQLite) con una ALARMA cada 20 s: en cada una lee vehicle_positions.json +
// trip_updates.json de Renfe, se queda solo con Rodalies (mismo formato que scripts/collect_gtfsrt.py)
// y lo guarda comprimido. Cada ~2 min, los avisos de incidencia. Un cron cada 10 min re-arma la
// alarma por si la cadena se cortara.
//
// Límites del plan gratuito y cómo se respetan:
//   · 10 ms de CPU por ejecución → UNA lectura por alarma y troceo barato del JSON (solo se parsean
//     las entidades de Rodalies; ~2 ms medidos).
//   · 100.000 peticiones/día → ~4.300 alarmas + 144 cron + las consultas del simulador.
//   · 1 GB de SQLite en Durable Objects → ~1,8 KB por lectura comprimida y se borran los >45 días.
//   Sin tarjeta en la cuenta no hay forma de que Cloudflare cobre: al pasarse, simplemente falla.
//
// API (cabecera Authorization: Bearer <TOKEN>):
//   GET  /status                       estado (último guardado, siguiente, sondeos, días)
//   GET  /data?since=<t>&limit=<n>     sondeos con t > since, NDJSON: {"day":..,"rec":{...}}
//   GET  /alerts?since=<seen>          avisos nuevos, NDJSON
//   GET  /incidents?since=<seen>       incidencias del RSS de la Generalitat, NDJSON: {"day":..,"rec":{...}}
//   POST /start  ·  POST /stop         activar / pausar la recogida

const VP_URL = "https://gtfsrt.renfe.com/vehicle_positions.json";
const TU_URL = "https://gtfsrt.renfe.com/trip_updates.json";
const AL_URL = "https://gtfsrt.renfe.com/alerts.json";
const EVERY_MS = 20_000;
const ALERTS_EVERY = 6; // 1 de cada 6 alarmas (~2 min)
// RSS oficial de incidencias de la Generalitat (hora de publicación, línea y texto; solo las vigentes):
// se captura cada ~5 min para que cada día guarde las suyas aunque el PC esté apagado.
const RSS_URL = "https://www.gencat.cat/rodalies/incidencies_rodalies_rss_ca_ES.xml";
const RSS_EVERY = 15; // 1 de cada 15 alarmas (~5 min), desfasada de los avisos
const RETENTION_DAYS = 45;
const RODALIES_ROUTE = /^5\dT/;
const RODALIES_TRIP = /\dR[A-Z0-9]*$/;

const madridDay = new Intl.DateTimeFormat("en-CA", { timeZone: "Europe/Madrid", year: "numeric", month: "2-digit", day: "2-digit" });

// Trocea el feed por entidades sin parsearlo entero (el formato de Renfe indenta cada entidad igual)
// y parsea solo las que pasan `keep`. Si el formato cambiara, cae al JSON.parse completo.
function entities(txt, keep) {
  const MARK = '{\n    "id": "';
  const starts = [];
  let p = -1;
  while ((p = txt.indexOf(MARK, p + 1)) >= 0) starts.push(p);
  if (starts.length === 0) {
    const all = (JSON.parse(txt).entity || []);
    return all.filter((e) => keep(JSON.stringify(e)));
  }
  const out = [];
  for (let k = 0; k < starts.length; k++) {
    const a = starts[k];
    const end = k + 1 < starts.length ? txt.lastIndexOf("}", starts[k + 1]) : txt.lastIndexOf("}", txt.lastIndexOf("]"));
    const chunk = txt.slice(a, end + 1);
    if (keep(chunk)) out.push(JSON.parse(chunk));
  }
  return out;
}

async function getText(url) {
  const r = await fetch(url, { headers: { "User-Agent": "rod4lia-sim/1.0 (cloudflare)" }, cf: { cacheTtl: 0 } });
  if (!r.ok) throw new Error(`${url} → HTTP ${r.status}`);
  return r.text();
}

async function poll() {
  const [vpT, tuT] = await Promise.all([getText(VP_URL), getText(TU_URL)]);
  const delays = {}, canceled = [], skipped = {};
  for (const e of entities(tuT, (c) => RODALIES_TRIP.test((/"tripId": ?"([^"]+)"/.exec(c) || [])[1] || ""))) {
    const u = e.tripUpdate || {};
    const id = u.trip && u.trip.tripId;
    if (!id) continue;
    if (u.trip.scheduleRelationship === "CANCELED") canceled.push(id);
    if ("delay" in u) delays[id] = u.delay;
    const sk = (u.stopTimeUpdate || []).filter((s) => s.scheduleRelationship === "SKIPPED").map((s) => s.stopId);
    if (sk.length) skipped[id] = sk;
  }
  const rows = [];
  for (const e of entities(vpT, (c) => /"label": ?"R/.test(c))) {
    const v = e.vehicle || {};
    const label = (v.vehicle && v.vehicle.label) || "";
    const line = label.split("-")[0];
    if (!(line[0] === "R" && line.length >= 2)) continue;
    const tid = (v.trip && v.trip.tripId) || "";
    const pos = v.position || {};
    const m = /PLATF\.\((\d+)\)/.exec(label);
    const st = { STOPPED_AT: "S", IN_TRANSIT_TO: "T", INCOMING_AT: "I" }[v.currentStatus] || "?";
    rows.push([tid, line, v.stopId || "", st, pos.latitude, pos.longitude, +(v.timestamp || 0), delays[tid] ?? null, m ? +m[1] : null]);
  }
  const t = +((/"timestamp": ?"(\d+)"/.exec(vpT) || [])[1] || Math.floor(Date.now() / 1000));
  return { t, v: rows, c: canceled, k: skipped };
}

async function gzip(str) {
  const cs = new Response(new Blob([str]).stream().pipeThrough(new CompressionStream("gzip")));
  return new Uint8Array(await cs.arrayBuffer());
}
async function gunzip(buf) {
  return new Response(new Blob([buf]).stream().pipeThrough(new DecompressionStream("gzip"))).text();
}

export class Collector {
  constructor(state, env) {
    this.state = state;
    this.env = env;
    this.sql = state.storage.sql;
    this.sql.exec(`CREATE TABLE IF NOT EXISTS polls (t INTEGER PRIMARY KEY, day TEXT, data BLOB)`);
    this.sql.exec(`CREATE TABLE IF NOT EXISTS alerts (seen INTEGER, id TEXT, text TEXT, data TEXT, PRIMARY KEY (id, text))`);
    this.sql.exec(`CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v TEXT)`);
    this.sql.exec(`CREATE TABLE IF NOT EXISTS incidents (key TEXT PRIMARY KEY, pub INTEGER, day TEXT, seen INTEGER, data TEXT)`);
  }

  get(k, d = null) {
    const r = this.sql.exec(`SELECT v FROM meta WHERE k = ?`, k).toArray()[0];
    return r ? JSON.parse(r.v) : d;
  }
  set(k, v) {
    this.sql.exec(`INSERT INTO meta (k, v) VALUES (?, ?) ON CONFLICT(k) DO UPDATE SET v = excluded.v`, k, JSON.stringify(v));
  }

  async ensureAlarm() {
    if (!this.get("enabled", true)) return;
    const a = await this.state.storage.getAlarm();
    if (a === null || a < Date.now() - 60_000) {
      await this.state.storage.setAlarm(Date.now() + 1000);
    }
  }

  async alarm() {
    if (!this.get("enabled", true)) return;
    const n = (this.get("ticks", 0) || 0) + 1;
    this.set("ticks", n);
    const next = Date.now() + EVERY_MS;
    await this.state.storage.setAlarm(next); // primero re-armar: si algo falla abajo, la cadena sigue
    this.set("next_at", next / 1000);
    // Señal de vida: se actualiza en CADA alarma aunque Renfe no traiga datos nuevos (de madrugada el
    // feed puede no cambiar). El recolector del PC la usa para saber si la nube está grabando.
    this.set("alive_at", Date.now() / 1000);
    try {
      if (n % RSS_EVERY === 7) {
        await this.pollRss();
      } else if (n % ALERTS_EVERY === 0) {
        await this.pollAlerts();
      } else {
        const rec = await poll();
        const day = madridDay.format(new Date(rec.t * 1000));
        const before = this.sql.exec(`SELECT 1 FROM polls WHERE t = ?`, rec.t).toArray().length;
        if (!before) {
          this.sql.exec(`INSERT OR IGNORE INTO polls (t, day, data) VALUES (?, ?, ?)`, rec.t, day, await gzip(JSON.stringify(rec)));
          this.set("polls", (this.get("polls", 0) || 0) + 1);
          this.set("saved_at", Date.now() / 1000);
          this.set("last", rec.t);
        }
      }
      if (n % 180 === 0) {
        // Una vez cada hora: borrar lo de más de 45 días (mantiene la base muy por debajo de 1 GB).
        this.sql.exec(`DELETE FROM polls WHERE t < ?`, Math.floor(Date.now() / 1000) - RETENTION_DAYS * 86400);
        this.sql.exec(`DELETE FROM alerts WHERE seen < ?`, Math.floor(Date.now() / 1000) - RETENTION_DAYS * 86400);
        this.sql.exec(`DELETE FROM incidents WHERE seen < ?`, Math.floor(Date.now() / 1000) - RETENTION_DAYS * 86400);
      }
    } catch (e) {
      this.set("errors", (this.get("errors", 0) || 0) + 1);
      this.set("last_error", String(e && e.message || e).slice(0, 300));
    }
  }

  // Incidencias del RSS oficial (misma clave que scripts/incidents.py: sha1("gencat|pub|texto")[:16]).
  async pollRss() {
    const xml = await getText(RSS_URL);
    const now = Math.floor(Date.now() / 1000);
    const un = (x) => x.replace(/<!\[CDATA\[|\]\]>/g, "").replace(/&#(\d+);/g, (_, d) => String.fromCharCode(+d))
      .replace(/&quot;/g, '"').replace(/&#39;|&apos;/g, "'").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&amp;/g, "&").trim();
    const tag = (item, t) => {
      const a = item.indexOf(`<${t}>`), b = item.indexOf(`</${t}>`);
      return a >= 0 && b > a ? un(item.slice(a + t.length + 2, b)) : "";
    };
    for (const item of xml.match(/<item>[\s\S]*?<\/item>/g) || []) {
      const title = tag(item, "title"), desc = tag(item, "description"), pubS = tag(item, "pubDate");
      const pub = Math.floor(Date.parse(pubS) / 1000);
      if (!pub) continue;
      const m = /^L[íi]nia\s+([^.]+)\./.exec(title);
      const lines = m ? m[1].split(",").map((x) => x.trim()) : [];
      const text = desc || title;
      const digest = await crypto.subtle.digest("SHA-1", new TextEncoder().encode(`gencat|${pub}|${text}`));
      const key = [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("").slice(0, 16);
      const rec = { key, source: "gencat", pub, lines, text, captured: now };
      this.sql.exec(`INSERT OR IGNORE INTO incidents (key, pub, day, seen, data) VALUES (?, ?, ?, ?, ?)`,
        key, pub, madridDay.format(new Date(pub * 1000)), now, JSON.stringify(rec));
    }
  }

  async pollAlerts() {
    const txt = await getText(AL_URL);
    const now = Math.floor(Date.now() / 1000);
    for (const e of entities(txt, (c) => /"routeId": ?"5\dT/.test(c))) {
      const a = e.alert || {};
      const routes = (a.informedEntity || []).map((x) => x.routeId || "");
      if (!routes.some((r) => RODALIES_ROUTE.test(r))) continue;
      const tr = ((a.descriptionText || a.headerText || {}).translation || [])[0] || {};
      const text = tr.text || "";
      const rec = { id: e.id, seen: now, period: a.activePeriod, routes, stops: (a.informedEntity || []).filter((x) => x.stopId).map((x) => x.stopId), text };
      this.sql.exec(`INSERT OR IGNORE INTO alerts (seen, id, text, data) VALUES (?, ?, ?, ?)`, now, e.id, text, JSON.stringify(rec));
    }
  }

  async fetch(req) {
    const url = new URL(req.url);
    const path = url.pathname;
    if (path === "/status") {
      await this.ensureAlarm();
      const days = this.sql.exec(`SELECT day, COUNT(*) AS n, SUM(LENGTH(data)) AS b FROM polls GROUP BY day ORDER BY day`).toArray();
      const enabled = this.get("enabled", true);
      return Response.json({
        enabled,
        running: enabled ? {
          polls: this.get("polls", 0), saved_at: this.get("saved_at", 0), next_at: this.get("next_at", 0),
          alive_at: this.get("alive_at", 0),
          last: this.get("last", 0), errors: this.get("errors", 0), last_error: this.get("last_error", null),
          started: this.get("started", 0),
        } : null,
        days: days.map((d) => ({ day: d.day, polls: d.n, mb: (d.b || 0) / 1e6 })),
        alerts: this.sql.exec(`SELECT COUNT(*) AS n FROM alerts`).toArray()[0].n,
        db_mb: this.state.storage.sql.databaseSize / 1e6,
        now: Date.now() / 1000,
      });
    }
    if (path === "/data") {
      const since = +(url.searchParams.get("since") || 0);
      const limit = Math.min(+(url.searchParams.get("limit") || 500), 1000);
      const rows = this.sql.exec(`SELECT t, day, data FROM polls WHERE t > ? ORDER BY t LIMIT ?`, since, limit).toArray();
      const lines = [];
      for (const r of rows) lines.push(`{"day":"${r.day}","rec":${await gunzip(r.data)}}`);
      return new Response(lines.join("\n") + (lines.length ? "\n" : ""), { headers: { "content-type": "application/x-ndjson" } });
    }
    if (path === "/incidents") {
      const since = +(url.searchParams.get("since") || 0);
      const rows = this.sql.exec(`SELECT day, data FROM incidents WHERE seen > ? ORDER BY seen`, since).toArray();
      return new Response(rows.map((r) => `{"day":"${r.day}","rec":${r.data}}`).join("\n") + (rows.length ? "\n" : ""), { headers: { "content-type": "application/x-ndjson" } });
    }
    if (path === "/alerts") {
      const since = +(url.searchParams.get("since") || 0);
      const rows = this.sql.exec(`SELECT data FROM alerts WHERE seen > ? ORDER BY seen`, since).toArray();
      return new Response(rows.map((r) => r.data).join("\n") + (rows.length ? "\n" : ""), { headers: { "content-type": "application/x-ndjson" } });
    }
    if (path === "/start" && req.method === "POST") {
      this.set("enabled", true);
      if (!this.get("started", 0)) this.set("started", Date.now() / 1000);
      await this.state.storage.setAlarm(Date.now() + 1000);
      return Response.json({ ok: true });
    }
    if (path === "/stop" && req.method === "POST") {
      this.set("enabled", false);
      await this.state.storage.deleteAlarm();
      return Response.json({ ok: true });
    }
    if (path === "/kick") {
      await this.ensureAlarm();
      return Response.json({ ok: true });
    }
    return new Response("not found", { status: 404 });
  }
}

function stub(env) {
  return env.COLLECTOR.get(env.COLLECTOR.idFromName("rodalies"));
}

export default {
  async fetch(req, env) {
    const auth = req.headers.get("authorization") || "";
    if (!env.TOKEN || auth !== `Bearer ${env.TOKEN}`) {
      return new Response("unauthorized", { status: 401 });
    }
    return stub(env).fetch(req);
  },
  // Vigilante cada 10 min: si la alarma se hubiera perdido, la vuelve a armar.
  async scheduled(_event, env) {
    await stub(env).fetch("https://collector/kick");
  },
};
