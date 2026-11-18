//! A picture of what the app has to work with, for finding out why
//! something does not behave as expected on someone's computer.
//!
//! It is made to be shared, so it is built the other way round from a
//! dump: nothing is included unless it is known to be harmless. Of what
//! the clients answer, only the **shape** is kept: which keys there are
//! and what kind of value each holds. A string is replaced by its length,
//! a number by the word "number". The few values that are kept are short
//! words from a fixed list of keys that say what state something is in
//! (a region, a phase, a tier), never who anyone is.

use super::*;
use serde_json::{json, Map, Value};

/// Keys whose short text values describe a state and are kept as they are.
const TELLING: [&str; 16] = [
    "type", "region", "locale", "webRegion", "phase", "tier", "division", "scope", "error", "product", "patchline", "method", "queueType",
    "highestTier", "state", "action",
];

/// What to ask Riot Client, and what to ask the League client.
const RIOT_PATHS: [&str; 9] = [
    "/rso-auth/v1/session",
    "/riot-login/v1/status",
    "/rso-authenticator/v1/authentication",
    "/riot-client-auth/v1/userinfo",
    "/player-account/aliases/v1/active",
    "/riotclient/region-locale",
    "/product-session/v1/external-sessions",
    "/entitlements/v1/token",
    "/patch/v1/installs",
];
const LEAGUE_PATHS: [&str; 7] = [
    "/lol-summoner/v1/current-summoner",
    "/lol-ranked/v1/current-ranked-stats",
    "/lol-gameflow/v1/gameflow-phase",
    "/riotclient/region-locale",
    "/lol-inventory/v1/wallet/lol_blue_essence",
    "/lol-champions/v1/owned-champions-minimal",
    "/lol-patch/v1/products/league_of_legends/state",
];

/// Whether a key is an id (an account, a session, a season) rather than a
/// name. Riot's ids are UUIDs and long hex strings: long, and full of
/// digits, which the names of fields are not.
fn id_like(key: &str) -> bool {
    let key = key.trim_start_matches(['-', ' ']).trim_matches(['"', '\'']);
    key.len() >= 16 && key.bytes().filter(u8::is_ascii_digit).count() >= 4
}

/// The shape of `value`: keys kept, values replaced by what kind they are.
pub(super) fn shape(key: &str, value: &Value) -> Value {
    match value {
        Value::Null => json!("null"),
        Value::Bool(_) => json!("bool"),
        Value::Number(_) => json!("number"),
        Value::String(text) => {
            // Some answers carry a whole JSON document as text.
            if let Ok(inner @ (Value::Object(_) | Value::Array(_))) = serde_json::from_str::<Value>(text) {
                return json!({ "json in text": shape(key, &inner) });
            }
            let telling = TELLING.contains(&key) && text.len() <= 24 && text.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b' '));
            if telling {
                json!(text)
            } else {
                json!(format!("text({})", text.chars().count()))
            }
        }
        Value::Array(items) => match items.first() {
            Some(first) => json!({ "list of": items.len(), "each like": shape(key, first) }),
            None => json!("empty list"),
        },
        Value::Object(map) => {
            // A map keyed by ids (accounts, seasons, sessions) would spell those ids out.
            let ids = map.len() > 1 && map.keys().all(|k| id_like(k));
            if ids {
                let first = map.values().next().map(|v| shape("", v)).unwrap_or(json!("null"));
                return json!({ "map of": map.len(), "each like": first });
            }
            // An id among names is put in its place one by one, even alone.
            let mut ids_seen = 0;
            let mut out = Map::new();
            for (k, v) in map {
                let shown = if id_like(k) {
                    ids_seen += 1;
                    format!("(id {ids_seen})")
                } else {
                    k.clone()
                };
                out.insert(shown, shape(k, v));
            }
            Value::Object(out)
        }
    }
}

