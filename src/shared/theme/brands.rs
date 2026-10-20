#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Brand {
    Github,
    Gitlab,
    Bitbucket,
    Google,
    Apple,
    Microsoft,
    Amazon,
    Aws,
    Netflix,
    Spotify,
    Discord,
    Slack,
    Twitch,
    Youtube,
    X,
    Instagram,
    Facebook,
    Linkedin,
    Telegram,
    Whatsapp,
    Dropbox,
    Drive,
    Onedrive,
    Protonmail,
    Paypal,
    Steam,
    Reddit,
    Cloudflare,
    Ebay,
    Shopify,
    Zoom,
    Stripe,
}

/// Outcome of the icon picker popup (`vault::picker`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    None,
    Set(Brand),
    Clear,
}

impl Brand {
    pub const ALL: [Brand; 32] = [
        Brand::Github,
        Brand::Gitlab,
        Brand::Bitbucket,
        Brand::Google,
        Brand::Apple,
        Brand::Microsoft,
        Brand::Amazon,
        Brand::Aws,
        Brand::Netflix,
        Brand::Spotify,
        Brand::Discord,
        Brand::Slack,
        Brand::Twitch,
        Brand::Youtube,
        Brand::X,
        Brand::Instagram,
        Brand::Facebook,
        Brand::Linkedin,
        Brand::Telegram,
        Brand::Whatsapp,
        Brand::Dropbox,
        Brand::Drive,
        Brand::Onedrive,
        Brand::Protonmail,
        Brand::Paypal,
        Brand::Steam,
        Brand::Reddit,
        Brand::Cloudflare,
        Brand::Ebay,
        Brand::Shopify,
        Brand::Zoom,
        Brand::Stripe,
    ];

