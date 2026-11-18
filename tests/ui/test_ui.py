"""Drives the window in a headless browser against a pretend backend.

The real app hands `window.__TAURI__` to the page; here a small script stands
in for it, so every screen can be exercised without Windows or Riot Client.

    pip install playwright && playwright install chromium
    python tests/ui/test_ui.py            # prints one line per check
    python tests/ui/test_ui.py shots/     # also saves screenshots there
"""
import asyncio
import json
import pathlib
import re
import sys

from playwright.async_api import async_playwright

UI = pathlib.Path(__file__).resolve().parent.parent.parent / "ui"
SHOTS = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else None

DAY = 86_400
STATE = {
    "accounts": [
        {"id": "ana", "name": "Ana Hesap", "game": "lol", "rank": "Gold II 45 LP", "level": 212, "riot_id": "Ana#TR1",
         "has_session": True, "has_password": True, "needs_login": False, "idle_days": None, "stale": False,
         "hotkey": "Ctrl+Shift+Digit1", "note": "", "ban": None, "last_ok": 300, "updated": 300, "icon": 4567, "region": "tr"},
        {"id": "smurf", "name": "Smurf", "game": "both", "rank": "Diamond IV 3 LP", "level": 34, "riot_id": "Smurf#EUW",
         "has_session": True, "has_password": False, "needs_login": False, "idle_days": 12, "stale": True,
         "note": "duo için", "ban": {"until": None}, "last_ok": 100, "updated": 100},
        {"id": "val", "name": "Zeta", "game": "valorant", "rank": None, "level": 80, "riot_id": None,
         "has_session": False, "has_password": True, "needs_login": True, "idle_days": None, "stale": False,
         "note": "", "ban": {"until": "__IN_3_DAYS__"}, "last_ok": 200, "updated": 200},
        {"id": "eski", "name": "Eski Hesap", "game": "lol", "rank": "Master 312 LP", "level": 410, "riot_id": "Eski#TR1",
         "has_session": True, "has_password": False, "needs_login": False, "idle_days": 9, "stale": True,
         "note": "", "ban": None, "last_ok": 50, "updated": 50, "lookup_problem": "not_found", "region": "tr"},
    ],
    "active": "ana", "client": {"running": True, "login": "signed_in"}, "unsaved_session": False, "in_game": False,
    "security": {"lock_enabled": True, "unlocked": True, "data_dir": "C:\\Users\\x\\AppData\\Local\\QuickSwitch",
                 "saved_sessions": 2, "saved_passwords": 2, "auto_lock_minutes": 15},
    "pending": None, "autostart": True, "version": "0.6.0",
    "net": {"decided": False, "online": False, "lol_ranks": True, "icons": True, "refresh_minutes": 30},
    "update": {"supported": True, "enabled": True, "available": None}, "window_hotkey": "Ctrl+Shift+Space",
}

