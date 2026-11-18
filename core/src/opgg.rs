//! Reads an account's public League profile from an OP.GG page.
//!
//! OP.GG is a third-party site that publishes what Riot's public API says
//! about a Riot ID: rank, points, wins and losses, level, profile icon. It
//! has no API we may use, so the page itself is read. That makes this
//! module the part of the app most likely to break without warning: when
//! OP.GG changes its page, ranks simply stop refreshing until this file is
//! updated. Nothing else depends on it.
//!
//! What is relied on, as seen in pages captured in October 2026:
//!
//! - the address is `/lol/summoners/<region>/<name>-<tag>`;
//! - the page's `<meta name="description">` reads
//!   `Name#TAG / Gold 2 45 LP / 120Win 98Lose Win rate 55% / ...` for a
//!   ranked account and `Name#TAG / Lv. 83` for an unranked one;
//! - the profile picture's address holds `profile_icons/profileIcon<N>.`;
//! - a script block holds `"initUpdatedAt": "<ISO time>"`, the moment
//!   OP.GG itself last refreshed the profile.
//!
//! Only the address is sent: a Riot ID and a region, both public.

/// Regions as OP.GG names them in its addresses, with what to show.
pub const REGIONS: [(&str, &str); 15] = [
    ("tr", "TR"),
    ("euw", "EUW"),
    ("eune", "EUNE"),
    ("na", "NA"),
    ("kr", "KR"),
    ("br", "BR"),
    ("jp", "JP"),
    ("oce", "OCE"),
    ("ru", "RU"),
    ("lan", "LAN"),
    ("las", "LAS"),
    ("me", "ME"),
    ("sea", "SEA"),
    ("tw", "TW"),
    ("vn", "VN"),
];

/// `region` as one of ours, whatever case or form it came in: OP.GG's own
/// code, the label, or the name the League client uses ("TR1", "LA2").
pub fn region_code(region: &str) -> Option<&'static str> {
    let r = region.trim().to_ascii_lowercase();
    let known = match r.as_str() {
        "tr1" => "tr",
        "euw1" => "euw",
        "eun1" | "eune1" => "eune",
        "na1" => "na",
        "br1" => "br",
        "jp1" => "jp",
        "oc1" | "oce1" => "oce",
        "ru1" => "ru",
        "la1" => "lan",
        "la2" => "las",
        "me1" => "me",
        "sg2" | "ph2" | "th2" => "sea",
        "tw2" => "tw",
        "vn2" => "vn",
        other => other,
    };
    REGIONS.iter().map(|(code, _)| *code).find(|code| *code == known)
}

/// A guess at the region from a Riot ID's tag: many accounts still carry
/// the default tag of the region they were made in ("#TR1", "#EUW").
pub fn region_from_tag(riot_id: &str) -> Option<&'static str> {
    region_code(riot_id.rsplit_once('#')?.1)
}

/// The page for a Riot ID, or `None` if it is not one (`Name#TAG`) or the
/// region is not known.
pub fn profile_url(region: &str, riot_id: &str) -> Option<String> {
    let region = region_code(region)?;
    let (name, tag) = riot_id.rsplit_once('#')?;
    let (name, tag) = (name.trim(), tag.trim());
    if name.is_empty() || tag.is_empty() {
        return None;
    }
    Some(format!("https://op.gg/lol/summoners/{region}/{}-{}", escape(name), escape(tag)))
}

/// Percent-encodes everything but letters, digits and a few safe marks, so
/// nothing in a name can change what the address means.
fn escape(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// What a profile page said.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Profile {
    /// "Gold II 45 LP", in the form the rest of the app uses; `None` when unranked.
    pub rank: Option<String>,
    pub wins: Option<u32>,
    pub losses: Option<u32>,
    pub level: Option<u64>,
    pub icon: Option<u32>,
    /// When OP.GG last refreshed this profile, in Unix seconds.
    pub updated: Option<u64>,
}

const TIERS: [&str; 10] = ["Iron", "Bronze", "Silver", "Gold", "Platinum", "Emerald", "Diamond", "Master", "Grandmaster", "Challenger"];

/// Reads a profile page. `None` when the page is not a profile at all
/// (not found, an error page, or a layout this module does not know).
pub fn parse(page: &str) -> Option<Profile> {
    let description = meta_description(page)?;
    let mut parts = description.split(" / ");
    // The first part is the Riot ID; a page without one is not a profile.
    if !parts.next()?.contains('#') {
        return None;
    }
    let mut profile = Profile::default();
    for part in parts {
        let part = part.trim();
        if let Some(rank) = rank_from(part) {
            profile.rank = Some(rank);
        } else if let Some((wins, losses)) = record_from(part) {
            profile.wins = Some(wins);
            profile.losses = Some(losses);
        } else if let Some(level) = part.strip_prefix("Lv.").and_then(|l| l.trim().parse().ok()) {
            profile.level = Some(level);
        }
    }
    profile.icon = number_after(page, "profile_icons/profileIcon");
    if profile.level.is_none() {
        profile.level = level_near_icon(page);
    }
    profile.updated = text_after(page, "\"initUpdatedAt\"").and_then(|t| unix_from_iso(&t));
    Some(profile)
}

