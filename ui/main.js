// Window logic. All real work happens in Rust; this file draws three views
// (account list, one account's settings, the add-account wizard) and
// forwards what the user does.

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

function show(name) {
  for (const v of ["list", "edit", "wizard", "unlock", "security", "settings"]) $(`view-${v}`).hidden = v !== name;
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

function visibleAccounts() {
  const q = $("filter").value.trim().toLocaleLowerCase("tr");
  return view.accounts.filter(
    (a) => plays(a, gameFilter) && `${a.name} ${a.riot_id || ""}`.toLocaleLowerCase("tr").includes(q),
  );
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
function say(text, isError = false, action = null) {
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

function hotkeyLabel(hotkey) {
  return hotkey.replace(/Key|Digit/g, "").replace("Numpad", "Num ");
}

function render() {
  renderGameFilter();
  const accounts = visibleAccounts();
  selected = Math.min(selected, Math.max(accounts.length - 1, 0));
  const rows = accounts.map((a, i) => row(a, i));
  if (!rows.length) {
    const empty = document.createElement("li");
    empty.className = "empty";
    empty.textContent = !view.accounts.length ? "Henüz hesap yok. \"Hesap ekle\" ile başla."
      : $("filter").value.trim() ? "Eşleşen hesap yok."
      : `Bu listede ${gameFilter === "lol" ? "League" : "VALORANT"} oynayan hesap yok.`;
    rows.push(empty);
  }
  // Rows animate in only when the set of accounts changes, not on the
  // periodic refresh.
  const ids = view.accounts.map((a) => a.id).sort().join("|");
  $("accounts").classList.toggle("arriving", ids !== shownIds);
  shownIds = ids;
  $("accounts").replaceChildren(...rows);
  $("footer-lock").classList.toggle("closed", !!view.security.lock_enabled);

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
  $("footer-lock").hidden = !view.security.lock_enabled;
}

function row(a, i) {
  const li = document.createElement("li");
  li.className = "account";
  li.dataset.id = a.id;
  li.style.setProperty("--i", i);
  li.addEventListener("pointerdown", (e) => startDrag(e, li));
  li.classList.toggle("active", a.id === view.active);
  li.classList.toggle("selected", i === selected);
  li.classList.toggle("busy", a.id === busy);

  const go = document.createElement("button");
  go.type = "button";
  go.className = "go";
  go.disabled = busy !== null;
  go.addEventListener("click", () => switchTo(a, rowGame(a)));

  const name = document.createElement("span");
  name.className = "name";
  const label = document.createElement("span");
  label.textContent = a.name;
  name.append(label);
  if (a.needs_login) name.append(tag("giriş gerekli", true));
  if (a.hotkey) name.append(tag(hotkeyLabel(a.hotkey)));

  const detail = document.createElement("span");
  detail.className = "detail";
  const part = (text, cls = "") => {
    const s = document.createElement("span");
    s.textContent = text;
    s.className = cls;
    detail.append(s);
  };
  if (a.id === view.active) part("Açık", "on");
  if (a.rank) part(a.rank, "rank");
  else if (a.level) part("Derecesiz");
  if (a.level) part(`Seviye ${a.level}`);
  if (a.game === "client") part(GAME_NAMES.client);
  if (a.riot_id) part(a.riot_id);
  if (!detail.childElementCount) part(a.has_session ? "Oturum kayıtlı" : "Şifreyle giriş");

  go.append(name, detail);

  const more = document.createElement("button");
  more.type = "button";
  more.className = "more";
  more.textContent = "⋯";
  more.title = "Ayarlar";
  more.addEventListener("click", () => openEdit(a.id));

  li.append(go);
  for (const game of rowGames(a)) {
    const alt = document.createElement("button");
    alt.type = "button";
    alt.className = "alt";
    alt.textContent = GAME_SHORT[game];
    alt.title = `${GAME_NAMES[game]} ile aç`;
    alt.disabled = busy !== null;
    alt.addEventListener("click", () => switchTo(a, game));
    li.append(alt);
  }
  li.append(more);
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

// `game` opens that game this once instead of the account's own. `force`
// is the user's "switch anyway" after an unreadable League client state.
async function switchTo(a, game = null, force = false) {
  if (busy) return;
  busy = a.id;
  say(`${a.name} açılıyor…`);
  render();
  try {
    await invoke("switch_account", { id: a.id, game, force });
    say("");
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
  if (e.button !== 0 || busy) return;
  const rows = [...$("accounts").querySelectorAll(".account")];
  if (rows.length < 2) return;
  const from = rows.indexOf(li);
  drag = { li, rows, from, to: from, startY: e.clientY, active: false, step: 0 };
}

window.addEventListener("pointermove", (e) => {
  if (!drag) return;
  const dy = e.clientY - drag.startY;
  if (!drag.active) {
    if (Math.abs(dy) < DRAG_START) return;
    drag.active = true;
    const r = drag.rows;
    // Distance between two rows, gap included.
    drag.step = r.length > 1 ? Math.abs(r[1].getBoundingClientRect().top - r[0].getBoundingClientRect().top) : drag.li.offsetHeight;
    $("accounts").classList.add("reordering");
    drag.li.classList.add("dragging");
  }
  const n = drag.rows.length;
  const min = -drag.from * drag.step - drag.step / 2;
  const max = (n - 1 - drag.from) * drag.step + drag.step / 2;
  const y = Math.max(min, Math.min(max, dy));
  drag.li.style.transform = `translateY(${y}px)`;
  drag.to = Math.max(0, Math.min(n - 1, drag.from + Math.round(y / drag.step)));
  drag.rows.forEach((row, i) => {
    if (row === drag.li) return;
    let shift = 0;
    if (drag.from < drag.to && i > drag.from && i <= drag.to) shift = -drag.step;
    if (drag.from > drag.to && i >= drag.to && i < drag.from) shift = drag.step;
    row.style.transform = shift ? `translateY(${shift}px)` : "";
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
  // The release would otherwise count as a click on the row.
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
  const sec = view.security;
  $("set-security-text").textContent =
    `${view.accounts.length} hesap · uygulama kilidi ${sec.lock_enabled ? "açık" : "kapalı"}`;
}

$("settings").addEventListener("click", () => {
  setSay("");
  fillSettings();
  show("settings");
});
$("set-back").addEventListener("click", () => {
  show("list");
  $("filter").focus();
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
  $("wiz-creds-legend").textContent =
    mode === "session" ? "Oturum dolarsa otomatik giriş için (isteğe bağlı)" : "Riot giriş bilgileri";
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
  $("sec-net").textContent = !update.supported
    ? "Uygulama internete bağlanmaz. Yalnızca bu bilgisayardaki Riot Client ile konuşur."
    : update.enabled
      ? "Uygulamanın tek internet bağlantısı güncelleme denetimidir; Ayarlar'dan kapatabilirsin. Bunun dışında yalnızca bu bilgisayardaki Riot Client ile konuşur."
      : "Güncelleme denetimi kapalı: uygulama internete bağlanmaz. Yalnızca bu bilgisayardaki Riot Client ile konuşur.";

  $("sec-counts").textContent =
    `${view.accounts.length} hesap, ${sec.saved_sessions} kayıtlı oturum, ${sec.saved_passwords} kayıtlı şifre.`;
  $("sec-path").textContent = sec.data_dir || "";
  $("sec-lock-off").hidden = !!sec.lock_enabled;
  $("sec-lock-on").hidden = !sec.lock_enabled;
  $("sec-lock-now").disabled = !sec.unlocked;
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


$("sec-back").addEventListener("click", () => {
  fillSettings();
  show("settings");
});
$("sec-open").addEventListener("click", (e) => whileWorking(e.currentTarget, () => secCall("open_data_dir", {}, "")));
$("sec-enable").addEventListener("click", () => {
  if ($("sec-new").value !== $("sec-new2").value) return secSay("İki şifre aynı değil.", true);
  const password = $("sec-new").value;
  secCall("set_lock", { current: null, new: password }, "Uygulama kilidi etkin.", "Oturumlar ve şifreler kilitleniyor…");
});
$("sec-disable").addEventListener("click", (e) =>
  whileWorking(e.currentTarget, () =>
    secCall("set_lock", { current: $("sec-current").value, new: null }, "Uygulama kilidi kaldırıldı.")));
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
  if (["in_game", "locked", "league_unknown"].includes(n.kind) && n.account) reportSwitchError({ kind: n.kind, message: n.text }, n.account);
  else say(n.text, n.error);
});
window.addEventListener("focus", () => {
  if (!$("view-list").hidden) $("filter").focus();
  refresh();
});
// What Riot Client is doing shows up in its own state, not as an event:
// poll while the window is on screen, faster while the wizard is waiting.
setInterval(() => {
  if (document.visibilityState === "visible" && !busy) refresh();
}, 2000);

(async () => {
  await refresh();
  if (!view.accounts.length) openWizard(); // first run
  else $("filter").focus();
})();
