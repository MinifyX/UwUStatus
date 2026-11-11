//! Minimaler DER-Leser für `notAfter` – mehr brauchen wir vom Zertifikat nicht.

use jiff::civil::{Date, DateTime, Time};
use jiff::{Timestamp, tz::TimeZone};

/// `notAfter` eines X.509-Zertifikats (DER). `None` bei allem, was nicht passt.
pub fn not_after(der: &[u8]) -> Option<Timestamp> {
    let (_, cert, _) = tlv(der)?;
    let (_, tbs, _) = tlv(cert)?;
    let mut rest = tbs;
    if rest.first() == Some(&0xa0) {
        rest = tlv(rest)?.2; // [0] version
    }
    rest = tlv(rest)?.2; // serialNumber
    rest = tlv(rest)?.2; // signature
    rest = tlv(rest)?.2; // issuer
    let (tag, validity, _) = tlv(rest)?;
    if tag != 0x30 {
        return None;
    }
    let (_, _, after) = tlv(validity)?; // notBefore
    let (tag, time, _) = tlv(after)?;
    parse_time(tag, time)
}

/// (Tag, Inhalt, Rest)
fn tlv(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = input.split_first()?;
    let (&len, mut rest) = rest.split_first()?;
    let len = if len < 0x80 {
        usize::from(len)
    } else {
        let n = usize::from(len & 0x7f);
        if n == 0 || n > 4 {
            return None;
        }
        let (bytes, r) = rest.split_at_checked(n)?;
        rest = r;
        bytes.iter().fold(0usize, |acc, &b| (acc << 8) | usize::from(b))
    };
    let (content, rest) = rest.split_at_checked(len)?;
    Some((tag, content, rest))
}

/// UTCTime (0x17, `YYMMDDHHMMSSZ`) oder GeneralizedTime (0x18, `YYYYMMDDHHMMSSZ`).
fn parse_time(tag: u8, s: &[u8]) -> Option<Timestamp> {
    let (year, rest) = match (tag, s.len()) {
        (0x17, 13) => {
            let yy = num(&s[..2])?;
            (if yy < 50 { 2000 + yy } else { 1900 + yy }, &s[2..])
        }
        (0x18, 15) => (num(&s[..4])?, &s[4..]),
        _ => return None,
    };
    if rest[10] != b'Z' {
        return None;
    }
    let p = |i: usize| num(&rest[i..i + 2]).and_then(|v| i8::try_from(v).ok());
    let date = Date::new(year.try_into().ok()?, p(0)?, p(2)?).ok()?;
    let time = Time::new(p(4)?, p(6)?, p(8)?, 0).ok()?;
    Some(DateTime::from_parts(date, time).to_zoned(TimeZone::UTC).ok()?.timestamp())
}

fn num(d: &[u8]) -> Option<i32> {
    d.iter().try_fold(0i32, |acc, &b| b.is_ascii_digit().then(|| acc * 10 + i32::from(b - b'0')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(parse_time(0x17, b"300101120000Z").unwrap().to_string(), "2030-01-01T12:00:00Z");
        assert_eq!(parse_time(0x17, b"991231235959Z").unwrap().to_string(), "1999-12-31T23:59:59Z");
        assert_eq!(parse_time(0x18, b"20991231235959Z").unwrap().to_string(), "2099-12-31T23:59:59Z");
        assert!(parse_time(0x17, b"301301120000Z").is_none());
        assert!(parse_time(0x17, b"30010112000+Z").is_none());
        assert!(parse_time(0x18, b"300101120000Z").is_none());
    }

    #[test]
    fn real_cert() {
        let mut p = rcgen::CertificateParams::new(vec!["localhost".into()]).unwrap();
        p.not_after = rcgen::date_time_ymd(2040, 5, 6);
        let der = p.self_signed(&rcgen::KeyPair::generate().unwrap()).unwrap().der().to_vec();
        assert_eq!(not_after(&der).unwrap().to_string(), "2040-05-06T00:00:00Z");
        // Jedes Präfix und jede Ein-Byte-Mutation darf nicht paniken.
        for i in 0..der.len() {
            let _ = not_after(&der[..i]);
            let mut m = der.clone();
            m[i] ^= 0xff;
            let _ = not_after(&m);
        }
    }

    #[test]
    fn fuzz_random() {
        let mut x = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..20_000 {
            let len = (x % 64) as usize;
            let buf: Vec<u8> = (0..len)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    // Häufig gültige Tags/Längen, damit der Parser tief kommt.
                    [0x30, 0x02, 0x17, 0x18, 0xa0, 0x81, 0x84, (x >> 8) as u8][(x % 8) as usize]
                })
                .collect();
            let _ = not_after(&buf);
        }
    }
}
