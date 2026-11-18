// Window logic. All real work happens in Rust; this file draws the views
// (the accounts, one account's settings, the add-account wizard, the pages
// the rail leads to) and forwards what the user does.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const GAME_NAMES = { lol: "League / TFT", valorant: "VALORANT", both: "League ve VALORANT", client: "Riot Client" };
const GAME_SHORT = { lol: "LoL", valorant: "VAL" };
// The game buttons on an account's row: one for its game, or one for each
// when it plays both. The row itself opens the first.
const rowGames = (a) => (a.game === "both" ? ["lol", "valorant"] : a.game in GAME_SHORT ? [a.game] : []);

let view = { accounts: [], active: null, client: {}, security: {}, unsaved_session: false, pending: null, autostart: false };
let selected = 0;      // index into the filtered list
let busy = null;       // id of the account being switched to
let editing = null;    // id of the account open in the settings view
let wizard = null;     // null, or { step, mode } while the wizard is open
let afterUnlock = null; // id of an account to switch to once the lock opens

let shownIds = null;    // account ids last drawn, to animate the list only when it changes
let drag = null;       // the row being dragged, while reordering
let gameFilter = "all"; // "all", "lol" or "valorant"
try { gameFilter = localStorage.getItem("gameFilter") || "all"; } catch {}

const errorText = (e) => (e && e.message) || String(e);
const errorKind = (e) => (e && e.kind) || "other";

const VIEWS = ["list", "edit", "wizard", "unlock", "security", "settings", "data", "net", "log"];
// The rail marks the page that is open; an account's settings, the wizard
// and the unlock prompt all belong to the accounts.
const RAIL_HOME = { edit: "list", wizard: "list", unlock: "list" };

function show(name) {
  for (const v of VIEWS) $(`view-${v}`).hidden = v !== name;
  const home = RAIL_HOME[name] || name;
  for (const b of document.querySelectorAll("#rail .nav")) {
    if (b.dataset.view === home) b.setAttribute("aria-current", "page");
    else b.removeAttribute("aria-current");
  }
}

// ---- the seal ------------------------------------------------------------------
// A padlock that closes (or opens) over the window while `work` runs, so
// the moment something is encrypted is visible. It stays up at least long
// enough to be read, and gets out of the way at once if `work` fails.

const pause = (ms) => new Promise((r) => setTimeout(r, ms));

async function withSeal(kind, text, doneText, work) {
  const seal = $("seal");
  const opening = kind === "open";
  seal.className = opening ? "opening working" : "working";
  $("seal-text").textContent = text;
  seal.hidden = false;
  try {
    const [result] = await Promise.all([work(), pause(450)]);
    seal.className = opening ? "opened" : "closed";
    $("seal-text").textContent = doneText;
    await pause(750);
    return result;
  } finally {
    seal.classList.add("leaving");
    await pause(seal.classList.contains("working") ? 0 : 200);
    seal.hidden = true;
  }
}

// Marks a button as working for as long as `work` runs: it is disabled and
// shows a moving line, so a press is never met with silence.
async function whileWorking(button, work) {
  button.disabled = true;
  button.classList.add("working");
  try {
    return await work();
  } finally {
    button.disabled = false;
    button.classList.remove("working");
  }
}

// ---- account list ---------------------------------------------------------

const plays = (a, game) => game === "all" || a.game === game || a.game === "both";

// ---- order and density ---------------------------------------------------------
// Both are this window's own taste, kept in the browser's storage.

const TIERS = ["iron", "bronze", "silver", "gold", "platinum", "emerald", "diamond", "master", "grandmaster", "challenger"];
const DIVISIONS = { IV: 0, III: 1, II: 2, I: 3 };

// "Gold II 45 LP" as a number that grows with the rank; -1 without one.
function rankValue(rank) {
  if (!rank) return -1;
  const parts = rank.split(" ");
  const tier = TIERS.indexOf(parts[0].toLowerCase());
  if (tier < 0) return -1;
  const division = DIVISIONS[parts[1]] ?? 0;
  const lp = Number(parts[parts.length - 2]) || 0;
  return tier * 100000 + division * 10000 + Math.min(lp, 9999);
}

const SORTS = {
  name: (a, b) => a.name.localeCompare(b.name, "tr"),
  rank: (a, b) => rankValue(b.rank) - rankValue(a.rank),
  level: (a, b) => (b.level || 0) - (a.level || 0),
  recent: (a, b) => Math.max(b.last_ok || 0, b.updated || 0) - Math.max(a.last_ok || 0, a.updated || 0),
};

let sortBy = "manual";
let layout = "cards";   // "cards" or "rows"
let statusFilter = "all"; // "all", "open", "attention", "banned" or "stale"
try {
  if (localStorage.getItem("sortBy") in SORTS) sortBy = localStorage.getItem("sortBy");
  if (localStorage.getItem("layout") === "rows") layout = "rows";
} catch {}

function renderTools() {
  $("sort").value = sortBy;
  for (const b of $("layout").children) b.setAttribute("aria-checked", String(b.dataset.layout === layout));
  $("accounts").className = [layout, ...["arriving", "reordering"].filter((c) => $("accounts").classList.contains(c))].join(" ");
}

$("sort").addEventListener("change", (e) => {
  sortBy = e.target.value;
  try { localStorage.setItem("sortBy", sortBy); } catch {}
  selected = 0;
  render();
});
$("layout").addEventListener("click", (e) => {
  const button = e.target.closest("button");
  if (!button) return;
  layout = button.dataset.layout;
  try { localStorage.setItem("layout", layout); } catch {}
  render();
});

// ---- what needs a look ---------------------------------------------------------
// The rail counts the accounts by state; each count filters the list to it.

const STATUS = {
  all: { label: "Hepsi", test: () => true },
  open: { label: "Açık", test: (a) => a.id === view.active },
  attention: { label: "Giriş gerekli", test: (a) => a.needs_login },
  banned: { label: "Banlı", test: (a) => !!banText(a.ban) },
  stale: { label: "Uzun süredir açılmadı", test: (a) => a.stale },
};

function renderStatus() {
  const counts = Object.fromEntries(Object.entries(STATUS).map(([key, s]) => [key, view.accounts.filter(s.test).length]));
  // Only states some account is in are worth a line; "Hepsi" comes with them.
  const shown = Object.keys(STATUS).filter((key) => key !== "all" && key !== "open" && counts[key]);
  if (!shown.includes(statusFilter)) statusFilter = "all";
  $("status").hidden = !shown.length;
  $("status").replaceChildren(...["all", ...shown].map((key) => {
    const b = document.createElement("button");
    b.type = "button";
    b.setAttribute("role", "radio");
    b.dataset.status = key;
    b.setAttribute("aria-checked", String(key === statusFilter));
    const label = document.createElement("span");
    label.textContent = STATUS[key].label;
    const count = document.createElement("b");
    count.textContent = counts[key];
    b.append(label, count);
    return b;
  }));
  $("nav-count").textContent = view.accounts.length || "";
}

$("status").addEventListener("click", (e) => {
  const button = e.target.closest("button");
  if (!button) return;
  statusFilter = button.dataset.status;
  selected = 0;
  show("list");
  render();
});

function visibleAccounts() {
  const q = $("filter").value.trim().toLocaleLowerCase("tr");
  const shown = view.accounts.filter(
    (a) => plays(a, gameFilter) && STATUS[statusFilter].test(a)
      && `${a.name} ${a.riot_id || ""} ${a.note || ""}`.toLocaleLowerCase("tr").includes(q),
  );
  // The stored order is the user's own; any other is only a way of looking.
  return sortBy in SORTS ? shown.sort(SORTS[sortBy]) : shown;
}

// How long ago a Unix time (seconds) was, in words.
function ago(seconds) {
  const minutes = Math.max(0, Math.round((Date.now() / 1000 - seconds) / 60));
  if (minutes < 2) return "az önce";
  if (minutes < 60) return `${minutes} dk önce`;
  if (minutes < 48 * 60) return `${Math.round(minutes / 60)} sa önce`;
  return `${Math.round(minutes / 1440)} gün önce`;
}

// Where an account's rank came from and how old it is, for a tooltip.
function rankSource(a) {
  if (!a.rank_at) return "";
  const from = a.rank_from === "client" ? "League istemcisinden okundu" : a.rank_from === "opgg" ? "OP.GG'den alındı" : "";
  return from ? `${from}, ${ago(a.rank_at)}` : "";
}

// ---- profile icons ---------------------------------------------------------------
// Pictures arrive from the app as data, keyed by icon number, and are kept
// for as long as the window lives. A number is asked for once.

const iconPictures = new Map(); // number -> data: address, or null when there is none to show
let iconsAsked = false;

async function fetchIcons() {
  const wanted = [...new Set(view.accounts.map((a) => a.icon).filter((n) => n != null && !iconPictures.has(n)))];
  if (!wanted.length || iconsAsked) return;
  iconsAsked = true;
  try {
    const got = await invoke("icons", { numbers: wanted });
    for (const n of wanted) iconPictures.set(n, got[n] || null);
    if (Object.keys(got).length) render();
  } catch {} finally {
    iconsAsked = false;
  }
}

// A League ban that is still running, as words; "" when there is none.
function banText(ban) {
  if (!ban) return "";
  if (ban.until == null) return "kalıcı ban";
  const left = ban.until - Date.now();
  if (left <= 0) return "";
  const hours = Math.ceil(left / 3600000);
  return hours >= 48 ? `ban: ${Math.ceil(hours / 24)} gün` : hours > 1 ? `ban: ${hours} saat` : "ban: 1 saatten az";
}

// The game an account opens from its row: with a game filter on, an
// account that plays both opens the filtered game.
const rowGame = (a) => (a.game === "both" && gameFilter !== "all" ? gameFilter : null);

// ---- game filter --------------------------------------------------------------
// Shown only when the accounts span both games, otherwise it filters nothing.

function renderGameFilter() {
  const useful = view.accounts.some((a) => a.game === "lol" || a.game === "both")
    && view.accounts.some((a) => a.game === "valorant" || a.game === "both");
  $("games").hidden = !useful;
  if (!useful) gameFilter = "all";
  for (const b of $("games").children) b.setAttribute("aria-checked", String(b.dataset.game === gameFilter));
}