/// The keys of a YAML file with the kind of each value, indentation kept.
/// Riot's settings files are simple enough to be read line by line.
fn yaml_outline(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .take(200)
        .map(|line| {
            let indent = line.len() - line.trim_start().len();
            let body = line.trim_start();
            let pad = " ".repeat(indent);
            match body.split_once(':') {
                Some((key, value)) if !key.contains(' ') || key.starts_with('-') => {
                    let key = if id_like(key) { "(id)" } else { key };
                    let value = value.trim().trim_matches('"');
                    if value.is_empty() {
                        format!("{pad}{key}:")
                    } else {
                        format!("{pad}{key}: text({})", value.chars().count())
                    }
                }
                _ => format!("{pad}text({})", body.chars().count()),
            }
        })
        .collect()
}

impl Switcher {
    /// Everything worth knowing about this installation and the clients it
    /// talks to, with nothing that identifies an account. `app_version` is
    /// the caller's own version string.
    pub fn diagnostics(&self, app_version: &str) -> Value {
        let cfg = self.config().unwrap_or_default();
        let state = self.state().unwrap_or_default();
        let riot = &self.env.riot_games_dir;

        // Session files: where they are and how big, never what is in them.
        let mut files = Vec::new();
        for rel in &cfg.session_paths {
            let path = riot.join(rel);
            match std::fs::metadata(&path) {
                Ok(m) if m.is_dir() => {
                    let inside = std::fs::read_dir(&path).map(|d| d.flatten().count()).unwrap_or(0);
                    files.push(json!({ "path": rel, "kind": "folder", "entries": inside }));
                }
                Ok(m) => {
                    let age = m.modified().ok().and_then(|t| t.elapsed().ok()).map(|d| d.as_secs());
                    let outline = if rel.ends_with(".yaml") { std::fs::read_to_string(&path).ok().map(|t| yaml_outline(&t)) } else { None };
                    files.push(json!({ "path": rel, "kind": "file", "bytes": m.len(), "seconds since written": age, "outline": outline }));
                }
                Err(_) => files.push(json!({ "path": rel, "kind": "missing" })),
            }
        }

        let ask = |league: bool, paths: &[&str]| -> Value {
            let mut out = Map::new();
            for path in paths {
                let answer = match self.api.probe(league, path) {
                    Some((status, body)) => json!({ "status": status, "shape": shape("", &body) }),
                    None => json!("no answer"),
                };
                out.insert(path.to_string(), answer);
            }
            Value::Object(out)
        };
        let wanted: Vec<String> = cfg.client_processes.iter().chain(&cfg.game_processes).cloned().collect();

        // Accounts by what they have, not by who they are.
        let accounts: Vec<Value> = state
            .accounts
            .iter()
            .map(|a| {
                json!({
                    "game": a.game,
                    "has session": !a.hash.is_empty(),
                    "has password": self.env.secret_file(&a.id).is_file(),
                    "has riot id": a.riot_id.is_some(),
                    "has account id": a.puuid.is_some(),
                    "region": a.region,
                    "needs login": a.needs_login,
                    "rank from": a.rank_from,
                    "lookup problem": a.lookup_problem,
                    "days since accepted": (a.last_ok > 0).then(|| self.now().saturating_sub(a.last_ok) / 86_400),
                })
            })
            .collect();

        json!({
            "quickswitch": app_version,
            "made at": self.now(),
            "system": { "os": std::env::consts::OS, "arch": std::env::consts::ARCH },
            "riot client": {
                "found": self.riot_exe(&cfg).is_ok(),
                "set by hand": cfg.riot_client_exe.is_some(),
                "data folder exists": riot.is_dir(),
            },
            "running": self.procs.running(&wanted),
            "lock": { "enabled": self.vault.enabled(), "unlocked": self.vault.unlocked(), "auto lock minutes": cfg.auto_lock_minutes },
            "internet": cfg.net,
            "accounts": accounts,
            "session files": files,
            "riot client answers": ask(false, &RIOT_PATHS),
            "league client answers": ask(true, &LEAGUE_PATHS),
            "log": self.log_tail(24 * 1024).lines().collect::<Vec<_>>(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shape_keeps_keys_and_kinds_and_drops_what_was_said() {
        let answer = json!({
            "type": "authenticated",
            "persistLogin": true,
            "accessToken": "eyJhbGciOiJSUzI1NiJ9.secret.secret",
            "subject": "b639ec2f-e2a0-509a-85f2-ffbec2526938",
            "gameName": "GizliAd",
            "region": "TR",
            "error": "auth_failure",
            "count": 3,
            "nothing": null,
            "list": [{"tier": "GOLD", "name": "GizliAd"}, {"tier": "IRON", "name": "x"}],
            "none": [],
            "userInfo": "{\"sub\":\"gizli-kimlik\",\"ban\":{\"restrictions\":[{\"scope\":\"lol\",\"type\":\"TIME_BAN\"}]}}",
        });
        let s = shape("", &answer);
        assert_eq!(s["type"], "authenticated", "a state word is kept");
        assert_eq!(s["region"], "TR");
        assert_eq!(s["error"], "auth_failure");
        assert_eq!(s["persistLogin"], "bool");
        assert_eq!(s["accessToken"], "text(34)");
        assert_eq!(s["gameName"], "text(7)");
        assert_eq!(s["count"], "number");
        assert_eq!(s["list"]["list of"], 2);
        assert_eq!(s["list"]["each like"]["tier"], "GOLD");
        assert_eq!(s["none"], "empty list");
        assert_eq!(s["userInfo"]["json in text"]["ban"]["restrictions"]["each like"]["type"], "TIME_BAN");
        let text = s.to_string();
        for secret in ["secret", "b639ec2f", "GizliAd", "gizli-kimlik"] {
            assert!(!text.contains(secret), "{secret} survived: {text}");
        }
    }

    #[test]
    fn a_state_word_is_only_kept_if_it_looks_like_one() {
        // A long or odd value under a telling key is not trusted to be a state.
        assert_eq!(shape("type", &json!("a much longer sentence than any state name")), "text(42)");
        assert_eq!(shape("region", &json!("tr/../x")), "text(7)");
        assert_eq!(shape("name", &json!("TR")), "text(2)", "and a short word under any other key is not kept");
    }

    #[test]
    fn a_map_keyed_by_ids_does_not_spell_them_out() {
        let seasons = json!({
            "b639ec2f-e2a0-509a-85f2-ffbec2526938": {"tier": "GOLD"},
            "aaaaaaaa-e2a0-509a-85f2-ffbec2526938": {"tier": "IRON"},
        });
        let s = shape("", &seasons);
        assert_eq!(s["map of"], 2);
        assert!(s["each like"]["tier"] == "GOLD" || s["each like"]["tier"] == "IRON");
        assert!(!s.to_string().contains("b639ec2f") && !s.to_string().contains("aaaaaaaa"));
    }

    #[test]
    fn an_id_is_not_spelled_out_even_when_it_is_the_only_key() {
        // One account, one session: a map with a single id for a key.
        let one = json!({ "b639ec2f-e2a0-509a-85f2-ffbec2526938": { "tier": "GOLD" } });
        let s = shape("", &one).to_string();
        assert!(!s.contains("b639ec2f"), "{s}");
        assert!(s.contains("GOLD"), "the shape inside is still told: {s}");
        // Ordinary long key names are not mistaken for ids.
        let named = json!({ "persistLoginEnabled": true, "lol_blue_essence": 3 });
        let n = shape("", &named);
        assert_eq!(n["persistLoginEnabled"], "bool");
        assert_eq!(n["lol_blue_essence"], "number");
    }

    #[test]
    fn a_settings_file_key_that_is_an_id_is_not_spelled_out() {
        let file = "accounts:\n    b639ec2f-e2a0-509a-85f2-ffbec2526938:\n        region: \"TR\"\n";
        let outline = yaml_outline(file).join("\n");
        assert!(!outline.contains("b639ec2f"), "{outline}");
        assert!(outline.contains("accounts:") && outline.contains("region: text(2)"), "{outline}");
    }

    #[test]
    fn a_settings_file_is_outlined_without_its_values() {
        let file = "riot-login:\n    persist:\n        region: \"TR\"\n        session:\n            cookies:\n            -   domain: \"auth.riotgames.com\"\n                value: \"gizli-token-degeri\"\n";
        let outline = yaml_outline(file).join("\n");
        assert!(outline.contains("riot-login:") && outline.contains("cookies:") && outline.contains("value: text(18)"), "{outline}");
        assert!(!outline.contains("gizli") && !outline.contains("auth.riotgames"), "{outline}");
    }
}