/// The `content` of the description meta tag, entities undone.
fn meta_description(page: &str) -> Option<String> {
    let at = page.find("name=\"description\"")?;
    // `content` may stand before or after `name` within the same tag.
    let start = page[..at].rfind('<')?;
    let end = at + page[at..].find('>')?;
    let tag = &page[start..end];
    let from = tag.find("content=\"")? + "content=\"".len();
    let to = from + tag[from..].find('"')?;
    Some(tag[from..to].replace("&amp;", "&").replace("&#x27;", "'").replace("&#39;", "'").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">"))
}

/// "Gold 2 45 LP", "Gold II 45 LP" or "Challenger 2,210 LP" as "Gold II 45 LP".
fn rank_from(part: &str) -> Option<String> {
    let mut words = part.split_whitespace();
    let tier = words.next()?;
    let rung = TIERS.iter().position(|t| t.eq_ignore_ascii_case(tier))?;
    let tier = TIERS[rung];
    let rest: Vec<&str> = words.collect();
    // Master and above have no divisions; everything below always has one.
    if rest.len() != if rung >= 7 { 2 } else { 3 } {
        return None;
    }
    let (lp_word, unit) = match rest.as_slice() {
        [.., lp, unit] => (*lp, *unit),
        _ => return None,
    };
    if !unit.eq_ignore_ascii_case("LP") {
        return None;
    }
    let lp: u32 = lp_word.replace(',', "").parse().ok()?;
    let division = match rest.first().copied().filter(|_| rest.len() == 3) {
        Some("1") | Some("I") => " I",
        Some("2") | Some("II") => " II",
        Some("3") | Some("III") => " III",
        Some("4") | Some("IV") => " IV",
        Some(_) => return None,
        None => "",
    };
    Some(format!("{tier}{division} {lp} LP"))
}

/// "120Win 98Lose Win rate 55%" as (120, 98).
fn record_from(part: &str) -> Option<(u32, u32)> {
    let mut words = part.split_whitespace();
    let wins = words.next()?.strip_suffix("Win")?.replace(',', "").parse().ok()?;
    let losses = words.next()?.strip_suffix("Lose")?.replace(',', "").parse().ok()?;
    Some((wins, losses))
}

