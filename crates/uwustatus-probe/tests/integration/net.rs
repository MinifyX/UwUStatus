use uwustatus_probe::{CheckKind, PingCheck, Status, TcpCheck};

use crate::{check, listener};

#[tokio::test]
async fn tcp_open_and_closed() {
    let (l, port) = listener().await;
    let o = check(2_000, CheckKind::Tcp(TcpCheck { host: "127.0.0.1".into(), port })).await;
    assert_eq!(o.status, Status::Up, "{}", o.message);
    drop(l);
    let o = check(2_000, CheckKind::Tcp(TcpCheck { host: "127.0.0.1".into(), port })).await;
    assert_eq!(o.status, Status::Down);
}

#[tokio::test]
async fn ping_localhost() {
    let o = check(2_000, CheckKind::Ping(PingCheck { host: "127.0.0.1".into() })).await;
    if o.message.starts_with("ICMP not permitted") {
        eprintln!("skip: {}", o.message);
        return;
    }
    assert_eq!((o.status, o.message.as_str()), (Status::Up, "reply from 127.0.0.1"));
    let o = check(2_000, CheckKind::Ping(PingCheck { host: "unresolvable.invalid".into() })).await;
    assert_eq!(o.status, Status::Down);
}
