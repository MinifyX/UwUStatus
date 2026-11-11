use std::net::Ipv4Addr;

use hickory_proto::op::{Message, ResponseCode};
use hickory_proto::rr::rdata::{A, TXT};
use hickory_proto::rr::{RData, Record, RecordType};
use tokio::net::UdpSocket;
use uwustatus_probe::{CheckKind, DnsCheck, Status};

use crate::check;

/// Fake-DNS: `up.test` A → 192.0.2.1 + 192.0.2.2, `txt.test` TXT, sonst NXDOMAIN.
async fn dns_server() -> String {
    let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = sock.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        let mut buf = [0u8; 1500];
        loop {
            let (n, peer) = sock.recv_from(&mut buf).await.unwrap();
            let Ok(query) = Message::from_vec(&buf[..n]) else { continue };
            let q = query.queries[0].clone();
            let mut resp = query.into_response();
            resp.metadata.recursion_available = true;
            let name = q.name().to_string();
            match (name.as_str(), q.query_type()) {
                ("up.test.", RecordType::A) => {
                    for ip in [Ipv4Addr::new(192, 0, 2, 1), Ipv4Addr::new(192, 0, 2, 2)] {
                        resp.add_answer(Record::from_rdata(q.name().clone(), 60, RData::A(A(ip))));
                    }
                }
                ("txt.test.", RecordType::TXT) => {
                    resp.add_answer(Record::from_rdata(
                        q.name().clone(),
                        60,
                        RData::TXT(TXT::new(vec!["v=uwu1".into()])),
                    ));
                }
                _ => resp.metadata.response_code = ResponseCode::NXDomain,
            }
            let _ = sock.send_to(&resp.to_vec().unwrap(), peer).await;
        }
    });
    addr
}

fn dns(resolver: &str, name: &str, rtype: &str, expected: &[&str]) -> CheckKind {
    CheckKind::Dns(DnsCheck {
        name: name.into(),
        record_type: rtype.into(),
        resolver: Some(resolver.into()),
        expected: expected.iter().map(|s| s.to_string()).collect(),
    })
}

#[tokio::test]
async fn records() {
    let r = dns_server().await;
    let o = check(3_000, dns(&r, "up.test", "a", &[])).await;
    assert_eq!(o.status, Status::Up, "{}", o.message);
    assert_eq!(o.details.dns_answers, ["192.0.2.1", "192.0.2.2"]);

    let o = check(3_000, dns(&r, "up.test.", "A", &["192.0.2.2"])).await;
    assert_eq!(o.status, Status::Up, "{}", o.message);

    let o = check(3_000, dns(&r, "up.test", "A", &["192.0.2.9"])).await;
    assert_eq!(o.status, Status::Down);
    assert!(o.message.contains("192.0.2.9"), "{}", o.message);

    let o = check(3_000, dns(&r, "txt.test", "TXT", &["v=uwu1"])).await;
    assert_eq!(o.status, Status::Up, "{} {:?}", o.message, o.details.dns_answers);

    let o = check(3_000, dns(&r, "nx.test", "A", &[])).await;
    assert_eq!(o.status, Status::Down);

    let o = check(3_000, dns(&r, "up.test", "BOGUS", &[])).await;
    assert!(o.message.contains("unknown record type"), "{}", o.message);
    let o = check(3_000, dns("not-an-ip", "up.test", "A", &[])).await;
    assert!(o.message.contains("invalid resolver"), "{}", o.message);
}