    /// The simple-icons slug, persisted as `Item::icon`.
    pub fn slug(self) -> &'static str {
        match self {
            Brand::Github => "github",
            Brand::Gitlab => "gitlab",
            Brand::Bitbucket => "bitbucket",
            Brand::Google => "google",
            Brand::Apple => "apple",
            Brand::Microsoft => "microsoft",
            Brand::Amazon => "amazon",
            Brand::Aws => "amazonaws",
            Brand::Netflix => "netflix",
            Brand::Spotify => "spotify",
            Brand::Discord => "discord",
            Brand::Slack => "slack",
            Brand::Twitch => "twitch",
            Brand::Youtube => "youtube",
            Brand::X => "x",
            Brand::Instagram => "instagram",
            Brand::Facebook => "facebook",
            Brand::Linkedin => "linkedin",
            Brand::Telegram => "telegram",
            Brand::Whatsapp => "whatsapp",
            Brand::Dropbox => "dropbox",
            Brand::Drive => "googledrive",
            Brand::Onedrive => "microsoftonedrive",
            Brand::Protonmail => "protonmail",
            Brand::Paypal => "paypal",
            Brand::Steam => "steam",
            Brand::Reddit => "reddit",
            Brand::Cloudflare => "cloudflare",
            Brand::Ebay => "ebay",
            Brand::Shopify => "shopify",
            Brand::Zoom => "zoom",
            Brand::Stripe => "stripe",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Brand::Github => "GitHub",
            Brand::Gitlab => "GitLab",
            Brand::Bitbucket => "Bitbucket",
            Brand::Google => "Google",
            Brand::Apple => "Apple",
            Brand::Microsoft => "Microsoft",
            Brand::Amazon => "Amazon",
            Brand::Aws => "AWS",
            Brand::Netflix => "Netflix",
            Brand::Spotify => "Spotify",
            Brand::Discord => "Discord",
            Brand::Slack => "Slack",
            Brand::Twitch => "Twitch",
            Brand::Youtube => "YouTube",
            Brand::X => "X (Twitter)",
            Brand::Instagram => "Instagram",
            Brand::Facebook => "Facebook",
            Brand::Linkedin => "LinkedIn",
            Brand::Telegram => "Telegram",
            Brand::Whatsapp => "WhatsApp",
            Brand::Dropbox => "Dropbox",
            Brand::Drive => "Google Drive",
            Brand::Onedrive => "OneDrive",
            Brand::Protonmail => "ProtonMail",
            Brand::Paypal => "PayPal",
            Brand::Steam => "Steam",
            Brand::Reddit => "Reddit",
            Brand::Cloudflare => "Cloudflare",
            Brand::Ebay => "eBay",
            Brand::Shopify => "Shopify",
            Brand::Zoom => "Zoom",
            Brand::Stripe => "Stripe",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Brand> {
        Some(match slug {
            "github" => Brand::Github,
            "gitlab" => Brand::Gitlab,
            "bitbucket" => Brand::Bitbucket,
            "google" => Brand::Google,
            "apple" => Brand::Apple,
            "microsoft" => Brand::Microsoft,
            "amazon" => Brand::Amazon,
            "amazonaws" => Brand::Aws,
            "netflix" => Brand::Netflix,
            "spotify" => Brand::Spotify,
            "discord" => Brand::Discord,
            "slack" => Brand::Slack,
            "twitch" => Brand::Twitch,
            "youtube" => Brand::Youtube,
            "x" => Brand::X,
            "instagram" => Brand::Instagram,
            "facebook" => Brand::Facebook,
            "linkedin" => Brand::Linkedin,
            "telegram" => Brand::Telegram,
            "whatsapp" => Brand::Whatsapp,
            "dropbox" => Brand::Dropbox,
            "googledrive" => Brand::Drive,
            "microsoftonedrive" => Brand::Onedrive,
            "protonmail" => Brand::Protonmail,
            "paypal" => Brand::Paypal,
            "steam" => Brand::Steam,
            "reddit" => Brand::Reddit,
            "cloudflare" => Brand::Cloudflare,
            "ebay" => Brand::Ebay,
            "shopify" => Brand::Shopify,
            "zoom" => Brand::Zoom,
            "stripe" => Brand::Stripe,
            _ => return None,
        })
    }
}

/// Domain → brand hint for an entry without an explicit icon; `Item::icon` wins.
pub fn brand_for_url(url: &str) -> Option<Brand> {
    let host = url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split(['/', ':'])
        .next()
        .unwrap_or("")
        .trim_start_matches("www.")
        .to_lowercase();
    // No `format!` per suffix: this runs for every row on every frame, and a
    // temporary `.{suffix}` string per brand was the allocation hot spot.
    let m = |suffix: &str| {
        host == suffix
            || (host.len() > suffix.len()
                && host.ends_with(suffix)
                && host.as_bytes()[host.len() - suffix.len() - 1] == b'.')
    };
    // specific domains before the broader ones they share
    if m("drive.google.com") {
        return Some(Brand::Drive);
    }
    if m("onedrive.live.com") {
        return Some(Brand::Onedrive);
    }
    if m("github.com") {
        return Some(Brand::Github);
    }
    if m("gitlab.com") {
        return Some(Brand::Gitlab);
    }
    if m("bitbucket.org") {
        return Some(Brand::Bitbucket);
    }
    if m("google.com") || m("gmail.com") {
        return Some(Brand::Google);
    }
    if m("apple.com") || m("icloud.com") {
        return Some(Brand::Apple);
    }
    if m("microsoft.com") || m("live.com") || m("outlook.com") || m("office.com") {
        return Some(Brand::Microsoft);
    }
    if m("amazonaws.com") || m("aws.amazon.com") || m("cloudfront.net") {
        return Some(Brand::Aws);
    }
    if host.contains("amazon.") {
        return Some(Brand::Amazon);
    }
    if m("netflix.com") {
        return Some(Brand::Netflix);
    }
    if m("spotify.com") {
        return Some(Brand::Spotify);
    }
    if m("discord.com") || m("discord.gg") {
        return Some(Brand::Discord);
    }
    if m("slack.com") {
        return Some(Brand::Slack);
    }
    if m("twitch.tv") {
        return Some(Brand::Twitch);
    }
    if m("youtube.com") {
        return Some(Brand::Youtube);
    }
    if m("twitter.com") || m("x.com") {
        return Some(Brand::X);
    }
    if m("instagram.com") {
        return Some(Brand::Instagram);
    }
    if m("facebook.com") || m("fb.com") {
        return Some(Brand::Facebook);
    }
    if m("linkedin.com") {
        return Some(Brand::Linkedin);
    }
    if m("telegram.org") || m("t.me") {
        return Some(Brand::Telegram);
    }
    if m("whatsapp.com") {
        return Some(Brand::Whatsapp);
    }
    if m("dropbox.com") {
        return Some(Brand::Dropbox);
    }
    if m("proton.me") || m("protonmail.com") {
        return Some(Brand::Protonmail);
    }
    if m("paypal.com") {
        return Some(Brand::Paypal);
    }
    if m("steamcommunity.com") || m("store.steampowered.com") {
        return Some(Brand::Steam);
    }
    if m("reddit.com") {
        return Some(Brand::Reddit);
    }
    if m("cloudflare.com") {
        return Some(Brand::Cloudflare);
    }
    if m("ebay.com") {
        return Some(Brand::Ebay);
    }
    if m("shopify.com") || m("myshopify.com") {
        return Some(Brand::Shopify);
    }
    if m("zoom.us") {
        return Some(Brand::Zoom);
    }
    if m("stripe.com") {
        return Some(Brand::Stripe);
    }
    None
}

pub fn brand_of(item: &crate::vault::model::Login) -> Option<Brand> {
    item.icon
        .as_deref()
        .and_then(Brand::from_slug)
        .or_else(|| brand_for_url(&item.url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_suggests_brands() {
        assert_eq!(
            brand_for_url("https://github.com/octocat"),
            Some(Brand::Github)
        );
        assert_eq!(
            brand_for_url("https://drive.google.com/u/0/"),
            Some(Brand::Drive)
        );
        assert_eq!(
            brand_for_url("https://onedrive.live.com/"),
            Some(Brand::Onedrive)
        );
        assert_eq!(
            brand_for_url("open.spotify.com/track/1"),
            Some(Brand::Spotify)
        );
        assert_eq!(
            brand_for_url("http://mystore.myshopify.com/"),
            Some(Brand::Shopify)
        );
        assert_eq!(
            brand_for_url("https://aws.amazon.com/console/"),
            Some(Brand::Aws)
        );
        assert_eq!(
            brand_for_url("https://www.amazon.es/gp/"),
            Some(Brand::Amazon)
        );
        assert_eq!(
            brand_for_url("https://app.slack.com/client"),
            Some(Brand::Slack)
        );
        assert_eq!(
            brand_for_url("https://www.linkedin.com/in/x"),
            Some(Brand::Linkedin)
        );
        assert_eq!(
            brand_for_url("https://outlook.live.com/"),
            Some(Brand::Microsoft)
        );
        assert_eq!(brand_for_url("https://example.com"), None);
        assert_eq!(brand_for_url(""), None);
    }

    #[test]
    fn slug_roundtrip() {
        for brand in Brand::ALL {
            assert_eq!(Brand::from_slug(brand.slug()), Some(brand));
        }
        assert_eq!(Brand::from_slug("nope"), None);
    }
}
