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

const errorText = (e) => (e && e.message) || String(e);
const errorKind = (e) => (e && e.kind) || "other";

function show(name) {
  for (const v of ["list", "edit", "wizard", "unlock", "security"]) $(`view-${v}`).hidden = v !== name;
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

function visibleAccounts() {
  const q = $("filter").value.trim().toLocaleLowerCase("tr");
  return view.accounts.filter((a) => `${a.name} ${a.riot_id || ""}`.toLocaleLowerCase("tr").includes(q));
}

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
  const accounts = visibleAccounts();
  selected = Math.min(selected, Math.max(accounts.length - 1, 0));
  const rows = accounts.map((a, i) => row(a, i));
  if (!rows.length) {
    const empty = document.createElement("li");
    empty.className = "empty";
    empty.textContent = view.accounts.length ? "Eşleşen hesap yok." : "Henüz hesap yok. \"Hesap ekle\" ile başla.";
    rows.push(empty);
  }
  // Rows animate in only when the set of accounts changes, not on the
  // periodic refresh.
  const ids = view.accounts.map((a) => a.id).join("|");
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
  $("autostart").checked = !!view.autostart;
}

function row(a, i) {
  const li = document.createElement("li");
  li.className = "account";
  li.style.setProperty("--i", i);
  li.classList.toggle("active", a.id === view.active);
  li.classList.toggle("selected", i === selected);
  li.classList.toggle("busy", a.id === busy);

  const go = document.createElement("button");
  go.type = "button";
  go.className = "go";
  go.disabled = busy !== null;
  go.addEventListener("click", () => switchTo(a));

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
}

// `game` opens that game this once instead of the account's own.
async function switchTo(a, game = null) {
  if (busy) return;
  busy = a.id;
  say(`${a.name} açılıyor…`);
  render();
  try {
    await invoke("switch_account", { id: a.id, game });
    say("");
    $("filter").value = "";
  } catch (e) {
    reportSwitchError(e, a.id);
  } finally {
    busy = null;
    await refresh();
  }
}

// A switch refused because a match is on can be queued instead; one
// refused because the app is locked continues after unlocking.
function reportSwitchError(e, id) {
  if (errorKind(e) === "locked") return openUnlock(id);
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

$("filter").addEventListener("input", () => {
  selected = 0;
  render();
});

document.addEventListener("keydown", (e) => {
  if ($("view-list").hidden || e.target !== $("filter")) return;
  const accounts = visibleAccounts();
  if (e.key === "ArrowDown") selected = Math.min(selected + 1, accounts.length - 1);
  else if (e.key === "ArrowUp") selected = Math.max(selected - 1, 0);
  else if (e.key === "Enter" && accounts[selected]) {
    // For an account that plays both, Shift+Enter opens VALORANT.
    const a = accounts[selected];
    return void switchTo(a, e.shiftKey && a.game === "both" ? "valorant" : null);
  }
  else return;
  e.preventDefault();
  render();
});

$("queue-cancel").addEventListener("click", async () => {
  await invoke("cancel_queue");
  refresh();
});

$("autostart").addEventListener("change", async (e) => {
  try {
    await invoke("set_autostart", { enabled: e.target.checked });
  } catch (err) {
    say(errorText(err), true);
  }
  refresh();
});

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
  if (!/^(Key[A-Z]|Digit\d|Numpad\d|F([1-9]|1[0-2]))$/.test(e.code)) return; // modifier alone, or a key we do not offer
  const parts = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  parts.push(e.code);
  editCall("set_hotkey", { hotkey: parts.join("+") }, "Kısayol kaydedildi.");
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
      ? "Uygulamanın tek internet bağlantısı güncelleme denetimidir; aşağıdan kapatabilirsin. Bunun dışında yalnızca bu bilgisayardaki Riot Client ile konuşur."
      : "Güncelleme denetimi kapalı: uygulama internete bağlanmaz. Yalnızca bu bilgisayardaki Riot Client ile konuşur.";
  $("sec-update-card").hidden = !update.supported;
  $("sec-update-on").checked = !!update.enabled;
  $("sec-version").textContent = `Kurulu sürüm: ${view.version || ""}`;
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

$("security").addEventListener("click", () => {
  secSay("");
  $("sec-wipe").textContent = "Tüm verileri sil";
  fillSecurity();
  show("security");
});
$("sec-back").addEventListener("click", () => show("list"));
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
$("sec-update-on").addEventListener("change", (e) => secCall("set_update_checks", { enabled: e.target.checked }, ""));
$("sec-update-check").addEventListener("click", (e) =>
  whileWorking(e.currentTarget, async () => {
    try {
      const found = await invoke("check_update_now");
      secSay(found ? `Yeni sürüm bulundu: ${found}. Ana ekrandan kurabilirsin.` : "Kurulu sürüm güncel.");
    } catch (err) {
      secSay(errorText(err), true);
    }
    await refresh();
  }));
$("update-install").addEventListener("click", (e) =>
  whileWorking(e.currentTarget, async () => {
    say("Güncelleme indiriliyor ve doğrulanıyor…");
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
listen("notice", (e) => {
  const n = e.payload;
  if ((n.kind === "in_game" || n.kind === "locked") && n.account) reportSwitchError({ kind: n.kind, message: n.text }, n.account);
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
