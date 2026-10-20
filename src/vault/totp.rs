use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;
use sha2::{Sha256, Sha512};
use zeroize::Zeroizing;

#[derive(Clone, Copy)]
enum Algorithm {
    Sha1,
    Sha256,
    Sha512,
}

struct Config {
    secret: Zeroizing<Vec<u8>>,
    algorithm: Algorithm,
    digits: u32,
    period: u64,
}

/// The code a seed is holding at `now`, and how long it has left.
pub fn at(input: &str, now: u64) -> Result<(Zeroizing<String>, u64), &'static str> {
    code(parse(input)?, now)
}

/// What a URI calls itself, for naming a code nobody typed a name for: the
/// `issuer` it carries, else the label before the `:` — `Vercel:me@x` is Vercel.
/// A bare seed has no name at all.
pub fn label(input: &str) -> Option<String> {
    let uri = url::Url::parse(input.trim()).ok()?;
    if uri.scheme() != "otpauth" || uri.host_str() != Some("totp") {
        return None;
    }
    let issuer = uri
        .query_pairs()
        .find(|(key, _)| key == "issuer")
        .map(|(_, value)| value.trim().to_owned())
        .filter(|issuer| !issuer.is_empty());
    issuer.or_else(|| {
        // The path keeps its escapes (`ACME%20Co`), the query does not.
        let path = uri.path().trim_start_matches('/');
        let name = path.split(':').next().unwrap_or(path).trim();
        let name = percent_encoding::percent_decode_str(name)
            .decode_utf8_lossy()
            .into_owned();
        let name = name.trim().to_owned();
        (!name.is_empty()).then_some(name)
    })
}

fn parse(input: &str) -> Result<Config, &'static str> {
    let input = input.trim();
    if input.is_empty() {
        return Err("TOTP secret is empty");
    }

    let (secret, algorithm, digits, period) = if input
        .get(..10)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("otpauth://"))
    {
        let uri = url::Url::parse(input).map_err(|_| "invalid TOTP URI")?;
        if uri.scheme() != "otpauth" || uri.host_str() != Some("totp") || uri.path().len() < 2 {
            return Err("invalid TOTP URI");
        }

        let mut secret = None;
        let mut algorithm = None;
        let mut digits = None;
        let mut period = None;
        for (key, value) in uri.query_pairs() {
            match key.as_ref() {
                "secret" => {
                    if secret.replace(Zeroizing::new(value.into_owned())).is_some() {
                        return Err("duplicate TOTP secret");
                    }
                }
                "algorithm" => {
                    if algorithm.replace(Algorithm::parse(&value)?).is_some() {
                        return Err("duplicate TOTP algorithm");
                    }
                }
                "digits" => {
                    let value = value
                        .parse::<u32>()
                        .map_err(|_| "invalid TOTP digit count")?;
                    if !(6..=8).contains(&value) {
                        return Err("TOTP digits must be between 6 and 8");
                    }
                    if digits.replace(value).is_some() {
                        return Err("duplicate TOTP digit count");
                    }
                }
                "period" => {
                    let value = value.parse::<u64>().map_err(|_| "invalid TOTP period")?;
                    if value == 0 {
                        return Err("TOTP period must be greater than zero");
                    }
                    if period.replace(value).is_some() {
                        return Err("duplicate TOTP period");
                    }
                }
                "encoder" if !value.eq_ignore_ascii_case("plain") => {
                    return Err("unsupported TOTP encoder");
                }
                _ => {}
            }
        }
        (
            secret.ok_or("TOTP URI has no secret")?,
            algorithm.unwrap_or(Algorithm::Sha1),
            digits.unwrap_or(6),
            period.unwrap_or(30),
        )
    } else {
        (Zeroizing::new(input.to_owned()), Algorithm::Sha1, 6, 30)
    };

    Ok(Config {
        secret: base32(&secret)?,
        algorithm,
        digits,
        period,
    })
}

impl Algorithm {
    fn parse(value: &str) -> Result<Self, &'static str> {
        match value.to_ascii_uppercase().as_str() {
            "SHA1" => Ok(Self::Sha1),
            "SHA256" => Ok(Self::Sha256),
            "SHA512" => Ok(Self::Sha512),
            _ => Err("unsupported TOTP algorithm"),
        }
    }
}

fn base32(value: &str) -> Result<Zeroizing<Vec<u8>>, &'static str> {
    if value.len() > 1024 {
        return Err("TOTP secret is too long");
    }
    let unpadded = value.trim_end_matches('=');
    let padding = value.len() - unpadded.len();
    let expected_padding = match unpadded.len() % 8 {
        0 => 0,
        2 => 6,
        4 => 4,
        5 => 3,
        7 => 1,
        _ => return Err("invalid Base32 TOTP secret"),
    };
    if padding > 0 && padding != expected_padding {
        return Err("invalid Base32 TOTP secret");
    }

    let mut output = Zeroizing::new(Vec::with_capacity(unpadded.len() * 5 / 8));
    let (mut buffer, mut bits) = (0u32, 0u32);
    for byte in unpadded.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a',
            b'2'..=b'7' => byte - b'2' + 26,
            _ => return Err("invalid Base32 TOTP secret"),
        };
        buffer = (buffer << 5) | u32::from(value);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            output.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    if output.is_empty() || (bits > 0 && buffer != 0) {
        return Err("invalid Base32 TOTP secret");
    }
    Ok(output)
}

