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
    /// Account id read out of a token, when a token happens to be a JWT
    /// that names its account. Less certain than `sub`: it is only ever
    /// used to recognise an account we already know, never to rule one out.
    pub hint: Option<String>,
}

pub fn read_file_session(riot_games_dir: &Path) -> FileSession {
    match std::fs::read_to_string(riot_games_dir.join(PRIVATE_SETTINGS)) {
        Ok(text) => {
            let (has_tokens, sub) = parse(&text);
            FileSession { exists: true, has_tokens, sub, hint: token_account(&text) }
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

/// The account id that the tokens in the file agree on, if any of them is
/// a JWT carrying one. `None` when there is none, or when they disagree.
pub fn token_account(text: &str) -> Option<String> {
    let mut found: Option<String> = None;
    for line in text.lines() {
        let Some((_, v)) = line.split_once(':') else { continue };
        let Some(sub) = jwt_sub(&unquote(v)) else { continue };
        match &found {
            Some(seen) if *seen != sub => return None,
            _ => found = Some(sub),
        }
    }
    found
}

/// The `sub` claim of a JWT, if `token` is one and the claim looks like a
/// Riot account id (a UUID). Nothing is verified: this reads, it does not trust.
fn jwt_sub(token: &str) -> Option<String> {
    let mut parts = token.split('.');
    let (_, payload) = (parts.next()?, parts.next()?);
    if payload.len() < 16 || payload.len() > 8192 {
        return None;
    }
    let claims: serde_json::Value = serde_json::from_slice(&base64url(payload)?).ok()?;
    let sub = claims.get("sub")?.as_str()?;
    let shape_ok = sub.len() == 36
        && sub.bytes().enumerate().all(|(i, b)| if matches!(i, 8 | 13 | 18 | 23) { b == b'-' } else { b.is_ascii_hexdigit() });
    shape_ok.then(|| sub.to_ascii_lowercase())
}

/// Decodes unpadded (or padded) base64url. `None` on any other character.
fn base64url(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for b in text.trim_end_matches('=').bytes() {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
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
    fn a_token_that_names_its_account_gives_a_hint() {
        // {"alg":"none"} . {"sub":"b639ec2f-e2a0-509a-85f2-ffbec2526938","x":1} . sig
        let jwt = "eyJhbGciOiJub25lIn0.eyJzdWIiOiJiNjM5ZWMyZi1lMmEwLTUwOWEtODVmMi1mZmJlYzI1MjY5MzgiLCJ4IjoxfQ.c2ln";
        let file = format!("psl:\n    authorization:\n        riot-client:\n            id_token: \"{jwt}\"\n            refresh_token: \"opaque-value\"\n");
        assert_eq!(token_account(&file).as_deref(), Some("b639ec2f-e2a0-509a-85f2-ffbec2526938"));
        assert_eq!(parse(&file), (true, None), "the certain id is still only the cookie");

        // No JWT, a JWT without an account id, and a `sub` that is not an account id.
        assert_eq!(token_account("refresh_token: \"opaque.value.here\"\n"), None);
        assert_eq!(token_account("id_token: \"eyJhbGciOiJub25lIn0.eyJ4IjoxfQ.c2ln\"\n"), None);
        // {"sub":"riot-client"}
        assert_eq!(token_account("id_token: \"eyJhbGciOiJub25lIn0.eyJzdWIiOiJyaW90LWNsaWVudCJ9.c2ln\"\n"), None);
        // Two tokens for two different accounts: no hint at all.
        let other = "eyJhbGciOiJub25lIn0.eyJzdWIiOiJhYWFhYWFhYS1lMmEwLTUwOWEtODVmMi1mZmJlYzI1MjY5MzgifQ.c2ln";
        assert_eq!(token_account(&format!("a: \"{jwt}\"\nb: \"{other}\"\n")), None);
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