BACKEND = """
window.__calls = [];
window.__listeners = {};
window.__emit = (name, payload) => (window.__listeners[name] || []).forEach((f) => f({ payload }));
const state = %s;
for (const a of state.accounts) if (a.ban && a.ban.until === "__IN_3_DAYS__") a.ban.until = Date.now() + 3 * 86400000 - 60000;
const answers = {
  get_state: () => state,
  bulk_add: (a) => ({ added: a.lines.split("\\n").filter((l) => l.includes("--")).length, already_here: 1, incomplete: 0 }),
  import_backup: (a) => { if (a.password !== "yedek parolası") throw { kind: "backup_not_opened", message: "Yedek açılamadı: parola yanlış ya da dosya bozulmuş." }; return { added: 2, already_here: 1, incomplete: 0 }; },
  export_backup: (a) => { if (a.master !== "ana şifre") throw { kind: "wrong_password", message: "Ana şifre yanlış." }; return "C:\\\\Users\\\\x\\\\Downloads\\\\QuickSwitch-yedek-1.qsbackup"; },
  export_plain: () => "C:\\\\Users\\\\x\\\\Downloads\\\\QuickSwitch-hesaplar-ACIK-SIFRELI-1.json",
  copy_password: (a) => { if (a.master !== "ana şifre") throw { kind: "wrong_password", message: "Ana şifre yanlış." }; return 30; },
  import_league_accounts: () => ({ added: 3, already_here: 0, incomplete: 1 }),
  read_log: () => "1795009950 switch: client closed in 8651 ms, session files swapped in 14 ms (no saved session)\\n1795009968 login: 'direct' did not bring it forward; foreground is chrome.exe\\n1795011035 switch: sign-in accepted\\n",
  reorder_accounts: (a) => { state.accounts.sort((x, y) => a.ids.indexOf(x.id) - a.ids.indexOf(y.id)); return null; },
  set_note: (a) => { state.accounts.find((x) => x.id === a.id).note = a.note; return null; },
  set_net: (a) => { state.net = a.settings; return null; },
  refresh_ranks: (a) => {
    const ana = state.accounts.find((x) => x.id === "ana");
    Object.assign(ana, { wins: 120, losses: 98, rank_at: Math.floor(Date.now() / 1000) - 720, rank_from: "opgg" });
    return a.id ? { changed: 0, same: 1, not_found: 0, failed: 0, skipped: 0, paused: false } : { changed: 1, same: 1, not_found: 1, failed: 0, skipped: 0, paused: false };
  },
  icons: (a) => Object.fromEntries(a.numbers.map((n) => [n, "data:image/svg+xml,%%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 8 8'%%3E%%3Crect width='8' height='8' fill='%%235b7fd6'/%%3E%%3Ccircle cx='4' cy='4' r='2.4' fill='%%23e3b25c'/%%3E%%3C/svg%%3E"])),
  net_ledger: () => {
    const now = Math.floor(Date.now() / 1000);
    return [
      { at: now - 5, purpose: "lol_ranks", host: "op.gg", outcome: "ok", bytes: 48211 },
      { at: now - 9, purpose: "lol_ranks", host: "op.gg", outcome: "429", bytes: 0 },
      { at: now - 60, purpose: "icons", host: "ddragon.leagueoflegends.com", outcome: "off", bytes: 0 },
      { at: now - 900, purpose: "updates", host: "github.com", outcome: "ok", bytes: 0 },
    ];
  },
  save_diagnostics: () => "C:\\\\Users\\\\x\\\\Downloads\\\\QuickSwitch-tani-1.json",
};
window.__TAURI__ = {
  core: { invoke: async (command, args) => { window.__calls.push([command, args]); return answers[command] ? answers[command](args || {}) : null; } },
  event: { listen: async (name, f) => { (window.__listeners[name] ||= []).push(f); } },
};
""" % json.dumps(STATE)

FAILED = []


def check(what, ok, detail=""):
    print(("ok   " if ok else "FAIL ") + what + (f"  ({detail})" if detail and not ok else ""))
    if not ok:
        FAILED.append(what)