$("games").addEventListener("click", (e) => {
  const game = e.target.dataset && e.target.dataset.game;
  if (!game) return;
  gameFilter = game;
  try { localStorage.setItem("gameFilter", game); } catch {}
  selected = 0;
  render();
  $("filter").focus();
});

// `action` is an optional { label, run } shown as a button beside the text.
let sayTimer = null;
function say(text, isError = false, action = null) {
  clearTimeout(sayTimer);
  // Something merely said goes away by itself; an error or a question
  // stays until it is answered or clicked away.
  if (text && !isError && !action) sayTimer = setTimeout(() => say(""), 6000);
  $("message").hidden = !text;
  $("message").classList.toggle("error", isError);
  $("message-text").textContent = text || "";
  const btn = $("message-action");
  btn.hidden = !action;
  if (action) {
    btn.textContent = action.label;
    btn.onclick = action.run;
  }
}

$("message-text").addEventListener("click", () => say(""));

function hotkeyLabel(hotkey) {
  return hotkey.replace(/Key|Digit/g, "").replace("Numpad", "Num ");
}

function render() {
  renderGameFilter();
  renderStatus();
  renderTools();
  const accounts = visibleAccounts();
  selected = Math.min(selected, Math.max(accounts.length - 1, 0));
  const rows = accounts.map((a, i) => row(a, i));
  if (!rows.length) rows.push(emptyState());
  // Accounts animate in only when the set of them changes, not on the
  // periodic refresh.
  const ids = view.accounts.map((a) => a.id).sort().join("|");
  $("accounts").classList.toggle("arriving", ids !== shownIds);
  shownIds = ids;
  $("accounts").replaceChildren(...rows);
  $("tools").hidden = view.accounts.length < 2;

  const update = view.update || {};
  $("update").hidden = !update.available;
  if (update.available) $("update-text").textContent = `Yeni sürüm hazır: ${update.available}`;
  const isLocked = view.security.lock_enabled && !view.security.unlocked;
  $("locked").hidden = !isLocked;
  $("unsaved").hidden = !view.unsaved_session || isLocked;
  const queued = view.accounts.find((a) => a.id === view.pending);
  $("queue").hidden = !queued;
  if (queued) $("queue-text").textContent = `Oyun kapanınca "${queued.name}" hesabına geçilecek.`;
  $("settings-dot").hidden = !update.available;

  // The internet: asked about once, then shown as what it is.
  const net = view.net || {};
  $("net-ask").hidden = !!net.decided || !view.accounts.length || isLocked;
  const ranksOn = !!(net.decided && net.online && net.lol_ranks);
  $("ranks-refresh").hidden = !ranksOn || !view.accounts.length;
  $("net-state").textContent = netWord(net).toLocaleLowerCase("tr");
  if (net.decided && net.online && net.icons) fetchIcons();

  // The rail's foot: the lock, as a button that does the obvious thing.
  $("rail-lock").hidden = !view.security.lock_enabled;
  $("footer-lock").classList.toggle("closed", isLocked);
  $("rail-lock-text").textContent = isLocked ? "Kilitli: aç" : "Kilit açık: kilitle";
  $("rail-version").textContent = view.version ? `Sürüm ${view.version}` : "";
}

// What the accounts area says when it has nothing to show, and what to do.
function emptyState() {
  const li = document.createElement("li");
  li.className = "empty";
  const title = document.createElement("b");
  const text = document.createElement("span");
  if (!view.accounts.length) {
    title.textContent = "Henüz hesap yok";
    text.textContent = "\"Hesap ekle\" ile ilk hesabını ekle. Çok hesabın varsa Taşı ve yedekle altından topluca ekleyebilirsin.";
  } else if ($("filter").value.trim()) {
    title.textContent = "Eşleşen hesap yok";
    text.textContent = "Aramayı değiştir ya da temizle.";
  } else if (statusFilter !== "all") {
    title.textContent = "Bu durumda hesap yok";
    text.textContent = "Soldan \"Hepsi\"ni seçerek bütün hesapları gör.";
  } else {
    title.textContent = `${gameFilter === "lol" ? "League" : "VALORANT"} oynayan hesap yok`;
    text.textContent = "Üstten \"Tümü\"nü seç.";
  }
  li.append(title, text);
  return li;
}

$("rail-lock").addEventListener("click", async () => {
  if (view.security.lock_enabled && !view.security.unlocked) return openUnlock(null);
  await withSeal("close", "Kilitleniyor…", "Kilitlendi", () => invoke("lock_now"));
  show("list");
  refresh();
});

// The theme sets headings and buttons in capitals, and the page is Turkish,
// where "i" capitalises to "İ". A name that is not Turkish (Riot, Diamond)
// goes in a span that says so, and keeps its own letters: RIOT, not RİOT.
function asWritten(text) {
  const span = document.createElement("span");
  span.lang = "en";
  span.textContent = text;
  return span;
}

// "Gold II 45 LP" in its parts; null without a rank.
function rankParts(rank) {
  if (!rank) return null;
  const words = rank.split(" ");
  const tier = words[0];
  if (!TIERS.includes(tier.toLowerCase())) return null;
  const division = words[1] in DIVISIONS ? words[1] : "";
  const lp = Number(words[words.length - 2]) || 0;
  return { tier, division, lp };
}

const SVG = "http://www.w3.org/2000/svg";

// ---- crests --------------------------------------------------------------------
// QuickSwitch's own mark for a rank: a hexagonal plate in the tier's metal,
// with the division inside. What grows with the tier is the setting around
// it, so the tier can be told at a glance without reading:
//
//   points  blades on the plate's corners: none for Iron, then one, two,
//           a crown of three, four, and all six from Emerald on
//   long    which of those are drawn long (the upright one from Diamond)
//   bevel   a second line inside the plate, from Bronze
//   edges   small blades between the corners, from Grandmaster
//   rings   a halo around the whole, one for Master, two for Challenger
//   glow    the crest lights the card behind it, from Diamond
//
// The metal itself is a gradient defined once in index.html (#metal-<tier>).
const CRESTS = {
  iron:        { points: [], long: [], bevel: false, edges: false, rings: 0, glow: false },
  bronze:      { points: [0], long: [], bevel: true, edges: false, rings: 0, glow: false },
  silver:      { points: [0, 3], long: [], bevel: true, edges: false, rings: 0, glow: false },
  gold:        { points: [5, 0, 1], long: [0], bevel: true, edges: false, rings: 0, glow: false },
  platinum:    { points: [5, 0, 1, 3], long: [0], bevel: true, edges: false, rings: 0, glow: false },
  emerald:     { points: [0, 1, 2, 3, 4, 5], long: [], bevel: true, edges: false, rings: 0, glow: false },
  diamond:     { points: [0, 1, 2, 3, 4, 5], long: [0, 3], bevel: true, edges: false, rings: 0, glow: true },
  master:      { points: [0, 1, 2, 3, 4, 5], long: [0, 1, 2, 3, 4, 5], bevel: true, edges: false, rings: 1, glow: true },
  grandmaster: { points: [0, 1, 2, 3, 4, 5], long: [0, 1, 2, 3, 4, 5], bevel: true, edges: true, rings: 1, glow: true },
  challenger:  { points: [0, 1, 2, 3, 4, 5], long: [0, 1, 2, 3, 4, 5], bevel: true, edges: true, rings: 2, glow: true },
};

const CREST = { size: 56, mid: 28, plate: 13.5 };

function svgNode(name, attrs) {
  const node = document.createElementNS(SVG, name);
  for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, value);
  return node;
}

// A point `r` from the crest's middle, `degrees` clockwise from straight up.
function crestPoint(r, degrees) {
  const a = (degrees - 90) * Math.PI / 180;
  return `${(CREST.mid + r * Math.cos(a)).toFixed(2)} ${(CREST.mid + r * Math.sin(a)).toFixed(2)}`;
}

const hexPath = (r) => `M${[0, 1, 2, 3, 4, 5].map((k) => crestPoint(r, k * 60)).join("L")}Z`;

// A blade: a narrow triangle standing on the plate at `degrees`, its foot
// tucked under the plate's edge, reaching `length` beyond `from`.
function blade(from, degrees, length, half) {
  return `M${crestPoint(from - 3, degrees - half)}L${crestPoint(from + length, degrees)}L${crestPoint(from - 3, degrees + half)}Z`;
}

// The crest for a rank ("Gold II 45 LP" in parts), or a dashed outline
// when there is none.
function crest(parts) {
  const svg = svgNode("svg", { viewBox: `0 0 ${CREST.size} ${CREST.size}`, class: "crest", "aria-hidden": "true" });
  const R = CREST.plate;
  if (!parts) {
    svg.append(svgNode("path", { class: "plate none", d: hexPath(R), "stroke-dasharray": "3 4" }));
    return svg;
  }
  const tier = parts.tier.toLowerCase();
  const spec = CRESTS[tier] || CRESTS.iron;
  const metal = `url(#metal-${tier})`;
  if (spec.glow) svg.classList.add("glow");

  for (let ring = 0; ring < spec.rings; ring++) {
    svg.append(svgNode("circle", { class: "ring", cx: CREST.mid, cy: CREST.mid, r: R + 7.5 + ring * 3.5, stroke: metal, "stroke-opacity": ring ? ".45" : ".8" }));
  }
  // Blades go under the plate, so only what stands clear of it shows.
  const blades = [];
  if (spec.edges) for (let k = 0; k < 6; k++) blades.push(blade(R * 0.866, k * 60 + 30, 6, 11));
  for (const k of spec.points) blades.push(blade(R, k * 60, spec.long.includes(k) ? 11 : 6.5, 15));
  if (blades.length) svg.append(svgNode("path", { class: "blades", d: blades.join(""), fill: metal }));

  svg.append(svgNode("path", { class: "plate", d: hexPath(R), stroke: metal }));
  if (spec.bevel) svg.append(svgNode("path", { class: "bevel", d: hexPath(R - 3.6), stroke: metal }));

  if (parts.division) {
    const text = svgNode("text", { x: CREST.mid, y: CREST.mid + 4.4, "text-anchor": "middle" });
    text.textContent = parts.division;
    svg.append(text);
  } else {
    // Above Diamond there are no divisions: a cut stone takes their place.
    const m = CREST.mid;
    svg.append(svgNode("path", { class: "stone", d: `M${m} ${m - 6.5}L${m + 5} ${m}L${m} ${m + 6.5}L${m - 5} ${m}Z`, fill: metal }));
    svg.append(svgNode("path", { class: "stone-cut", d: `M${m - 5} ${m}H${m + 5}M${m} ${m - 6.5}V${m + 6.5}` }));
  }
  return svg;
}

