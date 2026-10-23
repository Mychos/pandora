//! Whether a stored address accepts the page in front of the user: the page is
//! attacker-controlled and never widens what an entry claims.

use crate::vault::model::Login;
use serde::{Deserialize, Serialize};
use url::Url;

/// How an entry's URL claims a site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Rule {
    /// The registrable domain; the default a bare URL always meant.
    #[default]
    Domain,
    /// This exact host, and nothing below it.
    Host,
    /// Anything the stored URL starts with, once normalised.
    Prefix,
    /// Excludes this exact host only, so it drops only the page the user said no to.
    Never,
}

impl Rule {
    pub const ALL: [Rule; 4] = [Rule::Domain, Rule::Host, Rule::Prefix, Rule::Never];

    /// Keeps the default out of saved JSON, so an entry with no extra URL writes
    /// what it always did.
    pub fn is_domain(&self) -> bool {
        matches!(self, Rule::Domain)
    }

    pub fn label(self) -> &'static str {
        match self {
            Rule::Domain => "domain",
            Rule::Host => "host",
            Rule::Prefix => "prefix",
            Rule::Never => "never",
        }
    }
}

/// One more URL an entry answers to, with the rule it is matched by.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Uri {
    pub url: String,
    #[serde(default, skip_serializing_if = "Rule::is_domain")]
    pub rule: Rule,
}

/// Least specific first; `rank` keeps the highest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rank {
    Domain,
    Prefix,
    Host,
}

#[derive(Debug, Clone)]
pub struct Site {
    /// Lowercase, no `www.`, no trailing dot; an IPv6 literal keeps its brackets.
    pub host: String,
    /// eTLD+1, or `None` for an IP, a single-label host or a bare public suffix.
    pub registrable: Option<String>,
    pub url: String,
}

/// `None` when there is no host: empty, a bare scheme, a `data:` or `javascript:` URL.
pub fn parse(input: &str) -> Option<Site> {
    let input = input.trim();
    if input.is_empty() || is_other_scheme(input) {
        return None;
    }
    let text = if input.starts_with("http://") || input.starts_with("https://") {
        input.to_owned()
    } else {
        format!("https://{input}")
    };
    let parsed = Url::parse(&text).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let raw = parsed.host_str()?.trim_end_matches('.');
    let host = raw.strip_prefix("www.").unwrap_or(raw).to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    Some(Site {
        registrable: registrable(bare),
        host,
        url: parsed.as_str().to_owned(),
    })
}

/// `mailto:`, `javascript:`, `data:`: a scheme with no host behind it. A
/// `host:port` is not one of these, and neither is an IPv6 literal.
fn is_other_scheme(input: &str) -> bool {
    if input.starts_with('[') {
        return false;
    }
    let Some((head, rest)) = input.split_once(':') else {
        return false;
    };
    let bare = !head.is_empty() && !head.contains('.') && !head.contains('/');
    let port = !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit());
    bare && !port && !head.eq_ignore_ascii_case("http") && !head.eq_ignore_ascii_case("https")
}

/// The registrable domain, or `None` when the host is an IP, has no dot, or *is* a
/// public suffix; `Domain` then falls back to the exact host.
fn registrable(host: &str) -> Option<String> {
    if host.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }
    psl2::registrable_domain(host)
}

/// The best rank among an entry's URLs, with the URL that earned it. A `Never` URL
/// for the page's host drops the whole entry, whatever else it carries.
/// Whether one of a login's URLs is the site a passkey answers for. A passkey is bound
/// to its relying party id — a registrable domain or a host — which are the same two
/// shapes a login's own addresses take, so the link is made the same way.
pub fn claims(login: &Login, rp_id: &str) -> bool {
    let rp_id = rp_id.trim().trim_end_matches('.').to_ascii_lowercase();
    if rp_id.is_empty() {
        return false;
    }
    login.uris().any(|(url, rule)| {
        if rule == Rule::Never {
            return false;
        }
        let Some(site) = parse(url) else {
            return false;
        };
        site.host == rp_id || site.registrable.as_deref() == Some(rp_id.as_str())
    })
}