async def main():
    async with async_playwright() as p:
        browser = await p.chromium.launch()
        page = await browser.new_page(viewport={"width": 1080, "height": 700})
        errors = []
        page.on("pageerror", lambda e: errors.append(str(e)))
        # Everything the page asks for, to see that none of it is elsewhere.
        asked = []
        page.on("request", lambda r: asked.append(r.url))
        await page.add_init_script(BACKEND)
        await page.goto((UI / "index.html").as_uri())
        await page.wait_for_timeout(500)

        async def shot(name):
            if SHOTS:
                SHOTS.mkdir(parents=True, exist_ok=True)
                await page.screenshot(path=str(SHOTS / f"{name}.png"))

        text = lambda sel: page.inner_text(sel)
        # The same, as it is written rather than as the theme sets it: headings
        # and ranks are shown in capitals.
        written = lambda sel: page.text_content(sel)
        names = lambda: page.eval_on_selector_all("#accounts .account .name > span:first-child", "els => els.map(e => e.textContent)")
        calls = lambda name: page.evaluate("n => window.__calls.filter(c => c[0] === n).map(c => c[1])", name)

        # ---- the shell
        check("the rail marks the page that is open", await page.get_attribute('#rail .nav[data-view="list"]', "aria-current") == "page")
        check("the rail counts the accounts", await text("#nav-count") == "4")
        check("with the lock on, the rail offers to lock", "kilitle" in await text("#rail-lock"))
        check("the version is shown", "0.6.0" in await text("#rail-version"))

        # ---- the internet is asked about, once
        check("before an answer, the question is on the page", not await page.is_hidden("#net-ask"))
        check("and nothing has gone out", not await calls("refresh_ranks") and not await calls("icons") and not await calls("set_net"))
        check("the rail says it is undecided", await text("#net-state") == "?")
        check("there is nothing to refresh yet", await page.is_hidden("#ranks-refresh"))
        await shot("ask")
        await page.click("#net-yes")
        await page.wait_for_timeout(1900)
        check("a yes is kept", (await calls("set_net"))[-1]["settings"]["online"] is True and (await calls("set_net"))[-1]["settings"]["decided"] is True)
        check("the question does not come back", await page.is_hidden("#net-ask"))
        check("ranks are looked up right away, for everyone", (await calls("refresh_ranks"))[-1] == {"id": None})
        check("and the result is said in a sentence", "1 hesap güncellendi" in await text("#message") and "1 Riot ID bulunamadı" in await text("#message"), await text("#message"))
        await page.click("#message-text")
        check("icons are asked for by number, once", await calls("icons") == [{"numbers": [4567]}], str(await calls("icons")))
        check("an account with a picture wears it", await page.locator('#accounts .account[data-id="ana"] .portrait.pictured img').count() == 1)
        check("one without keeps its crest alone", await page.locator('#accounts .account[data-id="smurf"] .portrait img').count() == 0)
        check("wins, losses and the rate are on the card", "120G 98M %55" in await text('#accounts .account[data-id="ana"]'))
        check("where the rank came from is a hover away", "OP.GG" in (await page.get_attribute('#accounts .account[data-id="ana"] .rank .line, #accounts .account[data-id="ana"] [title*="OP.GG"]', "title") or ""))
        check("a Riot ID that is not there is flagged", "Riot ID bulunamadı" in await text('#accounts .account[data-id="eski"]'))
        check("the rail says the internet is on", await text("#net-state") == "açık")

        # ---- the accounts
        rows = await text("#accounts")
        check("accounts come in the stored order", await names() == ["Ana Hesap", "Smurf", "Zeta", "Eski Hesap"])
        check("they are laid out as cards, side by side", len(set(await page.eval_on_selector_all("#accounts .account", "els => els.map(e => Math.round(e.getBoundingClientRect().top))"))) < 4)
        check("a permanent ban is named", "kalıcı ban" in rows)
        check("a timed ban counts down in days", "ban: 3 gün" in rows)
        check("a ban takes the place of the unused-session warning", "12 gündür açılmadı" not in rows)
        check("an unused session is flagged", "9 gündür açılmadı" in rows)
        check("a note shows on its account", "duo için" in rows)
        check("rank is shown as tier and points", "Gold II" in await written("#accounts") and "45 LP" in rows)
        check("an English tier name is capitalised as English, not as Turkish", await page.inner_text('#accounts .account[data-id="smurf"] .rank-line b') == "DIAMOND IV", await page.inner_text('#accounts .account[data-id="smurf"] .rank-line b'))
        tiers = await page.eval_on_selector_all("#accounts .account", "els => els.map(e => e.dataset.tier || '')")
        check("each account carries its tier", tiers == ["gold", "diamond", "", "master"], str(tiers))
        fills = await page.eval_on_selector_all("#accounts .account .lp i", "els => els.map(e => e.style.getPropertyValue('--lp'))")
        check("the bar under the rank follows the points", fills == ["0.45", "0.03", "0", "1"], str(fills))
        crests = await page.eval_on_selector_all("#accounts .account .crest", "els => els.map(e => [e.querySelectorAll('.blades').length, e.querySelectorAll('.ring').length, e.classList.contains('glow'), e.querySelector('text') ? e.querySelector('text').textContent : ''])")
        check("a crest grows with its tier: blades, then a glow, then a ring", crests == [[1, 0, False, "II"], [1, 0, True, "IV"], [0, 0, False, ""], [1, 1, True, ""]], str(crests))
        metals = await page.eval_on_selector_all("#accounts .account .crest .plate:not(.none)", "els => els.map(e => e.getAttribute('stroke'))")
        check("each crest is drawn in its own metal, and that metal exists", metals == ["url(#metal-gold)", "url(#metal-diamond)", "url(#metal-master)"] and await page.evaluate("['gold','diamond','master','iron','challenger'].every(t => document.getElementById('metal-' + t))"), str(metals))
        check("the open account is said to be open", "Açık" in await text("#accounts .account.active"))
        check("an account in both games offers both", await page.eval_on_selector_all("#accounts .account:nth-child(2) .acts .alt", "els => els.map(e => e.textContent)") == ["League", "VALORANT"])
        widths = await page.eval_on_selector_all("#accounts .account .name > span:first-child", "els => els.map(e => e.scrollWidth <= e.clientWidth + 1)")
        check("no name is cut short by its tags", all(widths), str(widths))
        check("nothing overflows sideways", await page.evaluate("document.documentElement.scrollWidth <= window.innerWidth"))
        bar = await page.evaluate("(() => { const f = document.getElementById('filter'); return [Math.round(f.getBoundingClientRect().width - parseFloat(getComputedStyle(f).minWidth)), document.getElementById('add').getBoundingClientRect().right <= document.querySelector('.bar').getBoundingClientRect().right + 1]; })()")
        check("the bar ends where its line ends, with room to spare", bar[0] >= 24 and bar[1], str(bar))
        await shot("cards")

        # ---- order, layout and filters
        await page.select_option("#sort", "rank")
        check("by rank: highest first, unranked last", await names() == ["Eski Hesap", "Smurf", "Ana Hesap", "Zeta"])
        await page.select_option("#sort", "name")
        check("by name", await names() == ["Ana Hesap", "Eski Hesap", "Smurf", "Zeta"])
        await page.select_option("#sort", "recent")
        check("by last use", await names() == ["Ana Hesap", "Zeta", "Smurf", "Eski Hesap"])
        await page.select_option("#sort", "manual")
        await page.click('#layout [data-layout="rows"]')
        tops = await page.eval_on_selector_all("#accounts .account", "els => els.map(e => Math.round(e.getBoundingClientRect().top))")
        check("rows put one account on each line", len(set(tops)) == 4, str(tops))
        await shot("rows")

        # Dragging the first row below the second, in rows.
        box = await page.eval_on_selector_all("#accounts .account", "els => els.map(e => { const b = e.getBoundingClientRect(); return [b.left + 120, b.top + b.height / 2]; })")
        await page.mouse.move(*box[0]); await page.mouse.down()
        await page.mouse.move(box[0][0], box[1][1] + 4, steps=6); await page.mouse.up()
        await page.wait_for_timeout(200)
        check("a row can be dragged to a new place", await names() == ["Smurf", "Ana Hesap", "Zeta", "Eski Hesap"], str(await names()))
        check("the drop did not count as a click", len(await calls("switch_account")) == 0)

        await page.click('#layout [data-layout="cards"]')
        # And a card sideways, in the grid: back to where it was.
        box = await page.eval_on_selector_all("#accounts .account", "els => els.map(e => { const b = e.getBoundingClientRect(); return [b.left + b.width / 2, b.top + 30]; })")
        await page.mouse.move(*box[1]); await page.mouse.down()
        await page.mouse.move(box[0][0] - 10, box[0][1], steps=6); await page.mouse.up()
        await page.wait_for_timeout(200)
        check("a card can be dragged across the grid", await names() == ["Ana Hesap", "Smurf", "Zeta", "Eski Hesap"], str(await names()))

        statuses = await page.eval_on_selector_all("#status button", "els => els.map(e => e.textContent)")
        check("the rail counts what needs a look", statuses == ["Hepsi4", "Giriş gerekli1", "Banlı2", "Uzun süredir açılmadı2"], str(statuses))
        await page.click('#status [data-status="banned"]')
        check("a count filters the accounts to it", await names() == ["Smurf", "Zeta"])
        await page.click('#status [data-status="all"]')
        await page.fill("#filter", "duo")
        check("search looks in notes too", await names() == ["Smurf"])
        await page.fill("#filter", "yok böyle biri")
        check("an empty result says what to do", "Eşleşen hesap yok" in await written("#accounts"))
        await page.fill("#filter", "")

        # ---- the track
        step = lambda name, account="smurf": page.evaluate("([s, a]) => window.__emit('switch-step', { step: s, account: a })", [name, account])
        stations = lambda: page.eval_on_selector_all("#track-stations li", "els => els.map(e => e.textContent + ':' + e.className)")
        check("no switch, no track", await page.is_hidden("#track"))
        await step("closing")
        check("a switch lays out its stations", await stations() == ["Kapat:now", "Oturum:", "Aç:", "Giriş:", "Oyun:"], str(await stations()))
        check("and names the account", await text("#track-name") == "Smurf")
        await step("restoring"); await step("starting"); await step("waiting")
        check("stations behind are reached, the current one is lit", await stations() == ["Kapat:reached", "Oturum:reached", "Aç:reached", "Giriş:now", "Oyun:"], str(await stations()))
        await step("signing_in")
        check("the password station appears only when needed", await stations() == ["Kapat:reached", "Oturum:reached", "Aç:reached", "Giriş:reached", "Şifre:now", "Oyun:"], str(await stations()))
        await shot("track")
        await page.click("#track-cancel")
        check("cancel reaches the app", len(await calls("cancel_switch")) == 1)
        await step("cancelled", "")
        check("a cancelled switch clears the track", await page.is_hidden("#track"))

        await step("closing"); await step("restoring"); await step("starting"); await step("waiting"); await step("stopped")
        check("a switch that stops marks where", (await stations())[3] == "Giriş:failed" and "Tamamlanmadı" in await text("#track-text"))
        check("and stays until it is closed", not await page.is_hidden("#track") and await page.is_hidden("#track-cancel"))
        await page.click("#track-close")
        check("closing it clears the track", await page.is_hidden("#track"))

        await step("game", "ana"); 
        check("an account already signed in has only the game to open", await stations() == ["Oyun:now"], str(await stations()))
        await step("done", "ana")
        check("a finished switch says so", "Giriş yapıldı" in await text("#track-text"))
        await page.wait_for_timeout(2800)
        check("and leaves by itself", await page.is_hidden("#track"))

        await step("waiting"); await step("signing_in"); await step("verifying"); await step("stopped")
        check("a code that is asked for is not shown as a failure", "doğrulama kodu" in await text("#track-text") and (await stations())[-2] == "Şifre:now", str(await stations()))
        await page.click("#track-close")

        # ---- messages
        await page.evaluate("window.__emit('notice', { text: 'Riot kullanıcı adını ya da şifreyi kabul etmedi.', error: true, kind: null, account: null })")
        check("an error floats over the stage", "kabul etmedi" in await text("#message") and await page.eval_on_selector("#message", "e => e.classList.contains('error')"))
        await page.click("#message-text")
        check("and goes when clicked", await page.is_hidden("#message"))

        # ---- a right click
        menu = lambda: page.eval_on_selector_all("#menu button", "els => els.map(e => e.textContent)")
        await page.click('#accounts .account[data-id="ana"] .go', button="right")
        check("a right click offers what can be done with the account", await menu() == ["Oyunu aç", "Rankı yenile", "Riot ID'yi kopyala", "OP.GG'de aç", "Hesap ayarları"], str(await menu()))
        box = await page.eval_on_selector("#menu", "e => { const b = e.getBoundingClientRect(); return b.right <= innerWidth && b.bottom <= innerHeight; }")
        check("the menu stays inside the window", box)
        await page.wait_for_timeout(200)
        await shot("menu")
        await page.click("#menu button:nth-child(4)")
        check("the profile is opened by the app, by account", (await calls("open_profile"))[-1] == {"id": "ana"})
        check("choosing closes the menu", await page.is_hidden("#menu"))
        await page.click('#accounts .account[data-id="val"] .go', button="right")
        check("an account without a Riot ID is offered less", await menu() == ["Hesaba geç", "Hesap ayarları"], str(await menu()))
        await page.keyboard.press("Escape")
        check("Escape closes the menu", await page.is_hidden("#menu"))
        check("a right click switched nothing", len(await calls("switch_account")) == 0)

        # ---- one account: note and copying the password
        await page.click('#accounts .account[data-id="ana"] .more')
        await page.fill("#edit-note", "ana hesabım")
        await page.dispatch_event("#edit-note", "change")
        await page.wait_for_timeout(200)
        check("a note is saved", (await calls("set_note"))[-1] == {"id": "ana", "note": "ana hesabım"})
        check("with the lock on, copying asks for the master password", not await page.is_hidden("#edit-copy-master"))
        await page.click("#edit-copy-go")
        await page.wait_for_timeout(200)
        check("a wrong master password copies nothing", "Ana şifre yanlış" in await text("#edit-message"))
        await page.fill("#edit-copy-master", "ana şifre")
        await page.click("#edit-copy-go")
        await page.wait_for_timeout(200)
        check("copying says how long it stays", "30 saniye" in await text("#edit-message"))
        check("the master password box is emptied", await page.input_value("#edit-copy-master") == "")
        check("the lookup fields show what is known", await page.input_value("#edit-riot-id") == "Ana#TR1" and await page.input_value("#edit-region") == "tr")
        check("and say where the rank came from", "op.gg'den alındı" in (await text("#edit-lookup-state")).lower(), await text("#edit-lookup-state"))
        await page.fill("#edit-riot-id", "Ana Yeni#TR1")
        await page.select_option("#edit-region", "euw")
        await page.click("#edit-lookup-save")
        await page.wait_for_timeout(200)
        check("a Riot ID and region can be set by hand", (await calls("set_lookup"))[-1] == {"id": "ana", "riotId": "Ana Yeni#TR1", "region": "euw"}, str(await calls("set_lookup")))
        await page.click("#edit-rank-refresh")
        await page.wait_for_timeout(1900)
        check("one account's rank can be looked up alone", (await calls("refresh_ranks"))[-1] == {"id": "ana"} and "değişiklik yok" in await text("#edit-message"), await text("#edit-message"))
        await shot("edit")
        await page.click("#edit-back")
        await page.click('#accounts .account[data-id="smurf"] .more')
        check("no stored password, nothing to copy", await page.is_hidden("#edit-copy"))
        await page.click("#edit-back")

        # ---- moving accounts in and out
        await page.click("#set-data")
        check("the rail follows to the page", await page.get_attribute("#set-data", "aria-current") == "page")
        await page.wait_for_timeout(250)
        await shot("data")
        await page.fill("#data-backup-pass", "yedek parolası")
        await page.fill("#data-backup-pass2", "başka bir şey")
        await page.click("#data-backup")
        check("backup passwords must match", "aynı değil" in await text("#data-message"))
        check("and nothing was asked of the app", len(await calls("export_backup")) == 0)
        await page.fill("#data-backup-pass2", "yedek parolası")
        await page.click("#data-backup")
        await page.wait_for_timeout(1700)
        check("the app lock's password is needed for a backup", "Ana şifre yanlış" in await text("#data-message"))
        await page.fill("#data-master", "ana şifre")
        await page.click("#data-backup")
        await page.wait_for_timeout(1700)
        check("a backup says where it went", ".qsbackup" in await text("#data-message"))
        check("password boxes are emptied afterwards", await page.input_value("#data-backup-pass") == "")

        backup = {"name": "yedek.qsbackup", "mimeType": "application/json", "buffer": b"{}"}
        await page.fill("#data-restore-pass", "yanlış")
        await page.set_input_files("#data-restore-file", backup)
        await page.wait_for_timeout(200)
        check("a wrong backup password is said", "parola yanlış" in await text("#data-message"))
        await page.fill("#data-restore-pass", "yedek parolası")
        await page.set_input_files("#data-restore-file", backup)
        await page.wait_for_timeout(200)
        check("a backup is previewed before anything is added", "2 hesap eklenecek" in await text("#data-restore-preview"))
        check("the preview added nothing", all(c["dryRun"] for c in await calls("import_backup")))
        await page.click("#data-restore-go")
        await page.wait_for_timeout(200)
        check("then added on a yes", (await calls("import_backup"))[-1]["dryRun"] is False and "2 hesap eklendi" in await text("#data-message"))

        check("nothing to add, nothing to press", await page.is_disabled("#data-bulk-go"))
        await page.fill("#data-bulk", "u1--p1\nu2--Ad#TR1--p2\n")
        await page.wait_for_timeout(600)
        check("pasted lines are counted as typed", "2 hesap eklenecek, 1 zaten kayıtlı" in await text("#data-bulk-preview"))
        await page.click("#data-bulk-go")
        await page.wait_for_timeout(1700)
        check("pasted lines are added", (await calls("bulk_add"))[-1]["dryRun"] is False and await page.input_value("#data-bulk") == "")
        check("and the button waits for new lines", await page.is_disabled("#data-bulk-go"))

        await page.set_input_files("#sec-import-file", {"name": "export.json", "mimeType": "application/json", "buffer": b"[]"})
        await page.wait_for_timeout(200)
        check("a League Accounts file is imported", "3 hesap eklendi" in await text("#data-message"))

        check("the plain export is locked until its warning is ticked", await page.is_disabled("#data-plain"))
        await page.check("#data-plain-ok")
        await page.select_option("#data-plain-region", "euw")
        await page.click("#data-plain")
        await page.wait_for_timeout(200)
        check("the plain export passes the region on", (await calls("export_plain"))[-1]["region"] == "euw")
        check("and says to delete the file", "sil" in await text("#data-message"))
        check("the warning has to be ticked again next time", await page.is_disabled("#data-plain"))
        await shot("data-done")

        # ---- the other pages
        await page.click("#nav-log")
        await page.wait_for_timeout(200)
        lines = await page.eval_on_selector_all("#log-lines li", "els => els.map(e => e.className)")
        check("the log is shown line by line, trouble marked, switches set apart", lines == ["start", "bad", ""], str(lines))
        check("times are shown as a clock says them", ":" in await page.inner_text("#log-lines li time"))
        await page.click("#log-bundle")
        await page.wait_for_timeout(1900)
        check("a diagnostics bundle says where it went and what is not in it", "QuickSwitch-tani" in await text("#log-message") and "şifre" in await text("#log-message"), await text("#log-message"))
        await shot("log")

        # ---- the internet page
        await page.click("#nav-net")
        await page.wait_for_timeout(300)
        check("the internet page opens from the rail", not await page.is_hidden("#view-net") and await page.get_attribute("#nav-net", "aria-current") == "page")
        check("it shows the switches as they are", await page.is_checked("#net-online") and await page.is_checked("#net-ranks") and await text("#net-pill") == "Açık")
        ledger = await page.eval_on_selector_all("#net-ledger tbody tr", "els => els.map(e => [...e.cells].slice(1).map(c => c.textContent).join('|') + '|' + e.className)")
        check("every connection is listed: what for, where to, how it went", ledger == [
            "League rankı|op.gg|tamam|47 KB|",
            "League rankı|op.gg|sunucu 429 dedi||bad",
            "Profil simgesi|ddragon.leagueoflegends.com|kapalı olduğu için kurulmadı||quiet",
            "Güncelleme|github.com|tamam||",
        ], str(ledger))
        check("the page says where things go", "op.gg" in await text("#view-net .flows") and "ddragon.leagueoflegends.com" in await text("#view-net .flows"))
        check("nothing overflows sideways here either", await page.evaluate("document.documentElement.scrollWidth <= window.innerWidth"))
        await shot("net")
        await page.uncheck("#net-online")
        await page.wait_for_timeout(250)
        check("the master switch turns everything off", (await calls("set_net"))[-1]["settings"]["online"] is False and "hiçbir bağlantı kurmaz" in await text("#net-message"))
        check("the parts under it go quiet", await page.is_disabled("#net-ranks") and await page.is_disabled("#net-every") and await text("#net-pill") == "Kapalı")
        check("and the rail says so", await text("#net-state") == "kapalı")
        await page.click('#rail .nav[data-view="list"]')
        check("with the internet off there is nothing to refresh", await page.is_hidden("#ranks-refresh"))
        await page.click("#nav-net")
        await page.check("#net-online")
        await page.select_option("#net-every", "60")
        await page.wait_for_timeout(250)
        check("how often is kept", (await calls("set_net"))[-1]["settings"] == {"decided": True, "online": True, "lol_ranks": True, "icons": True, "refresh_minutes": 60}, str((await calls("set_net"))[-1]))
        await page.uncheck("#net-ranks")
        await page.uncheck("#net-icons")
        await page.wait_for_timeout(250)
        check("with only the update check left, it says so", await text("#net-pill") == "Yalnız güncelleme" and await text("#net-state") == "yalnız güncelleme", await text("#net-pill"))
        await page.check("#net-ranks")
        await page.check("#net-icons")
        await page.wait_for_timeout(250)
        await page.click("#net-clear")
        check("the list can be cleared", len(await calls("clear_net_ledger")) == 1)
        await page.click("#settings")
        await page.wait_for_timeout(250)
        check("settings open from the rail", not await page.is_hidden("#view-settings") and "0.6.0" in await text("#view-settings"))
        await shot("settings")
        await page.click("#set-security")
        await page.wait_for_timeout(250)
        check("security opens from the rail", "Uygulama kilidi" in await text("#view-security"))
        check("security names what the internet is used for", "güncelleme denetimi, League rankı, profil simgeleri" in await text("#sec-net"), await text("#sec-net"))
        await shot("security")
        await page.click('#rail .nav[data-view="list"]')
        check("and the rail leads back to the accounts", not await page.is_hidden("#view-list"))
        await page.click("#add")
        await page.wait_for_timeout(250)
        await shot("wizard")
        await page.click("#set-data")
        check("leaving the wizard by the rail closes it", await page.is_hidden("#view-wizard"))

        # ---- the smallest window the app allows
        await page.click('#rail .nav[data-view="list"]')
        await page.set_viewport_size({"width": 860, "height": 560})
        await page.wait_for_timeout(150)
        check("at its smallest, the window still holds the bar and the cards", await page.evaluate("document.documentElement.scrollWidth <= window.innerWidth && document.getElementById('add').getBoundingClientRect().right <= window.innerWidth"))
        fits = "(() => { const f = document.getElementById('filter'); return Math.round(f.getBoundingClientRect().width - parseFloat(getComputedStyle(f).minWidth)); })()"
        room = await page.evaluate(fits)
        check("on one line, with room to spare should a font run wider", room >= 24 and len(set(await page.eval_on_selector_all(".bar > :not([hidden])", "els => els.map(e => Math.round(e.getBoundingClientRect().top))"))) == 1, str(room))
        await shot("smallest")
        await page.set_viewport_size({"width": 1080, "height": 700})

        # ---- the theme keeps the window's promise: nothing from outside, nothing of Riot's
        style = (UI / "style.css").read_text(encoding="utf-8")
        drawn = re.findall(r"url\(\s*[\"']?([^\"')]*)", style)
        check("the stylesheet ships no font, and every picture in it is drawn in it", "@font-face" not in style and "@import" not in style and drawn and all(u.startswith("data:image/svg+xml,") for u in drawn), str([u[:40] for u in drawn]))
        outside = [u for u in asked if not u.startswith(("file:", "data:"))]
        check("the page asked for nothing outside itself", not outside, "; ".join(outside))
        check("corners are square and the brand keeps its own letters", await page.evaluate("getComputedStyle(document.querySelector('#accounts .account')).borderRadius") == "0px" and await page.inner_text("#rail .brand") == "QUICKSWITCH", await page.inner_text("#rail .brand"))

        check("no script errors", not errors, "; ".join(errors))
        await browser.close()
    print(f"\n{'FAILED: ' + str(len(FAILED)) if FAILED else 'all checks passed'}")
    sys.exit(1 if FAILED else 0)


asyncio.run(main())