fn code(config: Config, now: u64) -> Result<(Zeroizing<String>, u64), &'static str> {
    let counter = (now / config.period).to_be_bytes();
    let digest = match config.algorithm {
        Algorithm::Sha1 => {
            let mut mac =
                Hmac::<Sha1>::new_from_slice(&config.secret).map_err(|_| "invalid TOTP secret")?;
            mac.update(&counter);
            Zeroizing::new(mac.finalize().into_bytes().to_vec())
        }
        Algorithm::Sha256 => {
            let mut mac = Hmac::<Sha256>::new_from_slice(&config.secret)
                .map_err(|_| "invalid TOTP secret")?;
            mac.update(&counter);
            Zeroizing::new(mac.finalize().into_bytes().to_vec())
        }
        Algorithm::Sha512 => {
            let mut mac = Hmac::<Sha512>::new_from_slice(&config.secret)
                .map_err(|_| "invalid TOTP secret")?;
            mac.update(&counter);
            Zeroizing::new(mac.finalize().into_bytes().to_vec())
        }
    };
    let offset = usize::from(digest[digest.len() - 1] & 0x0f);
    let binary = (u32::from(digest[offset] & 0x7f) << 24)
        | (u32::from(digest[offset + 1]) << 16)
        | (u32::from(digest[offset + 2]) << 8)
        | u32::from(digest[offset + 3]);
    let value = binary % 10u32.pow(config.digits);
    let formatted = Zeroizing::new(format!("{value:0width$}", width = config.digits as usize));
    let remaining = config.period - now % config.period;
    Ok((formatted, remaining))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_rfc_6238_sha1_sha256_and_sha512_vectors() {
        let vectors = [
            ("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ", "SHA1", "94287082"),
            (
                "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZA",
                "SHA256",
                "46119246",
            ),
            (
                concat!(
                    "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ",
                    "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ",
                    "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ",
                    "GEZDGNA="
                ),
                "SHA512",
                "90693936",
            ),
        ];
        for (secret, algorithm, expected) in vectors {
            let uri = format!("otpauth://totp/Test?secret={secret}&algorithm={algorithm}&digits=8");
            let (actual, remaining) = at(&uri, 59).unwrap();
            assert_eq!(actual.as_str(), expected);
            assert_eq!(remaining, 1);
        }
    }

    #[test]
    fn accepts_plain_base32_and_uri_parameters() {
        let (plain, _) = at("JBSWY3DPEHPK3PXP", 30).unwrap();
        let (uri, remaining) = at(
            "otpauth://totp/GitHub:user?secret=JBSWY3DPEHPK3PXP&digits=7&period=60&issuer=GitHub",
            59,
        )
        .unwrap();
        assert_eq!(plain.len(), 6);
        assert_eq!(uri.len(), 7);
        assert_eq!(remaining, 1);
    }

    /// A URI names its account: the issuer first, else the label before the `:`.
    /// A bare seed names nothing, so the pane keeps the placeholder.
    #[test]
    fn a_uri_names_itself() {
        assert_eq!(
            label("otpauth://totp/Vercel:me@example.com?secret=JBSWY3DPEHPK3PXP&issuer=Vercel")
                .as_deref(),
            Some("Vercel")
        );
        assert_eq!(
            label("otpauth://totp/GitHub:octocat?secret=JBSWY3DPEHPK3PXP").as_deref(),
            Some("GitHub"),
            "no issuer, so the label before the colon"
        );
        assert_eq!(
            label("otpauth://totp/mi%20cuenta?secret=JBSWY3DPEHPK3PXP").as_deref(),
            Some("mi cuenta"),
            "an escape in the path is a character, not three"
        );
        assert_eq!(
            label("otpauth://totp/ACME%20Co:jdoe@example.com?secret=JBSWY3DPEHPK3PXP&issuer=ACME%20Co")
                .as_deref(),
            Some("ACME Co"),
            "and the issuer comes decoded out of the query"
        );
        assert_eq!(label("otpauth://totp/?secret=JBSWY3DPEHPK3PXP"), None);
        assert_eq!(label("otpauth://hotp/x?secret=JBSWY3DPEHPK3PXP"), None);
        assert_eq!(label("JBSWY3DPEHPK3PXP"), None, "a bare seed says nothing");
    }

    #[test]
    fn rejects_malformed_and_unsupported_secrets() {
        for input in [
            "",
            "not-base32!",
            "otpauth://hotp/Test?secret=JBSWY3DPEHPK3PXP",
            "otpauth://totp/Test?secret=JBSWY3DPEHPK3PXP&algorithm=MD5",
            "otpauth://totp/Test?secret=JBSWY3DPEHPK3PXP&digits=5",
            "otpauth://totp/Test?secret=JBSWY3DPEHPK3PXP&period=0",
            "otpauth://totp/Test?secret=JBSWY3DPEHPK3PXP&encoder=steam",
        ] {
            assert!(at(input, 0).is_err(), "accepted {input}");
        }
    }
}