pub fn rank<'a>(entry: &'a Login, page: &Site) -> Option<(Rank, &'a str)> {
    let mut best: Option<(Rank, &str)> = None;
    for (url, rule) in entry.uris() {
        let Some(site) = parse(url) else { continue };
        let hit = match rule {
            Rule::Never if site.host == page.host => return None,
            Rule::Never => None,
            Rule::Domain if covers(&site, page) => Some(Rank::Domain),
            Rule::Host if site.host == page.host => Some(Rank::Host),
            Rule::Prefix if page.url.starts_with(&site.url) => Some(Rank::Prefix),
            _ => None,
        };
        if let Some(rank) = hit {
            if best.is_none_or(|(best, _)| rank > best) {
                best = Some((rank, url));
            }
        }
    }
    best
}

/// Same site: the same host, or the same registrable domain when both have one.
fn covers(site: &Site, page: &Site) -> bool {
    site.host == page.host || (site.registrable.is_some() && site.registrable == page.registrable)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(url: &str, rule: Rule) -> Login {
        let mut item = Login::new(0);
        item.url = url.to_owned();
        item.url_match = rule;
        item
    }

    fn accepts(url: &str, rule: Rule, page: &str) -> bool {
        parse(page).is_some_and(|page| rank(&item(url, rule), &page).is_some())
    }

    /// A passkey's relying party is the site, not the record: a login on
    /// `mail.google.com` answers for a `google.com` passkey, and one on `github.io`
    /// does not answer for somebody else's tenant.
    #[test]
    fn a_passkey_hangs_from_the_site_the_login_claims() {
        let mut mail = item("https://mail.google.com", Rule::Domain);
        mail.extra_urls.push(Uri {
            url: "https://github.com".to_owned(),
            rule: Rule::Never,
        });
        assert!(claims(&mail, "google.com"), "the registrable domain");
        assert!(claims(&mail, "mail.google.com"), "the host itself");
        assert!(!claims(&mail, "github.com"), "a never rule is not a claim");
        assert!(!claims(&mail, "evil.example"));
        assert!(!claims(&mail, ""));
        let bank = item("https://bank.example", Rule::Domain);
        assert!(
            claims(&bank, "bank.example"),
            "a host with no public suffix"
        );

        let tenant = item("https://ana.github.io", Rule::Domain);
        assert!(
            !claims(&tenant, "other.github.io"),
            "shared suffixes stay apart"
        );
        assert!(claims(&tenant, "ana.github.io"));
    }

    #[test]
    fn parse_reads_a_host_and_a_url_the_same() {
        let site = parse("https://user:pw@www.Example.CO.UK:8443/login?q=1#f").unwrap();
        assert_eq!(site.host, "example.co.uk");
        assert_eq!(site.registrable.as_deref(), Some("example.co.uk"));
        assert_eq!(
            site.url,
            "https://user:pw@www.example.co.uk:8443/login?q=1#f"
        );

        assert_eq!(parse("example.com.").unwrap().host, "example.com");
        assert_eq!(parse("github.com").unwrap().host, "github.com");
        assert_eq!(parse("münchen.de").unwrap().host, "xn--mnchen-3ya.de");
    }

    #[test]
    fn a_host_without_a_dot_has_no_registrable_domain() {
        assert!(parse("localhost").unwrap().registrable.is_none());
        assert!(parse("wiki").unwrap().registrable.is_none());
        assert!(parse("127.0.0.1").unwrap().registrable.is_none());
        assert!(parse("[::1]:3000").unwrap().registrable.is_none());
        assert_eq!(parse("[::1]:3000").unwrap().host, "[::1]");
        assert!(parse("co.uk").unwrap().registrable.is_none());
        assert!(parse("github.io").unwrap().registrable.is_none());
        assert_eq!(
            parse("user.github.io").unwrap().registrable.as_deref(),
            Some("user.github.io")
        );
    }

    #[test]
    fn nothing_without_a_host_parses() {
        for bad in [
            "",
            "   ",
            "javascript:alert(1)",
            "data:text/html,x",
            "mailto:someone@example.com",
            "about:blank",
        ] {
            assert!(parse(bad).is_none(), "{bad} should not parse");
        }
    }

    #[test]
    fn a_colon_is_a_port_not_a_scheme() {
        assert_eq!(parse("example.com:8443").unwrap().host, "example.com");
        assert_eq!(parse("localhost:3000").unwrap().host, "localhost");
    }

    #[test]
    fn parse_is_idempotent() {
        for input in [
            "example.com",
            "https://a.b.example.co.uk:8443/x?y#z",
            "münchen.de",
            "[::1]:8080",
        ] {
            let once = parse(input).unwrap();
            assert_eq!(once.host, parse(&once.host).unwrap().host, "{input}");
        }
    }

    #[test]
    fn a_domain_rule_covers_the_registrable_domain() {
        assert!(accepts("github.com", Rule::Domain, "github.com"));
        assert!(accepts(
            "https://github.com/login",
            Rule::Domain,
            "gist.github.com"
        ));
        assert!(accepts(
            "accounts.google.com",
            Rule::Domain,
            "mail.google.com"
        ));
        assert!(accepts(
            "https://www.example.co.uk/x",
            Rule::Domain,
            "shop.example.co.uk"
        ));
        assert!(!accepts("github.com", Rule::Domain, "github.com.evil.com"));
        assert!(!accepts("github.com", Rule::Domain, "notgithub.com"));
        assert!(
            !accepts("example.co.uk", Rule::Domain, "other.co.uk"),
            "a public suffix is not a site"
        );
        assert!(
            !accepts("github.io", Rule::Domain, "user.github.io"),
            "a public suffix must not sweep its tenants"
        );
    }

    #[test]
    fn a_host_rule_pins_one_hostname() {
        assert!(accepts("example.com", Rule::Host, "example.com"));
        assert!(accepts(
            "https://example.com/login",
            Rule::Host,
            "example.com"
        ));
        assert!(!accepts("github.com", Rule::Host, "gist.github.com"));
        assert!(!accepts("www.example.com", Rule::Host, "cdn.example.com"));
    }

    #[test]
    fn a_prefix_rule_takes_the_start_of_the_url() {
        assert!(accepts(
            "https://example.com/app",
            Rule::Prefix,
            "https://example.com/app/login"
        ));
        assert!(!accepts(
            "https://example.com/app",
            Rule::Prefix,
            "https://example.com/other"
        ));
    }

    #[test]
    fn never_drops_the_whole_entry() {
        let mut entry = item("example.com", Rule::Domain);
        entry.extra_urls = vec![Uri {
            url: "evil.example.com".to_owned(),
            rule: Rule::Never,
        }];
        assert!(
            rank(&entry, &parse("evil.example.com").unwrap()).is_none(),
            "the exclusion wins over the domain rule"
        );
        assert!(
            rank(&entry, &parse("other.example.com").unwrap()).is_some(),
            "the rest of the entry still matches"
        );
    }

    #[test]
    fn the_most_specific_url_ranks_highest() {
        let mut entry = item("example.com", Rule::Domain);
        entry.extra_urls = vec![Uri {
            url: "login.example.com".to_owned(),
            rule: Rule::Host,
        }];
        let hit = rank(&entry, &parse("login.example.com").unwrap()).unwrap();
        assert_eq!(hit.0, Rank::Host);
        assert_eq!(hit.1, "login.example.com");
        assert_eq!(
            rank(&entry, &parse("www.example.com").unwrap()).unwrap().0,
            Rank::Domain
        );
    }

    #[test]
    fn an_entry_without_a_url_never_matches() {
        assert!(rank(&item("", Rule::Domain), &parse("example.com").unwrap()).is_none());
        assert!(rank(&item("   ", Rule::Host), &parse("example.com").unwrap()).is_none());
    }
}