/// The digits that follow the first `marker` on the page.
fn number_after<T: std::str::FromStr>(page: &str, marker: &str) -> Option<T> {
    let rest = &page[page.find(marker)? + marker.len()..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// The level sits in a small badge right after the profile picture: the
/// first tag after it whose whole text is a number.
fn level_near_icon(page: &str) -> Option<u64> {
    let from = page.find("profile_icons/profileIcon")?;
    let window = &page[from..(from + 2000).min(page.len())];
    // Walk the text between tags.
    let mut rest = window;
    while let Some(open) = rest.find('>') {
        rest = &rest[open + 1..];
        let text = &rest[..rest.find('<').unwrap_or(rest.len())];
        let text = text.trim();
        if !text.is_empty() && text.len() <= 4 && text.bytes().all(|b| b.is_ascii_digit()) {
            return text.parse().ok();
        }
    }
    None
}

/// The quoted text that follows `key` and a colon.
fn text_after(page: &str, key: &str) -> Option<String> {
    let rest = &page[page.find(key)? + key.len()..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_string())
}

/// "2026-10-14T15:52:49+09:00" (or with "Z") as Unix seconds.
pub fn unix_from_iso(text: &str) -> Option<u64> {
    let (date, time) = text.split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>());
    let (y, m, day) = (d.next()?.ok()?, d.next()?.ok()?, d.next()?.ok()?);
    // The offset starts at the last sign or "Z" of the time part.
    let (clock, offset_seconds) = if let Some(clock) = time.strip_suffix('Z') {
        (clock, 0)
    } else if let Some(at) = time.rfind(['+', '-']) {
        let (clock, offset) = time.split_at(at);
        let sign = if offset.starts_with('-') { -1 } else { 1 };
        let mut o = offset[1..].split(':').map(|p| p.parse::<i64>());
        (clock, sign * (o.next()?.ok()? * 3600 + o.next().and_then(Result::ok).unwrap_or(0) * 60))
    } else {
        (time, 0)
    };
    let mut c = clock.split(':');
    let (h, min) = (c.next()?.parse::<i64>().ok()?, c.next()?.parse::<i64>().ok()?);
    let s = c.next().and_then(|s| s.split('.').next()?.parse::<i64>().ok()).unwrap_or(0);
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) || h > 23 || min > 59 || s > 60 {
        return None;
    }
    // Days since 1970-01-01, by the civil-calendar arithmetic.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + h * 3600 + min * 60 + s - offset_seconds).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page in the layout the module relies on, with made-up contents.
    fn page(description: &str, icon: u32, badge: u32, updated: &str) -> String {
        format!(
            r#"<html><head><title>x</title><meta name="description" content="{description}"/></head><body>
<img src="https://static.example/meta/images/profile_icons/profileIcon{icon}.jpg?image=q_auto&v=17"/><div class="a"><span class="b c">{badge}</span></div>
<script type="application/json">{{"puuid": "abc", "initRenewableAt": "2026-10-14T15:54:49+09:00", "initUpdatedAt": "{updated}"}}</script>
</body></html>"#
        )
    }

    #[test]
    fn addresses() {
        assert_eq!(profile_url("tr", "Kedi Sever#TR1").as_deref(), Some("https://op.gg/lol/summoners/tr/Kedi%20Sever-TR1"));
        assert_eq!(profile_url("EUW", "Çay#ÖZ").as_deref(), Some("https://op.gg/lol/summoners/euw/%C3%87ay-%C3%96Z"));
        assert_eq!(profile_url("LA2", "a#b").as_deref(), Some("https://op.gg/lol/summoners/las/a-b"), "the client's own region names");
        // Nothing in a name can step out of its place in the address.
        let odd = profile_url("tr", "../../x?y=1&z#T/1").unwrap();
        assert_eq!(odd, "https://op.gg/lol/summoners/tr/..%2F..%2Fx%3Fy%3D1%26z-T%2F1");
        assert!(crate::net::Gate::permits(crate::net::Purpose::LolRanks, &odd));
        for bad in [("tr", "no tag"), ("tr", "#TR1"), ("tr", "name#"), ("moon", "a#b"), ("", "a#b")] {
            assert_eq!(profile_url(bad.0, bad.1), None, "{bad:?}");
        }
    }

    #[test]
    fn regions() {
        assert_eq!(region_code(" TR "), Some("tr"));
        assert_eq!(region_code("EUN1"), Some("eune"));
        assert_eq!(region_code("oc1"), Some("oce"));
        assert_eq!(region_code("elsewhere"), None);
        assert_eq!(region_from_tag("Ana#TR1"), Some("tr"));
        assert_eq!(region_from_tag("Ana#EUW"), Some("euw"));
        assert_eq!(region_from_tag("Ana#1234"), None, "a tag of one's own says nothing");
        assert_eq!(region_from_tag("Ana"), None);
    }

    #[test]
    fn a_ranked_profile() {
        let p = parse(&page("Kedi Sever#TR1 / Gold 2 45 LP / 120Win 98Lose Win rate 55% / Ahri - 10Win 4Lose Win rate 71%", 4567, 212, "2026-10-14T15:52:49+09:00")).unwrap();
        assert_eq!(p.rank.as_deref(), Some("Gold II 45 LP"));
        assert_eq!((p.wins, p.losses), (Some(120), Some(98)));
        assert_eq!(p.level, Some(212), "from the badge beside the picture");
        assert_eq!(p.icon, Some(4567));
        assert_eq!(p.updated, Some(1_791_960_769));
    }

    #[test]
    fn the_top_tiers_have_no_division_and_big_numbers() {
        let p = parse(&page("A#KR1 / Challenger 2,210 LP / 1,442Win 367Lose Win rate 55%", 6, 945, "2026-10-14T06:52:49Z")).unwrap();
        assert_eq!(p.rank.as_deref(), Some("Challenger 2210 LP"));
        assert_eq!((p.wins, p.losses), (Some(1442), Some(367)));
        let roman = parse(&page("A#EUW / Diamond IV 3 LP / 5Win 5Lose Win rate 50%", 1, 30, "x")).unwrap();
        assert_eq!(roman.rank.as_deref(), Some("Diamond IV 3 LP"));
        assert_eq!(roman.updated, None, "a time that cannot be read is not guessed");
    }

    #[test]
    fn an_unranked_profile() {
        let p = parse(&page("Yeni Hesap#TR1 / Lv. 83", 5098, 83, "2026-10-14T15:52:47+09:00")).unwrap();
        assert_eq!(p.rank, None);
        assert_eq!((p.wins, p.losses, p.level, p.icon), (None, None, Some(83), Some(5098)));
    }

    #[test]
    fn pages_that_are_not_profiles() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("<html><head><title>Not found</title></head></html>"), None);
        assert_eq!(parse(r#"<meta name="description" content="OP.GG is a site about games"/>"#), None, "no Riot ID, no profile");
        // Words that look like a rank but are not one are left alone.
        let p = parse(&page("A#B / Goldfish 2 45 LP / Gold 9 45 LP / Gold 2 LP", 1, 1, "x")).unwrap();
        assert_eq!(p.rank, None);
    }

    #[test]
    fn times() {
        assert_eq!(unix_from_iso("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(unix_from_iso("2026-10-14T15:52:49+09:00"), Some(1_791_960_769));
        assert_eq!(unix_from_iso("2026-10-14T06:52:49Z"), Some(1_791_960_769));
        assert_eq!(unix_from_iso("2026-10-14T03:52:49-03:00"), Some(1_791_960_769));
        assert_eq!(unix_from_iso("2024-02-29T12:00:00.123Z"), Some(1_709_208_000));
        for bad in ["", "yesterday", "2026-13-01T00:00:00Z", "2026-10-14", "2026-10-14T25:00:00Z"] {
            assert_eq!(unix_from_iso(bad), None, "{bad}");
        }
    }
}