function row(a, i) {
  const li = document.createElement("li");
  li.className = "account";
  li.dataset.id = a.id;
  li.style.setProperty("--i", i);
  li.addEventListener("pointerdown", (e) => startDrag(e, li));
  const active = a.id === view.active;
  li.classList.toggle("active", active);
  li.classList.toggle("selected", i === selected);
  li.classList.toggle("busy", a.id === busy);
  const rank = rankParts(a.rank);
  if (rank) li.dataset.tier = rank.tier.toLowerCase();

  const go = document.createElement("button");
  go.type = "button";
  go.className = "go";
  go.disabled = busy !== null;
  go.addEventListener("click", () => switchTo(a, rowGame(a)));

  // Who it is: the name with what needs saying about it, the Riot ID under it.
  const name = document.createElement("span");
  name.className = "name";
  const label = document.createElement("span");
  label.textContent = a.name;
  name.append(label);
  const ban = banText(a.ban);
  if (ban) {
    const t = tag(ban, true);
    t.title = a.ban.until == null
      ? "Riot bu hesap için kalıcı League banı bildirdi. Oyun açılırsa bağlantı kesilir."
      : `Riot bu hesap için League banı bildirdi; ${new Date(a.ban.until).toLocaleString("tr")} tarihinde bitiyor. O zamana kadar oyun açılırsa bağlantı kesilir.`;
    name.append(t);
  }
  if (a.lookup_problem === "not_found") {
    const t = tag("Riot ID bulunamadı", true);
    t.title = "OP.GG'de bu Riot ID bu bölgede yok. Hesap ayarlarından Riot ID'yi ve bölgeyi denetle.";
    name.append(t);
  }
  if (a.needs_login) name.append(tag("giriş gerekli", true));
  // A ban says more than an unused session does; one warning of the two is enough.
  else if (a.stale && !ban) {
    // Riot ends sessions that go unused; this one may already be gone.
    const t = tag(`${a.idle_days} gündür açılmadı`, true);
    t.title = a.has_password
      ? "Uzun süredir kullanılmayan oturumu Riot sonlandırmış olabilir. Öyleyse kayıtlı şifreyle giriş yapılır."
      : "Uzun süredir kullanılmayan oturumu Riot sonlandırmış olabilir. Öyleyse Riot Client'ta bir kez elle giriş yapman gerekir.";
    name.append(t);
  }
  const sub = document.createElement("span");
  sub.className = "sub";
  if (active) {
    const on = document.createElement("span");
    on.className = "on";
    on.textContent = "Açık";
    sub.append(on);
  }
  sub.append(a.riot_id || (a.has_session ? "Oturum kayıtlı" : a.has_password ? "Şifreyle giriş" : "Giriş bekliyor"));
  const who = document.createElement("span");
  who.className = "who";
  who.append(name, sub);

  // Where it stands: tier and division, and how far into it.
  const standing = document.createElement("span");
  standing.className = rank ? "rank" : "rank none";
  const line = document.createElement("span");
  line.className = "rank-line";
  const tier = document.createElement("b");
  tier.textContent = rank ? `${rank.tier} ${rank.division}`.trim() : a.game === "valorant" ? "VALORANT" : a.level ? "Derecesiz" : "Rank bilinmiyor";
  if (rank) tier.lang = "en"; // tier names are English
  line.append(tier);
  if (rank) {
    const lp = document.createElement("span");
    lp.textContent = `${rank.lp} LP`;
    line.append(lp);
  }
  line.title = rankSource(a);
  const bar = document.createElement("span");
  bar.className = "lp";
  const fill = document.createElement("i");
  // Up to Diamond a division is a hundred points; above it the points only grow.
  fill.style.setProperty("--lp", rank ? (rank.division ? Math.min(rank.lp, 100) / 100 : 1) : 0);
  bar.append(fill);
  standing.append(line, bar);

  const detail = document.createElement("span");
  detail.className = "detail";
  const part = (text, cls = "") => {
    const s = document.createElement("span");
    s.textContent = text;
    s.className = cls;
    detail.append(s);
  };
  if (a.wins != null && a.losses != null && a.wins + a.losses > 0) {
    part(`${a.wins}G ${a.losses}M %${Math.round((a.wins * 100) / (a.wins + a.losses))}`, "record");
  }
  if (a.level) part(`Seviye ${a.level}`);
  if (a.game === "client") part(GAME_NAMES.client);
  if (a.hotkey) part(hotkeyLabel(a.hotkey), "key");
  if (a.note) part(a.note, "note");

  // The face of the account: its profile icon when we have the picture,
  // with the crest set into its corner; the crest alone otherwise.
  const portrait = document.createElement("span");
  portrait.className = "portrait";
  const picture = a.icon != null && iconPictures.get(a.icon);
  if (picture) {
    const img = document.createElement("img");
    img.src = picture;
    img.alt = "";
    img.draggable = false;
    portrait.classList.add("pictured");
    portrait.append(img);
  }
  portrait.append(crest(rank));
  go.append(portrait, who, standing, detail);

  const acts = document.createElement("span");
  acts.className = "acts";
  const games = a.game === "both" ? ["lol", "valorant"] : [];
  for (const game of games) {
    const alt = document.createElement("button");
    alt.type = "button";
    alt.className = "alt open";
    alt.textContent = game === "lol" ? "League" : "VALORANT";
    alt.title = `${GAME_NAMES[game]} ile aç`;
    alt.disabled = busy !== null;
    alt.addEventListener("click", () => switchTo(a, game));
    acts.append(alt);
  }
  if (!games.length) {
    const open = document.createElement("button");
    open.type = "button";
    open.className = "open";
    if (active && a.game === "client") open.append(asWritten("Riot Client"), "'ı aç");
    else open.textContent = active ? "Oyunu aç" : "Hesaba geç";
    open.disabled = busy !== null;
    open.addEventListener("click", () => switchTo(a, rowGame(a)));
    acts.append(open);
  }
  const more = document.createElement("button");
  more.type = "button";
  more.className = "more";
  more.textContent = "⋯";
  more.title = "Hesap ayarları";
  more.setAttribute("aria-label", `${a.name} ayarları`);
  more.addEventListener("click", () => openEdit(a.id));
  acts.append(more);

  li.append(go, acts);
  li.addEventListener("contextmenu", (e) => openMenu(e, a));
  return li;
}

function tag(text, warn = false) {
  const t = document.createElement("span");
  t.className = warn ? "tag warn" : "tag";
  t.textContent = text;
  return t;
}

async function refresh() {
  if (drag) return; // redrawing would drop the row out of the user's hand
  try {
    view = await invoke("get_state");
  } catch (e) {
    say(errorText(e), true);
    return;
  }
  render();
  if (editing) fillEdit();
  if (wizard) wizardTick();
  if (!$("view-security").hidden) fillSecurity();
  if (!$("view-settings").hidden) fillSettings();
}

// ---- the track: what a switch is doing -----------------------------------------
// The app reports each step of a switch and for which account. They are
// drawn as stations on a line along the bottom of the window: reached ones
// behind, the current one lit, the rest ahead. The sign-in is followed long
// after the switch command itself has returned, and a switch can also start
// from a hotkey or the tray, so the track listens rather than being told.

const STATIONS = {
  closing: ["Kapat", "Riot Client kapatılıyor"],
  restoring: ["Oturum", "Kayıtlı oturum yerine konuyor"],
  starting: ["Aç", "Riot Client açılıyor"],
  waiting: ["Giriş", "Riot'un oturumu kabul etmesi bekleniyor"],
  signing_in: ["Şifre", "Kayıtlı şifreyle giriş yapılıyor"],
  game: ["Oyun", "Giriş yapıldı, oyun açılıyor"],
};
const ROUTE = ["closing", "restoring", "starting", "waiting", "game"];
let track = null; // { account, route, at, started, timer, verifying, over }

function drawTrack() {
  const stage = $("stage");
  stage.classList.toggle("tracking", !!track);
  $("track").hidden = !track;
  if (!track) return;
  const account = view.accounts.find((a) => a.id === track.account);
  $("track-name").textContent = account ? account.name : "Hesap değiştiriliyor";
  $("track").className = track.over || "";
  $("track-text").textContent = track.over === "done" ? "Giriş yapıldı"
    : track.verifying ? "Riot doğrulama kodu istiyor; kodu Riot Client'ta gir"
    : track.over === "failed" ? "Tamamlanmadı"
    : STATIONS[track.route[track.at]][1];
  $("track-stations").replaceChildren(...track.route.map((step, i) => {
    const li = document.createElement("li");
    const reached = i < track.at || track.over === "done";
    if (reached) li.className = "reached";
    else if (i === track.at) li.className = track.over === "failed" && !track.verifying ? "failed" : "now";
    li.append(document.createElement("i"), STATIONS[step][0]);
    return li;
  }));
  $("track-cancel").hidden = !!track.over;
  $("track-close").hidden = !track.over;
  $("track-time").textContent = `${Math.round((Date.now() - track.started) / 1000)} sn`;
}

function endTrack() {
  if (track) clearInterval(track.timer);
  track = null;
  drawTrack();
}

