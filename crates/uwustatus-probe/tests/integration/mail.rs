use uwustatus_probe::{CheckKind, MailCheck, MailTls, Status};

use crate::{check, mail_server, silent_server};

const SMTP: &[(&str, &str)] = &[
    ("EHLO", "250-fake.test hi\r\n250-PIPELINING\r\n250 STARTTLS\r\n"),
    ("STARTTLS", "220 go ahead\r\n"),
    ("QUIT", "221 bye\r\n"),
];
const SMTP_NO_TLS: &[(&str, &str)] = &[("EHLO", "250-fake.test hi\r\n250 PIPELINING\r\n")];
const IMAP: &[(&str, &str)] = &[("u1 STARTTLS", "u1 OK begin TLS\r\n"), ("u2 LOGOUT", "* BYE\r\nu2 OK\r\n")];
const IMAP_NO_TLS: &[(&str, &str)] = &[("u1 STARTTLS", "u1 BAD not here\r\n")];

fn mail(port: u16, tls: MailTls, ignore_tls_errors: bool) -> MailCheck {
    MailCheck { host: "127.0.0.1".into(), port, tls, ignore_tls_errors }
}

#[tokio::test]
async fn smtp() {
    let port = mail_server("220-fake.test ESMTP\r\n220 ready\r\n", SMTP, false).await;
    let o = check(3_000, CheckKind::Smtp(mail(port, MailTls::None, false))).await;
    assert_eq!((o.status, o.message.as_str()), (Status::Up, "fake.test ESMTP ready"));
    assert!(o.details.cert_days_left.is_none());

    let o = check(3_000, CheckKind::Smtp(mail(port, MailTls::StartTls, true))).await;
    assert_eq!(o.status, Status::Up, "{}", o.message);
    assert_eq!(o.details.cert_expires_at.unwrap().to_string(), "2040-05-06T00:00:00Z");

    let o = check(3_000, CheckKind::Smtp(mail(port, MailTls::StartTls, false))).await;
    assert_eq!(o.status, Status::Down);
    assert!(o.message.starts_with("TLS:"), "{}", o.message);

    let port = mail_server("220 ready\r\n", SMTP_NO_TLS, false).await;
    let o = check(3_000, CheckKind::Smtp(mail(port, MailTls::StartTls, true))).await;
    assert_eq!((o.status, o.message.as_str()), (Status::Down, "STARTTLS not offered"));

    let port = mail_server("554 go away\r\n", SMTP, false).await;
    let o = check(3_000, CheckKind::Smtp(mail(port, MailTls::None, false))).await;
    assert_eq!((o.status, o.message.as_str()), (Status::Down, "SMTP 554 go away"));

    let port = mail_server("\u{1b}[31mgarbage\r\n", SMTP, false).await;
    let o = check(3_000, CheckKind::Smtp(mail(port, MailTls::None, false))).await;
    assert_eq!(o.status, Status::Down);
    assert!(!o.message.contains('\u{1b}'));

    let port = mail_server("220 tls ready\r\n", SMTP, true).await;
    let o = check(3_000, CheckKind::Smtp(mail(port, MailTls::Tls, true))).await;
    assert_eq!((o.status, o.message.as_str()), (Status::Up, "tls ready"));
    assert!(o.details.cert_days_left.is_some());

    let port = silent_server().await;
    let o = check(300, CheckKind::Smtp(mail(port, MailTls::None, false))).await;
    assert_eq!(o.message, "timed out after 300 ms");
}

#[tokio::test]
async fn imap() {
    let port = mail_server("* OK [CAPABILITY IMAP4rev1 STARTTLS] ready\r\n", IMAP, false).await;
    let o = check(3_000, CheckKind::Imap(mail(port, MailTls::None, false))).await;
    assert_eq!((o.status, o.message.as_str()), (Status::Up, "OK [CAPABILITY IMAP4rev1 STARTTLS] ready"));

    let o = check(3_000, CheckKind::Imap(mail(port, MailTls::StartTls, true))).await;
    assert_eq!(o.status, Status::Up, "{}", o.message);
    assert!(o.details.cert_days_left.is_some());

    let port = mail_server("* OK ready\r\n", IMAP_NO_TLS, false).await;
    let o = check(3_000, CheckKind::Imap(mail(port, MailTls::StartTls, true))).await;
    assert_eq!((o.status, o.message.as_str()), (Status::Down, "STARTTLS: BAD not here"));

    let port = mail_server("* BYE overloaded\r\n", IMAP, false).await;
    let o = check(3_000, CheckKind::Imap(mail(port, MailTls::None, false))).await;
    assert_eq!((o.status, o.message.as_str()), (Status::Down, "IMAP greeting: * BYE overloaded"));

    let port = mail_server("* OK imaps\r\n", IMAP, true).await;
    let o = check(3_000, CheckKind::Imap(mail(port, MailTls::Tls, true))).await;
    assert_eq!(o.status, Status::Up, "{}", o.message);
}
