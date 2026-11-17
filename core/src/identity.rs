//! Reads what Riot Client's private settings file says about the saved
//! login, without the client running.
//!
//! Two formats are in the wild. Older clients keep browser-style cookies
//! under `riot-login` (an `ssid` session cookie, and a `sub` cookie holding
//! the account id). Newer ones keep a refresh token under a top-level `psl`
//! key and no account id at all. A small line scanner covers both and
//! avoids a YAML dependency.

use std::path::Path;

pub const PRIVATE_SETTINGS: &str = "Riot Client/Data/RiotGamesPrivateSettings.yaml";

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct FileSession {
    pub exists: bool,
    /// The file holds login tokens, i.e. "stay signed in" was saved.
    pub has_tokens: bool,
    /// Account id (PUUID), only present in the older format.
    pub sub: Option<String>,
}

pub fn read_file_session(riot_games_dir: &Path) -> FileSession {
    match std::fs::read_to_string(riot_games_dir.join(PRIVATE_SETTINGS)) {
        Ok(text) => {
            let (has_tokens, sub) = parse(&text);
            FileSession { exists: true, has_tokens, sub }
        }
        Err(_) => FileSession::default(),
    }
}

/// Returns (has_tokens, sub).
pub fn parse(text: &str) -> (bool, Option<String>) {
    let mut has_tokens = false;
    let mut sub = None;

    // Cookie currently being read: a `- ` list item with `name` and `value`
    // in either order.
    let mut name: Option<String> = None;
    let mut value: Option<String> = None;
    let flush = |name: &mut Option<String>, value: &mut Option<String>, has_tokens: &mut bool, sub: &mut Option<String>| {
        if let (Some(n), Some(v)) = (name.take(), value.take()) {
            if !is_empty(&v) {
                match n.as_str() {
                    "ssid" => *has_tokens = true,
                    "sub" => *sub = Some(v),
                    _ => {}
                }
            }
        }
        *name = None;
        *value = None;
    };

    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let body = if let Some(rest) = t.strip_prefix('-') {
            flush(&mut name, &mut value, &mut has_tokens, &mut sub);
            rest.trim()
        } else {
            t
        };
        let Some((k, v)) = body.split_once(':') else { continue };
        let (k, v) = (k.trim().trim_matches(['"', '\'']), unquote(v));
        match k {
            "name" => name = Some(v),
            "value" => value = Some(v),
            "private" | "access_token" | "refresh_token" | "id_token" => has_tokens |= !is_empty(&v),
            _ => {}
        }
    }
    flush(&mut name, &mut value, &mut has_tokens, &mut sub);
    (has_tokens, sub)
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let s = s
        .strip_prefix('"')
        .and_then(|x| x.strip_suffix('"'))
        .or_else(|| s.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')))
        .unwrap_or(s);
    s.to_string()
}

fn is_empty(v: &str) -> bool {
    matches!(v.trim(), "" | "{}" | "[]" | "~") || v.trim().eq_ignore_ascii_case("null")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_format() {
        let t = r#"
riot-login:
    persist:
        session:
            cookies:
                -   domain: "auth.riotgames.com"
                    name: "ssid"
                    value: "eyJ.abc.def"
                -   value: "1111-puuid"
                    name: "sub"
                -   name: "tdid"
                    value: "x"
"#;
        assert_eq!(parse(t), (true, Some("1111-puuid".into())));
    }

    #[test]
    fn psl_format() {
        let t = "psl:\n    authorization:\n        riot-client:\n            refresh_token: \"abc\"\nriot-login:\n    persist: null\n";
        assert_eq!(parse(t), (true, None));
    }

    #[test]
    fn signed_out_files() {
        assert_eq!(parse(""), (false, None));
        assert_eq!(parse("riot-login:\n    persist: null\n"), (false, None));
        // Only a tracking cookie, and empty token fields.
        let t = "private: ''\npsl:\n    authorization:\n        riot-client:\n            refresh_token: \"\"\nriot-login:\n    persist:\n        session:\n            cookies:\n                -   name: \"tdid\"\n                    value: \"x\"\n                -   name: \"ssid\"\n                    value: \"\"\n";
        assert_eq!(parse(t), (false, None));
    }
}