listen("switch-step", (e) => {
  const { step, account } = e.payload;
  if (step === "cancelled") return endTrack();
  if (step === "done" || step === "stopped") {
    if (!track) return;
    clearInterval(track.timer);
    track.over = step === "done" ? "done" : "failed";
    drawTrack();
    // A switch that went through needs no reading; one that did not stays.
    if (step === "done") {
      const finished = track;
      setTimeout(() => { if (track === finished) endTrack(); }, 2500);
    }
    return;
  }
  if (step === "verifying") {
    if (track) track.verifying = true;
    return drawTrack();
  }
  if (!(step in STATIONS)) return;
  // A new switch starts a new line, from the station it begins at: one for
  // an account that is already signed in has only the game to open.
  if (!track || track.over || track.account !== account) {
    if (track) clearInterval(track.timer);
    const from = Math.max(ROUTE.indexOf(step), 0);
    track = { account, route: ROUTE.slice(from), at: 0, started: Date.now(), verifying: false, over: null };
    track.timer = setInterval(drawTrack, 1000);
  }
  // The password station appears only on switches that come to need it.
  if (step === "signing_in" && !track.route.includes(step)) track.route.splice(track.route.length - 1, 0, step);
  track.at = Math.max(track.route.indexOf(step), track.at);
  drawTrack();
});

$("track-cancel").addEventListener("click", () => invoke("cancel_switch").catch(() => {}));
$("track-close").addEventListener("click", endTrack);

// `game` opens that game this once instead of the account's own. `force`
// is the user's "switch anyway" after an unreadable League client state.
async function switchTo(a, game = null, force = false) {
  if (busy) return;
  busy = a.id;
  say("");
  render();
  try {
    await invoke("switch_account", { id: a.id, game, force });
    $("filter").value = "";
  } catch (e) {
    reportSwitchError(e, a.id, game);
  } finally {
    busy = null;
    await refresh();
  }
}

// A switch refused because a match is on can be queued instead; one
// refused because the app is locked continues after unlocking.
function reportSwitchError(e, id, game = null) {
  if (errorKind(e) === "locked") return openUnlock(id);
  if (errorKind(e) === "league_unknown") {
    const a = view.accounts.find((x) => x.id === id);
    return say(errorText(e), true, a && { label: "Yine de geç", run: () => { say(""); switchTo(a, game, true); } });
  }
  if (errorKind(e) !== "in_game") return say(errorText(e), true);
  say(`${errorText(e)} Oyun kapanınca geçilsin mi?`, true, {
    label: "Kapanınca geç",
    run: async () => {
      try {
        await invoke("queue_switch", { id });
        say("");
      } catch (err) {
        say(errorText(err), true);
      }
      refresh();
    },
  });
}

// ---- reordering by drag ---------------------------------------------------------
// Press on a row and move: past a few pixels it becomes a drag (and the
// press no longer counts as a click). Done with pointer events, because
// the app window reserves the browser's own drag-and-drop for files.

const DRAG_START = 6; // px of movement before a press becomes a drag

function startDrag(e, li) {
  // Dragging rearranges the stored order, so it needs that order on screen.
  if (e.button !== 0 || busy || sortBy !== "manual" || statusFilter !== "all") return;
  const rows = [...$("accounts").querySelectorAll(".account")];
  if (rows.length < 2) return;
  const from = rows.indexOf(li);
  drag = { li, rows, from, to: from, startX: e.clientX, startY: e.clientY, active: false, slots: [] };
}

// Works the same for a column of rows and a grid of cards: every account
// has a slot (where it sat when the drag began); the held one follows the
// pointer, and the others move to the slots they would have if it were
// dropped on the slot nearest to it.
window.addEventListener("pointermove", (e) => {
  if (!drag) return;
  const dx = e.clientX - drag.startX;
  const dy = e.clientY - drag.startY;
  if (!drag.active) {
    if (Math.hypot(dx, dy) < DRAG_START) return;
    drag.active = true;
    drag.slots = drag.rows.map((r) => {
      const b = r.getBoundingClientRect();
      return { x: b.left, y: b.top, cx: b.left + b.width / 2, cy: b.top + b.height / 2 };
    });
    $("accounts").classList.add("reordering");
    drag.li.classList.add("dragging");
  }
  drag.li.style.transform = `translate(${dx}px, ${dy}px)`;
  const mine = drag.slots[drag.from];
  const at = { x: mine.cx + dx, y: mine.cy + dy };
  let best = drag.from;
  let bestDistance = Infinity;
  drag.slots.forEach((slot, i) => {
    const d = Math.hypot(slot.cx - at.x, slot.cy - at.y);
    if (d < bestDistance) { bestDistance = d; best = i; }
  });
  drag.to = best;
  drag.rows.forEach((row, i) => {
    if (row === drag.li) return;
    let slot = i;
    if (drag.from < drag.to && i > drag.from && i <= drag.to) slot = i - 1;
    if (drag.from > drag.to && i >= drag.to && i < drag.from) slot = i + 1;
    const move = drag.slots[slot];
    const home = drag.slots[i];
    row.style.transform = slot === i ? "" : `translate(${move.x - home.x}px, ${move.y - home.y}px)`;
  });
});

function endDrag() {
  if (!drag) return;
  const { active, rows, from, to } = drag;
  drag = null;
  for (const row of rows) row.style.transform = "";
  $("accounts").classList.remove("reordering");
  rows[from].classList.remove("dragging");
  if (!active) return;
  // The release would otherwise count as a click on the account.
  // The browser sends that click right after this event; if the pointer
  // was released elsewhere none comes, so the guard is dropped a tick later.
  const swallow = (ev) => { ev.stopPropagation(); ev.preventDefault(); };
  window.addEventListener("click", swallow, { capture: true, once: true });
  setTimeout(() => window.removeEventListener("click", swallow, { capture: true }), 0);
  if (to === from) return;
  const ids = rows.map((r) => r.dataset.id);
  ids.splice(to, 0, ids.splice(from, 1)[0]);
  selected = to;
  saveVisibleOrder(ids);
}

window.addEventListener("pointerup", endDrag);
window.addEventListener("pointercancel", endDrag);
window.addEventListener("blur", endDrag);

// `ids` is the new order of the rows on screen, which may be a filtered
// subset: they keep the places their accounts held in the full list.
async function saveVisibleOrder(ids) {
  const shown = new Set(ids);
  const slots = view.accounts.map((a, i) => (shown.has(a.id) ? i : -1)).filter((i) => i >= 0);
  const next = view.accounts.slice();
  slots.forEach((slot, k) => { next[slot] = view.accounts.find((a) => a.id === ids[k]); });
  view.accounts = next;
  render();
  try {
    await invoke("reorder_accounts", { ids: next.map((a) => a.id) });
  } catch (err) {
    say(errorText(err), true);
  }
  refresh();
}

// Alt+arrows move the selected account, as dragging does.
function moveSelected(e, accounts) {
  e.preventDefault();
  const to = selected + (e.key === "ArrowUp" ? -1 : 1);
  if (to < 0 || to >= accounts.length) return;
  const ids = accounts.map((a) => a.id);
  ids.splice(to, 0, ids.splice(selected, 1)[0]);
  selected = to;
  saveVisibleOrder(ids);
}

$("filter").addEventListener("input", () => {
  selected = 0;
  render();
});

document.addEventListener("keydown", (e) => {
  if ($("view-list").hidden || e.target !== $("filter")) return;
  const accounts = visibleAccounts();
  if (e.altKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) return void moveSelected(e, accounts);
  if (e.key === "ArrowDown") selected = Math.min(selected + 1, accounts.length - 1);
  else if (e.key === "ArrowUp") selected = Math.max(selected - 1, 0);
  else if (e.key === "Enter" && accounts[selected]) {
    // For an account that plays both, Shift+Enter opens VALORANT.
    const a = accounts[selected];
    return void switchTo(a, e.shiftKey && a.game === "both" ? "valorant" : rowGame(a));
  }

  else return;
  e.preventDefault();
  render();
});

$("queue-cancel").addEventListener("click", async () => {
  await invoke("cancel_queue");
  refresh();
});

// ---- hotkey recording ----------------------------------------------------------
// Turns a key press into a shortcut such as "Ctrl+Shift+Digit1", or explains
// why it cannot be one. Ctrl+Alt is AltGr on many keyboards (Turkish among
// them): if the press types a character, a global shortcut would stop that
// character from being typed anywhere, so it is refused.

function hotkeyFromEvent(e) {
  if (!/^(Key[A-Z]|Digit\d|Numpad\d|F([1-9]|1[0-2])|Space)$/.test(e.code)) return null; // a modifier alone, or a key we do not offer
  if (!e.ctrlKey && !e.altKey) return { error: "Kısayolda Ctrl ya da Alt olmalı." };
  if (e.ctrlKey && e.altKey && e.key && e.key.length === 1) {
    const plain = e.code.startsWith("Key") ? e.code.slice(3).toLowerCase() : e.code.replace(/^(Digit|Numpad)/, "");
    if (e.key.toLowerCase() !== plain) {
      return { error: `Ctrl+Alt+${plain.toUpperCase()} klavyende "${e.key}" yazıyor; kısayol yapılırsa o karakteri yazamazsın. Shift ekle ya da başka bir tuş seç.` };
    }
  }
  const parts = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  parts.push(e.code);
  return { hotkey: parts.join("+") };
}

// ---- settings --------------------------------------------------------------------

function setSay(text, isError = false) {
  $("set-message").className = isError ? "hint error" : "hint";
  $("set-message").textContent = text;
}

function fillSettings() {
  const update = view.update || {};
  $("set-version").textContent = view.version || "";
  const pill = $("set-update-state");
  pill.className = "pill";
  if (!update.supported) {
    pill.textContent = "";
    $("set-update-text").textContent = "Bu sürüm kendini güncelleyemez. Yeni sürümleri deponun Releases sayfasından indirip üstüne kurabilirsin.";
  } else if (update.available) {
    pill.textContent = "Yeni sürüm";
    pill.classList.add("new");
    $("set-update-text").textContent = `${update.available} hazır. Güncelle'ye basınca indirilir, imzası doğrulanır ve uygulama yeniden açılır.`;
  } else {
    pill.textContent = "Güncel";
    pill.classList.add("good");
    $("set-update-text").textContent = update.enabled
      ? "Uygulama açılışta ve birkaç saatte bir yeni sürüme bakar; hiçbir şey sen onaylamadan kurulmaz."
      : "Kendiliğinden denetim kapalı. İstediğinde aşağıdan denetleyebilirsin.";
  }
  $("set-update-actions").hidden = !update.supported;
  $("set-update-auto-row").hidden = !update.supported;
  $("set-update-install").hidden = !update.available;
  $("set-update-auto").checked = !!update.enabled;
  $("set-autostart").checked = !!view.autostart;
  if (document.activeElement !== $("set-window-hotkey")) $("set-window-hotkey").value = hotkeyLabel(view.window_hotkey || "");
}

