const state = { brand: "all", city: "all", status: "all", type: "all", q: "" };

async function getJSON(url) {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
  return res.json();
}

function el(tag, cls, text) {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (text !== undefined) node.textContent = text;
  return node;
}

function fmtDate(d) {
  if (!d) return "—";
  const dt = new Date(d);
  if (Number.isNaN(dt.getTime())) return String(d);
  return dt.toISOString().slice(0, 10);
}

function fmtDateTime(d) {
  if (!d) return "—";
  const dt = new Date(d);
  if (Number.isNaN(dt.getTime())) return "—";
  return dt.toISOString().replace("T", " ").slice(0, 16) + " UTC";
}

function fillStats(stats) {
  const cards = document.getElementById("cards");
  cards.innerHTML = "";

  const by = stats.byStatus || {};
  const items = [
    ["Total actions", String(stats.totalActions ?? 0)],
    ["Suspended", String(by.suspended ?? 0)],
    ["Reopened", String(by.reopened ?? 0)],
    ["Active / notices", String(by.active ?? 0)],
    ["Last scrape", fmtDateTime(stats.lastRun?.finishedAt)],
    ["Latest action", fmtDate(stats.latestActionDate)],
  ];
  for (const [label, value] of items) {
    const card = el("div", "card");
    card.append(el("span", "card-label", label), el("span", "card-value", value));
    cards.append(card);
  }
  if (stats.lastRun?.status === "error") {
    cards.append(el("div", "card card-error", "Last scrape failed"));
  }
}

function fillSelect(id, values, placeholder) {
  const select = document.getElementById(id);
  select.innerHTML = "";
  const all = el("option", null, placeholder);
  all.value = "all";
  select.append(all);
  for (const row of values) {
    const opt = el("option", null, `${row.name} (${row.count})`);
    opt.value = row.name;
    select.append(opt);
  }
}

function renderRows(rows) {
  const tbody = document.getElementById("rows");
  const empty = document.getElementById("empty");
  tbody.innerHTML = "";
  empty.hidden = rows.length > 0;

  for (const a of rows) {
    const tr = document.createElement("tr");
    tr.append(el("td", null, fmtDate(a.actionDate)));
    const name = el("td", "strong", a.establishment);
    if (a.area) name.append(el("div", "muted small", a.area));
    tr.append(name);

    tr.append(el("td", null, a.city ?? "—"));
    tr.append(el("td", null, a.brand ?? "—"));

    const typeCell = el("td", null);
    typeCell.append(el("span", `badge badge-${a.outletType || "other"}`, humanize(a.outletType || "other")));
    tr.append(typeCell);

    tr.append(el("td", null, el("span", `badge badge-${a.actionType}`, humanize(a.actionType))));

    const viol = el("td", "violations");
    const list = Array.isArray(a.violations) ? a.violations : [];
    for (const v of list.slice(0, 2)) viol.append(el("div", "violation", v));
    if (list.length > 2) viol.append(el("div", "muted small", `+${list.length - 2} more`));
    if (a.complianceScore != null) viol.append(el("div", "score", `compliance ${a.complianceScore}%`));
    tr.append(viol);

    const platCell = el("td", "platforms");
    const plats = Array.isArray(a.platforms) ? a.platforms : [];
    for (const [i, p] of plats.entries()) {
      if (i >= 4) break;
      platCell.append(el("span", `chip chip-${p}`, p));
    }
    tr.append(platCell);

    const src = el("td");
    const link = el("a", null, a.sourcePublisher || "article");
    link.href = a.sourceUrl;
    link.target = "_blank";
    link.rel = "noopener";
    src.append(link);
    tr.append(src);

    tbody.append(tr);
  }
}

function humanize(s) {
  return s.replace(/_/g, " ");
}

function url() {
  const p = new URLSearchParams();
  for (const [k, v] of Object.entries(state)) {
    if (v && v !== "all") p.set(k, v);
  }
  return `/api/actions?${p.toString()}`;
}

async function init() {
  try {
    const [stats, brands, cities] = await Promise.all([
      getJSON("/api/stats"),
      getJSON("/api/brands"),
      getJSON("/api/cities"),
    ]);
    fillStats(stats);
    fillSelect("f-brand", brands.rows || [], "All brands");
    fillSelect("f-city", cities.rows || [], "All cities");
  } catch (err) {
    document.getElementById("cards").append(el("div", "card card-error", `Failed to load: ${err.message}`));
  }

  for (const id of ["f-brand", "f-city", "f-status", "f-type"]) {
    document.getElementById(id).addEventListener("change", (e) => {
      state[id.replace("f-", "")] = e.target.value;
      refresh();
    });
  }
  let t = null;
  document.getElementById("f-q").addEventListener("input", (e) => {
    clearTimeout(t);
    t = setTimeout(() => {
      state.q = e.target.value.trim();
      refresh();
    }, 300);
  });

  await refresh();
}

async function refresh() {
  const meta = document.getElementById("meta");
  try {
    const data = await getJSON(url());
    renderRows(data.actions || []);
    meta.textContent = `${data.count} record${data.count === 1 ? "" : "s"} · showing ${(data.actions || []).length}`;
  } catch (err) {
    meta.textContent = `Error: ${err.message}`;
  }
}

init();