$("settings").addEventListener("click", () => {
  setSay("");
  fillSettings();
  show("settings");
});
// Leaving by the rail closes whatever was open on the accounts' side.
$("rail").addEventListener("click", (e) => {
  if (!e.target.closest(".nav")) return;
  wizard = null;
  editing = null;
}, { capture: true });

// The rail's first entry leads back to the accounts from anywhere.
document.querySelector('#rail .nav[data-view="list"]').addEventListener("click", () => {
  show("list");
  $("filter").focus();
  refresh();
});

$("set-security").addEventListener("click", () => {
  secSay("");
  $("sec-wipe").textContent = "Tüm verileri sil";
  fillSecurity();
  show("security");
});

async function setCall(command, args, okText = "") {
  try {
    await invoke(command, args);
    setSay(okText);
  } catch (e) {
    setSay(errorText(e), true);
  }
  await refresh();
}

$("set-autostart").addEventListener("change", (e) => setCall("set_autostart", { enabled: e.target.checked }));
$("set-update-auto").addEventListener("change", (e) => setCall("set_update_checks", { enabled: e.target.checked }));
$("set-update-check").addEventListener("click", (e) =>
  whileWorking(e.currentTarget, async () => {
    try {
      const found = await invoke("check_update_now");
      setSay(found ? "" : "Kurulu sürüm güncel.");
    } catch (err) {
      setSay(errorText(err), true);
    }
    await refresh();
  }));
$("set-update-install").addEventListener("click", () => $("update-install").click());

$("set-window-hotkey").addEventListener("keydown", (e) => {
  if (e.key === "Tab") return;
  e.preventDefault();
  const r = hotkeyFromEvent(e);
  if (!r) return;
  if (r.error) return setSay(r.error, true);
  setCall("set_window_hotkey", { hotkey: r.hotkey }, "Kısayol kaydedildi.").then(() => {
    if ($("set-message").className === "hint") $("set-window-hotkey").value = hotkeyLabel(r.hotkey);
  });
});
$("set-window-hotkey").addEventListener("blur", fillSettings);

// ---- one account's settings -------------------------------------------------

const editAccount = () => view.accounts.find((a) => a.id === editing);

function openEdit(id) {
  editing = id;
  $("edit-message").textContent = "";
  $("edit-username").value = "";
  $("edit-password").value = "";
  $("edit-remove").textContent = "Hesabı sil";
  $("edit-name").value = editAccount().name;
  $("edit-note").value = editAccount().note || "";
  $("edit-riot-id").value = editAccount().riot_id || "";
  $("edit-region").value = editAccount().region || "";
  $("edit-copy-master").value = "";
  fillEdit();
  show("edit");
}

// Fields the user is not typing in are kept in step with the stored state.
function fillEdit() {
  const a = editAccount();
  if (!a) return closeEdit();
  $("edit-title").textContent = a.name;
  $("edit-game").value = a.game;
  $("edit-hotkey").value = a.hotkey ? hotkeyLabel(a.hotkey) : "";
  // Saved by an older version, a plain Ctrl+Alt+key may be AltGr.
  if (a.hotkey && /^Ctrl\+Alt\+(Key|Digit)/.test(a.hotkey) && !$("edit-message").textContent) {
    $("edit-message").className = "hint error";
    $("edit-message").textContent = "Ctrl+Alt ile başlayan kısayollar Türkçe klavyede AltGr ile aynıdır ve #, @, { gibi karakterleri yazmanı engelleyebilir. Kutuya tıklayıp Shift ekleyerek yeniden kaydetmeni öneririm.";
  }
  $("edit-hotkey-clear").disabled = !a.hotkey;
  $("edit-cred-state").textContent = a.has_password
    ? "Şifre kayıtlı. Oturumun süresi dolarsa otomatik yazılır."
    : "Şifre kaydedersen, oturumun süresi dolduğunda giriş otomatik yapılır. Şifre bu bilgisayarda şifreli saklanır.";
  $("edit-cred-clear").hidden = !a.has_password;
  $("edit-copy").hidden = !a.has_password;
  const net = view.net || {};
  const ranksOn = !!(net.decided && net.online && net.lol_ranks);
  $("edit-lookup-state").textContent = a.game === "valorant"
    ? "Bu hesap yalnızca VALORANT oynuyor; League rankı aranmaz."
    : !ranksOn ? "Güncel rank kapalı. İnternet bölümünden açabilirsin."
    : a.lookup_problem === "not_found" ? "OP.GG'de bu Riot ID bu bölgede bulunamadı. İkisini de denetle."
    : a.rank_at && rankSource(a) ? `Rank: ${rankSource(a)}.`
    : "Riot ID ve bölge biliniyorsa rank kendiliğinden alınır. Hesaba bir kez giriş yapınca ikisi de öğrenilir; istersen elle de yazabilirsin.";
  $("edit-rank-refresh").disabled = !ranksOn || a.game === "valorant";
  $("edit-profile").disabled = !a.riot_id;
  $("edit-copy-master").hidden = !view.security.lock_enabled;
}

function closeEdit() {
  editing = null;
  show("list");
  $("filter").focus();
}

async function editCall(command, args, okText = "") {
  try {
    await invoke(command, { id: editing, ...args });
    $("edit-message").className = "hint";
    $("edit-message").textContent = okText;
  } catch (e) {
    $("edit-message").className = "hint error";
    $("edit-message").textContent = errorText(e);
  }
  await refresh();
}

$("edit-back").addEventListener("click", closeEdit);

$("edit-name").addEventListener("change", (e) => {
  const name = e.target.value.trim();
  if (name && name !== editAccount().name) editCall("rename_account", { name });
});

$("edit-lookup-save").addEventListener("click", () =>
  editCall("set_lookup", { riotId: $("edit-riot-id").value, region: $("edit-region").value }, "Kaydedildi."));

$("edit-rank-refresh").addEventListener("click", (e) => whileWorking(e.currentTarget, async () => {
  try {
    const done = await invoke("refresh_ranks", { id: editing });
    $("edit-message").className = "hint";
    $("edit-message").textContent = refreshText(done);
  } catch (err) {
    $("edit-message").className = "hint error";
    $("edit-message").textContent = errorText(err);
  }
  await refresh();
}));

$("edit-profile").addEventListener("click", () => editCall("open_profile", {}, ""));

$("edit-note").addEventListener("change", (e) => {
  if (e.target.value.trim() !== (editAccount().note || "")) editCall("set_note", { note: e.target.value }, "Not kaydedildi.");
});

$("edit-copy-go").addEventListener("click", async (e) => {
  const master = view.security.lock_enabled ? $("edit-copy-master").value : null;
  await whileWorking(e.currentTarget, async () => {
    try {
      const seconds = await invoke("copy_password", { id: editing, master });
      $("edit-message").className = "hint";
      $("edit-message").textContent = `Şifre panoda. ${seconds} saniye sonra panodan silinecek.`;
    } catch (err) {
      $("edit-message").className = "hint error";
      $("edit-message").textContent = errorText(err);
    }
  });
  $("edit-copy-master").value = "";
});

$("edit-game").addEventListener("change", (e) => editCall("set_game", { game: e.target.value }));

// Pressing a combination in the hotkey box records it.
$("edit-hotkey").addEventListener("keydown", (e) => {
  if (e.key === "Tab") return;
  e.preventDefault();
  if (e.key === "Backspace" || e.key === "Delete") return void editCall("set_hotkey", { hotkey: null });
  const r = hotkeyFromEvent(e);
  if (!r) return;
  if (r.error) {
    $("edit-message").className = "hint error";
    $("edit-message").textContent = r.error;
    return;
  }
  editCall("set_hotkey", { hotkey: r.hotkey }, "Kısayol kaydedildi.");
});

$("edit-hotkey-clear").addEventListener("click", () => editCall("set_hotkey", { hotkey: null }));

$("edit-cred-save").addEventListener("click", async () => {
  const args = { id: editing, username: $("edit-username").value, password: $("edit-password").value };
  try {
    await withSeal("close", "Şifre şifreleniyor…", "Şifrelendi", () => invoke("set_credentials", args));
    $("edit-message").className = "hint";
    $("edit-message").textContent = "Şifre kaydedildi.";
  } catch (e) {
    $("edit-message").className = "hint error";
    $("edit-message").textContent = errorText(e);
  }
  await refresh();
  if ($("edit-message").className === "hint") {
    $("edit-username").value = "";
    $("edit-password").value = "";
  }
});

$("edit-cred-clear").addEventListener("click", () => editCall("clear_credentials", {}, "Kayıtlı şifre silindi."));

$("edit-shortcut").addEventListener("click", (e) =>
  whileWorking(e.currentTarget, () => editCall("create_shortcut", {}, "Kısayol masaüstüne eklendi.")));

$("edit-remove").addEventListener("click", async (e) => {
  if (e.target.textContent !== "Emin misin? Tekrar tıkla") {
    e.target.textContent = "Emin misin? Tekrar tıkla"; // first click arms, second deletes
    return;
  }
  await editCall("remove_account", {});
});

// ---- add-account wizard -------------------------------------------------------

function wizardStep(step) {
  wizard.step = step;
  for (const s of ["choose", "wait", "name", "done"]) $(`wiz-${s}`).hidden = s !== step;
  $("wiz-message").textContent = "";
}

function openWizard() {
  if (view.security.lock_enabled && !view.security.unlocked) return openUnlock(null);
  wizard = { step: "choose", mode: null };
  $("wiz-intro").textContent = view.accounts.length
    ? "Hesabı nasıl eklemek istersin?"
    : "Hoş geldin. İlk hesabını ekleyelim; nasıl eklemek istersin?";
  show("wizard");
  wizardStep("choose");
}

function closeWizard() {
  wizard = null;
  show("list");
  $("filter").focus();
  refresh();
}

function wizardFail(e) {
  $("wiz-message").textContent = errorText(e);
}

// Shows the naming step. With `mode` "session" the password is optional.
function wizardName(mode) {
  wizard.mode = mode;
  const riotId = view.client.riot_id || "";
  $("wiz-found").textContent = mode === "session" ? (riotId ? `Giriş yapıldı: ${riotId}` : "Giriş yapıldı.") : "";
  $("wiz-found").hidden = mode !== "session";
  $("wiz-name-input").value = mode === "session" ? riotId.split("#")[0] : "";
  $("wiz-game").value = "lol";
  $("wiz-username").value = "";
  $("wiz-pass").value = "";
  if (mode === "session") $("wiz-creds-legend").textContent = "Oturum dolarsa otomatik giriş için (isteğe bağlı)";
  else $("wiz-creds-legend").replaceChildren(asWritten("Riot"), " giriş bilgileri");
  wizardStep("name");
  $("wiz-name-input").focus();
}

// The wait step says "opening" until Riot Client is really up, then shows
// what to do in it.
function wizardWaitTitle(opening) {
  $("wiz-wait-title").textContent = opening ? "Riot Client açılıyor…" : "Riot Client açıldı.";
  $("wiz-wait-bar").hidden = !opening;
  $("wiz-wait-steps").hidden = opening;
  if (opening) {
    $("wiz-anyway").hidden = true;
    $("wiz-wait-status").className = "hint";
    $("wiz-wait-status").textContent = "Bu birkaç saniye sürebilir.";
  }
}

// Called on every state refresh while the wizard is open.
function wizardTick() {
  if (wizard.step !== "wait") return;
  const c = view.client;
  // Up means the client answers: until then its window may not even exist.
  const opening = wizard.opening || !c.running || c.login === "unknown";
  wizardWaitTitle(opening);
  if (opening) return;
  if (c.login === "signed_in" && view.unsaved_session) {
    // Riot Client's own report about the box is sometimes wrong, so it is
    // a warning with a way through, never a dead end.
    if (c.persist === false) {
      $("wiz-wait-status").textContent =
        "Giriş yapıldı. Riot Client \"Oturumu açık tut\" kutusunun işaretli olmadığını bildiriyor; bu bildirim bazen yanlış oluyor. Kutuyu işaretlediysen devam et.";
      $("wiz-wait-status").className = "hint";
      $("wiz-anyway").hidden = false;
      return;
    }
    return wizardName("session");
  }
  $("wiz-anyway").hidden = true;
  $("wiz-wait-status").className = "hint";
  $("wiz-wait-status").textContent =
    c.login === "signed_in" ? "Bu hesap zaten kayıtlı. Başka bir hesapla giriş yap." : "Giriş bekleniyor…";
}

$("wiz-session").addEventListener("click", async () => {
  // A session that is already open and unsaved can be saved straight away.
  if (view.unsaved_session) return wizardName("session");
  // Show the wait step at once: closing the running client and starting
  // a fresh one takes a few seconds, and the press must be acknowledged.
  wizard.opening = true;
  wizardStep("wait");
  wizardWaitTitle(true);
  try {
    await invoke("begin_add");
    wizard.opening = false;
    refresh();
  } catch (e) {
    wizard.opening = false;
    wizardStep("choose");
    wizardFail(e);
  }
});

$("wiz-password").addEventListener("click", () => wizardName("password"));
$("wiz-anyway").addEventListener("click", () => {
  $("wiz-anyway").hidden = true;
  wizardName("session");
});

$("wiz-save").addEventListener("click", async () => {
  const name = $("wiz-name-input").value;
  const game = $("wiz-game").value;
  const username = $("wiz-username").value.trim();
  const password = $("wiz-pass").value;
  $("wiz-save").disabled = true;
  try {
    if (wizard.mode === "password") {
      await withSeal("close", "Şifre şifreleniyor…", "Şifrelendi", () =>
        invoke("add_with_password", { name, username, password, game }));
    } else {
      if (!!username !== !!password) throw { message: "Otomatik giriş için kullanıcı adı ve şifrenin ikisi de gerekli." };
      await withSeal("close", "Oturum şifreleniyor…", "Şifrelendi", async () => {
        const id = await invoke("capture_account", { name, game });
        if (username) await invoke("set_credentials", { id, username, password });
      });
    }
    $("wiz-done-text").textContent = `"${name.trim()}" kaydedildi.`;
    wizardStep("done");
  } catch (e) {
    wizardFail(e);
  } finally {
    $("wiz-save").disabled = false;
    $("wiz-pass").value = "";
    refresh();
  }
});

$("wiz-another").addEventListener("click", () => wizardStep("choose"));
$("wiz-finish").addEventListener("click", closeWizard);
$("wiz-close").addEventListener("click", closeWizard);
$("add").addEventListener("click", openWizard);
$("unsaved-save").addEventListener("click", () => {
  openWizard();
  wizardName("session");
});

// ---- unlock prompt -------------------------------------------------------------------

function openUnlock(thenSwitchTo) {
  afterUnlock = thenSwitchTo;
  $("unlock-password").value = "";
  $("unlock-message").textContent = "";
  show("unlock");
  $("unlock-password").focus();
}

$("unlock-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const password = $("unlock-password").value;
  $("unlock-password").value = "";
  try {
    await withSeal("open", "Kilit açılıyor…", "Açıldı", () => invoke("unlock", { password }));
  } catch (err) {
    $("unlock-message").textContent = errorText(err);
    return;
  }
  const next = view.accounts.find((a) => a.id === afterUnlock);
  afterUnlock = null;
  show("list");
  await refresh();
  if (next) switchTo(next);
});

$("unlock-back").addEventListener("click", () => {
  afterUnlock = null;
  show("list");
});
$("locked-open").addEventListener("click", () => openUnlock(null));

// ---- security screen -----------------------------------------------------------------

function fillSecurity() {
  const sec = view.security;
  // What is said about the network depends on what this build can do.
  const update = view.update || {};
  const net = view.net || {};
  const master = !net.decided || net.online;
  const uses = [];
  if (update.supported && update.enabled && master) uses.push("güncelleme denetimi");
  if (net.decided && net.online && net.lol_ranks) uses.push("League rankı");
  if (net.decided && net.online && net.icons) uses.push("profil simgeleri");
  $("sec-net").textContent = uses.length
    ? `Uygulama internete yalnızca şunlar için bağlanır: ${uses.join(", ")}. Şifreler, kullanıcı adları ve oturumlar hiçbir bağlantıda gönderilmez; kurulan her bağlantı İnternet bölümündeki kayıtta görünür. Bunun dışında yalnızca bu bilgisayardaki Riot Client ile konuşur.`
    : "Uygulama şu an internete bağlanmıyor. Yalnızca bu bilgisayardaki Riot Client ile konuşur.";

  $("sec-counts").textContent =
    `${view.accounts.length} hesap, ${sec.saved_sessions} kayıtlı oturum, ${sec.saved_passwords} kayıtlı şifre.`;
  $("sec-path").textContent = sec.data_dir || "";
  $("sec-lock-off").hidden = !!sec.lock_enabled;
  $("sec-lock-on").hidden = !sec.lock_enabled;
  $("sec-lock-now").disabled = !sec.unlocked;
  const autoLock = $("sec-auto-lock");
  const minutes = String(sec.auto_lock_minutes ?? 15);
  // A value set by hand in config.json still shows up as itself.
  if (![...autoLock.options].some((o) => o.value === minutes)) autoLock.add(new Option(`${minutes} dakika sonra`, minutes));
  autoLock.value = minutes;
  $("sec-lock-state").textContent = sec.lock_enabled
    ? "Açık. Uygulama her başladığında, hesap değiştirmeden önce ana şifre sorulur."
    : "Kapalı. Açarsan kayıtlı oturumlar ve şifreler ayrıca bir ana şifreyle şifrelenir; uygulama her başladığında bir kez sorulur. Ana şifre unutulursa kurtarılamaz.";
}

function secSay(text, isError = false) {
  $("sec-message").className = isError ? "hint error" : "hint";
  $("sec-message").textContent = text;
}

// With `sealText`, the padlock closes over the window while it runs.
async function secCall(command, args, okText, sealText = null) {
  try {
    if (sealText) await withSeal("close", sealText, "Kilitlendi", () => invoke(command, args));
    else await invoke(command, args);
    secSay(okText);
  } catch (e) {
    secSay(errorText(e), true);
  }
  for (const id of ["sec-new", "sec-new2", "sec-current"]) $(id).value = "";
  await refresh();
}


$("sec-open").addEventListener("click", (e) => whileWorking(e.currentTarget, () => secCall("open_data_dir", {}, "")));
$("sec-enable").addEventListener("click", () => {
  if ($("sec-new").value !== $("sec-new2").value) return secSay("İki şifre aynı değil.", true);
  const password = $("sec-new").value;
  secCall("set_lock", { current: null, new: password }, "Uygulama kilidi etkin.", "Oturumlar ve şifreler kilitleniyor…");
});
$("sec-disable").addEventListener("click", (e) =>
  whileWorking(e.currentTarget, () =>
    secCall("set_lock", { current: $("sec-current").value, new: null }, "Uygulama kilidi kaldırıldı.")));
$("sec-auto-lock").addEventListener("change", (e) => {
  const minutes = Number(e.target.value);
  secCall("set_auto_lock", { minutes }, minutes ? `Kullanılmayınca ${e.target.selectedOptions[0].text.replace(" sonra", "")} içinde kilitlenir.` : "Kendiliğinden kilitlenme kapalı.");
});
$("sec-lock-now").addEventListener("click", async () => {
  await withSeal("close", "Kilitleniyor…", "Kilitlendi", () => invoke("lock_now"));
  show("list");
  refresh();
});
$("update-install").addEventListener("click", (e) =>
  whileWorking(e.currentTarget, async () => {
    say("Güncelleme indiriliyor…");
    try {
      await invoke("install_update"); // the app restarts on success
    } catch (err) {
      say(errorText(err), true);
    }
  }));
$("sec-wipe").addEventListener("click", async (e) => {
  if (e.target.textContent !== "Emin misin? Geri alınamaz. Tekrar tıkla") {
    e.target.textContent = "Emin misin? Geri alınamaz. Tekrar tıkla"; // first click arms
    return;
  }
  e.target.textContent = "Tüm verileri sil";
  await secCall("wipe_all", {}, "Tüm veriler silindi.");
});

// ---- events from the app --------------------------------------------------------

listen("state-changed", refresh);
listen("update-progress", (e) => {
  const p = e.payload;
  say(p.installing ? "Kuruluyor, uygulama birazdan yeniden açılacak…"
    : p.percent == null ? "Güncelleme indiriliyor…"
    : `Güncelleme indiriliyor: %${p.percent}`);
});
listen("notice", (e) => {
  const n = e.payload;
  if (n.kind === "click_riot") return say(n.text);
  if (n.kind === "click_riot_done") return say("");
  if (["in_game", "locked", "league_unknown"].includes(n.kind) && n.account) reportSwitchError({ kind: n.kind, message: n.text }, n.account);
  else say(n.text, n.error);
});
window.addEventListener("focus", () => {
  if (!$("view-list").hidden) $("filter").focus();
  refresh();
});
// ---- moving accounts in and out -----------------------------------------------

function dataSay(text, isError = false) {
  $("data-message").className = isError ? "hint error" : "hint";
  $("data-message").textContent = text;
  if (text) $("data-message").scrollIntoView({ block: "nearest" });
}

// "3 hesap eklendi, 2 zaten kayıtlıydı" from an import's numbers; `verb`
// is "eklendi" for what was done and "eklenecek" for a preview.
function importText(done, verb) {
  const parts = [`${done.added} hesap ${verb}`];
  if (done.already_here) parts.push(`${done.already_here} zaten kayıtlı`);
  if (done.incomplete) parts.push(`${done.incomplete} satır ya da kayıt eksik`);
  return parts.join(", ");
}

// The master password box is only there when the app lock is on.
const dataMaster = () => (view.security.lock_enabled ? $("data-master").value : null);

function openData() {
  dataSay("");
  for (const id of ["data-master", "data-backup-pass", "data-backup-pass2", "data-restore-pass", "data-bulk"]) $(id).value = "";
  $("data-master").hidden = !view.security.lock_enabled;
  $("data-restore-confirm").hidden = true;
  $("data-bulk-preview").textContent = "";
  $("data-bulk-go").disabled = true;
  $("data-plain-ok").checked = false;
  $("data-plain").disabled = true;
  show("data");
}

$("set-data").addEventListener("click", openData);
$("data-backup").addEventListener("click", (e) => whileWorking(e.currentTarget, async () => {
  const password = $("data-backup-pass").value;
  if (password !== $("data-backup-pass2").value) return dataSay("İki yedek parolası aynı değil.", true);
  try {
    const path = await withSeal("close", "Yedek şifreleniyor…", "Şifrelendi", () => invoke("export_backup", { master: dataMaster(), password }));
    dataSay(`Yedek alındı: ${path}`);
    for (const id of ["data-master", "data-backup-pass", "data-backup-pass2"]) $(id).value = "";
  } catch (err) {
    dataSay(errorText(err), true);
  }
}));

// A backup is opened and counted first; it is added only after a second yes.
let restoreFile = null;
$("data-restore").addEventListener("click", () => $("data-restore-file").click());
$("data-restore-file").addEventListener("change", async (e) => {
  const file = e.target.files[0];
  e.target.value = "";
  if (!file) return;
  if (file.size > 5 * 1024 * 1024) return dataSay("Bu dosya bir yedek olamayacak kadar büyük.", true);
  restoreFile = await file.text();
  try {
    const preview = await invoke("import_backup", { file: restoreFile, password: $("data-restore-pass").value, dryRun: true });
    $("data-restore-preview").textContent = `Bu yedekten ${importText(preview, "eklenecek")}.`;
    $("data-restore-go").disabled = !preview.added;
    $("data-restore-confirm").hidden = false;
    dataSay("");
  } catch (err) {
    restoreFile = null;
    $("data-restore-confirm").hidden = true;
    dataSay(errorText(err), true);
  }
});
$("data-restore-cancel").addEventListener("click", () => {
  restoreFile = null;
  $("data-restore-confirm").hidden = true;
});
$("data-restore-go").addEventListener("click", (e) => whileWorking(e.currentTarget, async () => {
  try {
    const done = await invoke("import_backup", { file: restoreFile, password: $("data-restore-pass").value, dryRun: false });
    dataSay(`${importText(done, "eklendi")}. Şifresi olmayan hesaplara bir kez elle giriş yapman gerekir.`);
  } catch (err) {
    dataSay(errorText(err), true);
  }
  restoreFile = null;
  $("data-restore-pass").value = "";
  $("data-restore-confirm").hidden = true;
  await refresh();
}));

// Pasted lines are counted as they are typed, so what "Ekle" will do is known before it is pressed.
let bulkTimer = null;
$("data-bulk").addEventListener("input", () => {
  clearTimeout(bulkTimer);
  $("data-bulk-go").disabled = true;
  bulkTimer = setTimeout(async () => {
    const lines = $("data-bulk").value;
    if (!lines.trim()) return void ($("data-bulk-preview").textContent = "");
    try {
      const preview = await invoke("bulk_add", { lines, dryRun: true });
      $("data-bulk-preview").textContent = `${importText(preview, "eklenecek")}.`;
      $("data-bulk-go").disabled = !preview.added;
    } catch (err) {
      $("data-bulk-preview").textContent = errorText(err);
    }
  }, 300);
});
$("data-bulk-go").addEventListener("click", async (e) => {
  const button = e.currentTarget;
  await whileWorking(button, async () => {
    try {
      const done = await withSeal("close", "Şifreler şifreleniyor…", "Şifrelendi", () => invoke("bulk_add", { lines: $("data-bulk").value, dryRun: false }));
      dataSay(`${importText(done, "eklendi")}.`);
      $("data-bulk").value = "";
      $("data-bulk-preview").textContent = "";
    } catch (err) {
      dataSay(errorText(err), true);
    }
    await refresh();
  });
  // Whatever is in the box now has to be counted again before it can be added.
  button.disabled = true;
});

// The file is read here and its text handed to the app; nothing of it is
// kept in the window afterwards.
$("sec-import").addEventListener("click", () => $("sec-import-file").click());
$("sec-import-file").addEventListener("change", async (e) => {
  const file = e.target.files[0];
  e.target.value = "";
  if (!file) return;
  if (file.size > 5 * 1024 * 1024) return dataSay("Bu dosya bir hesap listesi olamayacak kadar büyük.", true);
  try {
    const done = await invoke("import_league_accounts", { json: await file.text() });
    dataSay(`${importText(done, "eklendi")}. Dışa aktarma dosyasını silmeyi unutma.`);
  } catch (err) {
    dataSay(errorText(err), true);
  }
  await refresh();
});

$("data-plain-ok").addEventListener("change", (e) => {
  $("data-plain").disabled = !e.target.checked;
});
$("data-plain").addEventListener("click", async (e) => {
  const button = e.currentTarget;
  await whileWorking(button, async () => {
    try {
      const path = await invoke("export_plain", { master: dataMaster(), region: $("data-plain-region").value });
      dataSay(`Açık şifreli dosya oluşturuldu: ${path}. İşin bitince bu dosyayı sil.`);
    } catch (err) {
      dataSay(errorText(err), true);
    }
  });
  // The warning is agreed to once per file, not once and for all.
  $("data-plain-ok").checked = false;
  button.disabled = true;
  $("data-master").value = "";
});

// ---- the internet --------------------------------------------------------------

// What a round of looking up ranks came to, in a sentence.
function refreshText(done) {
  if (done.paused && !(done.changed + done.same + done.not_found + done.failed)) {
    return "Şu an bakılamıyor: ya internet kapalı ya da OP.GG bir süre beklememizi istedi.";
  }
  const parts = [];
  if (done.changed) parts.push(`${done.changed} hesap güncellendi`);
  if (done.same) parts.push(`${done.same} hesapta değişiklik yok`);
  if (done.not_found) parts.push(`${done.not_found} Riot ID bulunamadı`);
  if (done.failed) parts.push(`${done.failed} hesaba bakılamadı`);
  if (done.skipped) parts.push(`${done.skipped} hesabın Riot ID'si ya da bölgesi bilinmiyor`);
  if (done.paused) parts.push("OP.GG beklememizi istedi, kalanı sonra");
  return parts.length ? `${parts.join(", ")}.` : "Bakılacak hesap yok.";
}

$("ranks-refresh").addEventListener("click", (e) => whileWorking(e.currentTarget, async () => {
  try {
    say(refreshText(await invoke("refresh_ranks", { id: null })));
  } catch (err) {
    say(errorText(err), true);
  }
  await refresh();
}));

// The state of the internet in a word or two.
function netWord(net) {
  if (!net.decided) return "?";
  if (!net.online) return "Kapalı";
  return net.lol_ranks || net.icons ? "Açık" : "Yalnız güncelleme";
}

// The question asked once: any answer is kept. A no leaves the update
// check as it has always been; only the İnternet page turns that off too.
async function answerNet(yes) {
  try {
    await invoke("set_net", { settings: { decided: true, online: true, lol_ranks: yes, icons: yes, refresh_minutes: 30 } });
  } catch (err) {
    say(errorText(err), true);
  }
  await refresh();
  if (yes) $("ranks-refresh").click();
}
$("net-yes").addEventListener("click", () => answerNet(true));
$("net-no").addEventListener("click", () => answerNet(false));
$("net-more").addEventListener("click", () => openNet());

const PURPOSES = { updates: "Güncelleme", lol_ranks: "League rankı", icons: "Profil simgesi" };
const OUTCOMES = { ok: "tamam", off: "kapalı olduğu için kurulmadı", refused: "izinli adres değil, reddedildi", too_large: "cevap çok büyük, atıldı", unreachable: "ulaşılamadı" };

function fillNet() {
  const net = view.net || {};
  // Before an answer only the update check runs, so that is what is shown.
  $("net-online").checked = !net.decided || !!net.online;
  $("net-ranks").checked = !!(net.decided && net.lol_ranks);
  $("net-icons").checked = !!(net.decided && net.icons);
  $("net-ranks").disabled = $("net-icons").disabled = $("net-every").disabled = !$("net-online").checked;
  const every = String(net.refresh_minutes || 30);
  if (![...$("net-every").options].some((o) => o.value === every)) $("net-every").add(new Option(`${every} dakikada bir`, every));
  $("net-every").value = every;
  const pill = $("net-pill");
  pill.className = net.decided && net.online ? "pill good" : "pill";
  pill.textContent = net.decided ? netWord(net) : "Henüz sorulmadı";
}

async function fillLedger() {
  let entries = [];
  try {
    entries = await invoke("net_ledger");
  } catch {}
  $("net-empty").hidden = entries.length > 0;
  $("net-ledger").hidden = !entries.length;
  $("net-ledger").tBodies[0].replaceChildren(...entries.slice(0, 120).map((entry) => {
    const tr = document.createElement("tr");
    const good = entry.outcome === "ok";
    if (!good) tr.className = entry.outcome === "off" ? "quiet" : "bad";
    const when = new Date(entry.at * 1000).toLocaleString("tr", { day: "2-digit", month: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit" });
    const outcome = OUTCOMES[entry.outcome] || (/^\d+$/.test(entry.outcome) ? `sunucu ${entry.outcome} dedi` : entry.outcome);
    // The updater does not say how much it read; nothing is better than "0 B".
    const size = good && entry.bytes ? (entry.bytes >= 1024 ? `${Math.round(entry.bytes / 1024)} KB` : `${entry.bytes} B`) : "";
    for (const text of [when, PURPOSES[entry.purpose] || entry.purpose, entry.host, outcome, size]) {
      const td = document.createElement("td");
      td.textContent = text;
      tr.append(td);
    }
    return tr;
  }));
}

function openNet() {
  $("net-message").textContent = "";
  fillNet();
  fillLedger();
  show("net");
}
$("nav-net").addEventListener("click", openNet);

async function saveNet() {
  const settings = {
    decided: true,
    online: $("net-online").checked,
    lol_ranks: $("net-ranks").checked,
    icons: $("net-icons").checked,
    refresh_minutes: Number($("net-every").value) || 30,
  };
  try {
    await invoke("set_net", { settings });
    $("net-message").className = "hint";
    $("net-message").textContent = settings.online ? "Kaydedildi." : "İnternet kapatıldı; uygulama artık hiçbir bağlantı kurmaz.";
  } catch (err) {
    $("net-message").className = "hint error";
    $("net-message").textContent = errorText(err);
  }
  await refresh();
  fillNet();
}
for (const id of ["net-online", "net-ranks", "net-icons", "net-every"]) $(id).addEventListener("change", saveNet);
$("net-reload").addEventListener("click", fillLedger);
$("net-clear").addEventListener("click", async () => {
  await invoke("clear_net_ledger").catch(() => {});
  fillLedger();
});

// ---- what can be done with one account, on a right click -----------------------

function closeMenu() {
  $("menu").hidden = true;
}

function openMenu(e, a) {
  e.preventDefault();
  if (busy) return;
  const net = view.net || {};
  const items = [];
  if (a.game === "both") {
    items.push(["League ile aç", () => switchTo(a, "lol")], ["VALORANT ile aç", () => switchTo(a, "valorant")]);
  } else {
    items.push([a.id === view.active ? "Oyunu aç" : "Hesaba geç", () => switchTo(a, rowGame(a))]);
  }
  if (net.decided && net.online && net.lol_ranks && a.game !== "valorant" && a.riot_id) {
    items.push(["Rankı yenile", async () => {
      try {
        say(refreshText(await invoke("refresh_ranks", { id: a.id })));
      } catch (err) {
        say(errorText(err), true);
      }
      refresh();
    }]);
  }
  if (a.riot_id) {
    items.push(["Riot ID'yi kopyala", async () => {
      try {
        await navigator.clipboard.writeText(a.riot_id);
        say("Riot ID kopyalandı.");
      } catch {
        say("Panoya yazılamadı.", true);
      }
    }]);
    items.push(["OP.GG'de aç", () => invoke("open_profile", { id: a.id }).catch((err) => say(errorText(err), true))]);
  }
  items.push(["Hesap ayarları", () => openEdit(a.id)]);

  const menu = $("menu");
  menu.replaceChildren(...items.map(([label, run]) => {
    const b = document.createElement("button");
    b.type = "button";
    b.setAttribute("role", "menuitem");
    b.textContent = label;
    b.addEventListener("click", () => { closeMenu(); run(); });
    return b;
  }));
  menu.hidden = false;
  // Kept inside the window whichever corner the click was in.
  const { width, height } = menu.getBoundingClientRect();
  menu.style.left = `${Math.min(e.clientX, window.innerWidth - width - 8)}px`;
  menu.style.top = `${Math.min(e.clientY, window.innerHeight - height - 8)}px`;
  menu.firstElementChild.focus();
}

window.addEventListener("click", (e) => { if (!e.target.closest("#menu")) closeMenu(); });
window.addEventListener("blur", closeMenu);
window.addEventListener("keydown", (e) => { if (e.key === "Escape") closeMenu(); });
// Nowhere else has a use for the browser's own menu.
window.addEventListener("contextmenu", (e) => { if (!e.target.closest("input, textarea, .log")) e.preventDefault(); });

// ---- the diagnostic log --------------------------------------------------------
// Each line of the file is a time in Unix seconds and a sentence. Shown
// with the time as a clock would say it, a rule above each new switch, and
// the lines that speak of something going wrong marked.

const LOG_TROUBLE = /did not|gave up|lost the window|refused|needs a manual|not possible|not confirmed|failed|could not|was stopped|discarded|asks for a verification/;
let logText = "";

function logLine(line) {
  const at = line.indexOf(" ");
  const seconds = Number(line.slice(0, at));
  const text = at > 0 && seconds ? line.slice(at + 1) : line;
  const when = seconds ? new Date(seconds * 1000) : null;
  const stamp = when ? when.toLocaleString("tr", { day: "2-digit", month: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit" }) : "";
  const li = document.createElement("li");
  if (text.startsWith("switch: client closed") || text.startsWith("switch: the client did not quit")) li.className = "start";
  if (LOG_TROUBLE.test(text)) li.classList.add("bad");
  const time = document.createElement("time");
  time.textContent = stamp;
  const body = document.createElement("span");
  const colon = text.indexOf(": ");
  if (colon > 0 && colon < 12) {
    const part = document.createElement("b");
    part.className = "part";
    part.textContent = text.slice(0, colon + 1);
    body.append(part, text.slice(colon + 1));
  } else {
    body.textContent = text;
  }
  li.append(time, body);
  return { li, plain: `${stamp}  ${text}` };
}

async function openLog() {
  show("log");
  $("log-message").className = "hint";
  $("log-message").textContent = "";
  let raw = "";
  try {
    raw = await invoke("read_log");
  } catch (e) {
    $("log-message").className = "hint error";
    $("log-message").textContent = errorText(e);
  }
  const lines = (raw || "").split("\n").filter((l) => l.trim()).map(logLine);
  logText = lines.map((l) => l.plain).join("\n");
  $("log-lines").replaceChildren(...lines.map((l) => l.li));
  $("log-lines").hidden = !lines.length;
  $("log-copy").disabled = !lines.length;
  if (!lines.length && !$("log-message").textContent) $("log-message").textContent = "Henüz kayıt yok. İlk hesap geçişinden sonra burada satırlar görünür.";
  // The newest lines are the ones asked about.
  const main = $("log-lines").parentElement;
  main.scrollTop = main.scrollHeight;
}

$("nav-log").addEventListener("click", openLog);
$("log-refresh").addEventListener("click", (e) => whileWorking(e.currentTarget, openLog));
$("log-bundle").addEventListener("click", (e) => whileWorking(e.currentTarget, async () => {
  try {
    const path = await invoke("save_diagnostics");
    $("log-message").className = "hint";
    $("log-message").textContent = `Tanı paketi yazıldı: ${path}. İçinde hesap adı, kullanıcı adı, şifre ya da oturum yok; olduğu gibi gönderebilirsin.`;
  } catch (err) {
    $("log-message").className = "hint error";
    $("log-message").textContent = errorText(err);
  }
}));

$("log-copy").addEventListener("click", async () => {
  try {
    await navigator.clipboard.writeText(logText);
  } catch {
    // Where the clipboard is not offered to the page, the old way still works.
    const box = document.createElement("textarea");
    box.value = logText;
    document.body.append(box);
    box.select();
    document.execCommand("copy");
    box.remove();
  }
  $("log-message").className = "hint";
  $("log-message").textContent = "Kayıt panoya kopyalandı.";
});

// Using the window counts as using the app, so the lock does not close
// itself under the user's hands. Told at most twice a minute.
let lastSeenHere = 0;
function stillHere() {
  if (!view.security.lock_enabled || !view.security.unlocked) return;
  const now = Date.now();
  if (now - lastSeenHere < 30000) return;
  lastSeenHere = now;
  invoke("still_here").catch(() => {});
}
window.addEventListener("keydown", stillHere, { capture: true, passive: true });
window.addEventListener("pointerdown", stillHere, { capture: true, passive: true });

// What Riot Client is doing shows up in its own state, not as an event:
// poll while the window is on screen, faster while the wizard is waiting.
setInterval(() => {
  if (document.visibilityState === "visible" && !busy) refresh();
}, 2000);

(async () => {
  show("list");
  await refresh();
  if (!view.accounts.length) openWizard(); // first run
  else $("filter").focus();
})();
