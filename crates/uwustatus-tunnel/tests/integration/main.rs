use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;
use uwustatus_tunnel::proto::{self, MAX_MESSAGE};
use uwustatus_tunnel::{
    Assignment, CodeError, Identity, MainClient, MainEvent, MainMessage, MainSettings, PairingCode, Refusal,
    SatelliteEvent, SatelliteListener, SatelliteSettings, Timing, Token,
};

const WAIT: Duration = Duration::from_secs(10);

fn timing() -> Timing {
    Timing {
        keep_alive: Duration::from_millis(200),
        idle_timeout: Duration::from_secs(2),
        retry_min: Duration::from_millis(20),
        retry_max: Duration::from_millis(100),
        refused_retry: Duration::from_millis(100),
        handshake_timeout: Duration::from_secs(5),
    }
}

fn satellite(token: Option<Token>) -> (SatelliteListener, UnboundedReceiver<SatelliteEvent>) {
    let mut settings = SatelliteSettings::new((Ipv4Addr::LOCALHOST, 0).into(), Identity::generate().unwrap());
    settings.token = token;
    settings.timing = timing();
    settings.satellite_version = "test".into();
    SatelliteListener::bind(settings).unwrap()
}

fn code_of(listener: &SatelliteListener) -> PairingCode {
    PairingCode::parse(&listener.pairing_code(&[Ipv4Addr::LOCALHOST.into()]).unwrap()).unwrap()
}

fn main_from(mut settings: MainSettings) -> (MainClient, UnboundedReceiver<MainEvent>) {
    settings.timing = timing();
    MainClient::start(settings)
}

async fn within<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(WAIT, future).await.expect("timed out")
}

/// The next event that `pick` takes, skipping the others.
async fn next<E, T>(events: &mut UnboundedReceiver<E>, mut pick: impl FnMut(E) -> Option<T>) -> T {
    within(async {
        loop {
            if let Some(found) = pick(events.recv().await.expect("events ended")) {
                return found;
            }
        }
    })
    .await
}

async fn until(mut condition: impl FnMut() -> bool) {
    within(async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
}

/// A paired, connected pair.
async fn paired() -> (SatelliteListener, UnboundedReceiver<SatelliteEvent>, MainClient, UnboundedReceiver<MainEvent>) {
    let (listener, mut sat_events) = satellite(Some(Token::generate()));
    let (main, mut main_events) =
        main_from(MainSettings::from_code(&code_of(&listener), Identity::generate().unwrap()));
    next(&mut main_events, |e| matches!(e, MainEvent::Connected { .. }).then_some(())).await;
    next(&mut sat_events, |e| matches!(e, SatelliteEvent::Connected { .. }).then_some(())).await;
    (listener, sat_events, main, main_events)
}

#[tokio::test]
async fn pairing_pins_the_main_instance_and_uses_up_the_token() {
    let (listener, mut sat_events) = satellite(Some(Token::generate()));
    let code = code_of(&listener);
    assert_eq!(code.fingerprint, listener.fingerprint());
    let identity = Identity::generate().unwrap();
    let (main, mut main_events) = main_from(MainSettings::from_code(&code, identity.clone()));

    assert_eq!(next(&mut main_events, |e| matches!(e, MainEvent::Paired).then_some(())).await, ());
    let version = next(&mut main_events, |e| match e {
        MainEvent::Connected { satellite_version, .. } => Some(satellite_version),
        _ => None,
    })
    .await;
    assert_eq!(version, "test");
    let pinned = next(&mut sat_events, |e| match e {
        SatelliteEvent::Paired { main } => Some(main),
        _ => None,
    })
    .await;
    assert_eq!(pinned, identity.fingerprint());
    assert_eq!(listener.paired_main(), Some(identity.fingerprint()));
    assert!(listener.pairing_code(&[Ipv4Addr::LOCALHOST.into()]).is_none(), "the token is used up");

    // Assignments and the dead man's switch arrive, and again after a reconnect.
    let checks = vec![Assignment { id: "web".into(), interval_s: 60, check: json!({"type": "http"}) }];
    main.assign(checks.clone());
    main.dead_man(json!({"after_s": 300}));
    let assigned = next(&mut sat_events, |e| match e {
        SatelliteEvent::Assign(checks) => Some(checks),
        _ => None,
    })
    .await;
    assert_eq!(assigned, checks);
    next(&mut sat_events, |e| matches!(e, SatelliteEvent::DeadMan(_)).then_some(())).await;

    listener.disconnect();
    next(&mut sat_events, |e| matches!(e, SatelliteEvent::Lost { .. }).then_some(())).await;
    next(&mut sat_events, |e| matches!(e, SatelliteEvent::Connected { .. }).then_some(())).await;
    let again = next(&mut sat_events, |e| match e {
        SatelliteEvent::Assign(checks) => Some(checks),
        _ => None,
    })
    .await;
    assert_eq!(again, checks);

    // A second main instance with the same, used code is refused.
    let (_other, mut other_events) = main_from(MainSettings::from_code(&code, Identity::generate().unwrap()));
    let reason = next(&mut other_events, |e| match e {
        MainEvent::Refused { reason, .. } => Some(reason),
        _ => None,
    })
    .await;
    assert_eq!(reason, Refusal::OtherMain);
    assert!(main.is_connected(), "the paired one stays connected");
}

#[tokio::test]
async fn a_wrong_token_or_none_is_refused() {
    let (listener, _events) = satellite(Some(Token::generate()));
    let mut code = code_of(&listener);
    code.token = Token::generate();
    let (_main, mut events) = main_from(MainSettings::from_code(&code, Identity::generate().unwrap()));
    let reason = next(&mut events, |e| match e {
        MainEvent::Refused { reason, .. } => Some(reason),
        _ => None,
    })
    .await;
    assert_eq!(reason, Refusal::WrongToken);

    let (_main, mut events) =
        main_from(MainSettings::pinned(code.addresses.clone(), code.fingerprint, Identity::generate().unwrap()));
    let reason = next(&mut events, |e| match e {
        MainEvent::Refused { reason, .. } => Some(reason),
        _ => None,
    })
    .await;
    assert_eq!(reason, Refusal::NotPaired);
    assert_eq!(listener.paired_main(), None);
}

#[tokio::test]
async fn a_wrong_fingerprint_fails_the_handshake() {
    let (listener, _events) = satellite(Some(Token::generate()));
    let mut code = code_of(&listener);
    code.fingerprint = Identity::generate().unwrap().fingerprint();
    let (main, mut events) = main_from(MainSettings::from_code(&code, Identity::generate().unwrap()));
    let error = next(&mut events, |e| match e {
        MainEvent::Unreachable { error } => Some(error),
        _ => None,
    })
    .await;
    assert!(error.contains("certificate") || error.contains("handshake"), "{error}");
    assert!(!main.is_connected());
    assert_eq!(listener.paired_main(), None, "the token is still there for the real main instance");
}

#[test]
fn the_code_checksum_catches_typos() {
    let code = PairingCode {
        addresses: vec!["192.0.2.10:4443".parse().unwrap(), "[2001:db8::10]:4443".parse().unwrap()],
        fingerprint: Identity::generate().unwrap().fingerprint(),
        token: Token::generate(),
    };
    let text = code.encode();
    assert!(text.starts_with("uwusat1"));
    assert!(text.chars().all(|c| c.is_ascii_alphanumeric()), "double-click selects the whole code");
    let parsed = PairingCode::parse(&text).unwrap();
    assert_eq!(parsed.addresses, code.addresses);
    assert_eq!(parsed.fingerprint, code.fingerprint);
    assert!(parsed.token.matches(&code.token));
    let (a, b) = text.split_at(30);
    assert!(PairingCode::parse(&format!(" {}\n{}- ", a.to_uppercase(), b)).is_ok(), "copied with breaks");

    assert_eq!(PairingCode::parse("uwugw1abc").unwrap_err(), CodeError::NotACode);
    assert_eq!(PairingCode::parse(&text[..text.len() - 3]).unwrap_err(), CodeError::Damaged);
    for at in [10, 40, text.len() - 1] {
        let mut typo: Vec<char> = text.chars().collect();
        typo[at] = if typo[at] == 'a' { 'b' } else { 'a' };
        assert_eq!(PairingCode::parse(&typo.into_iter().collect::<String>()).unwrap_err(), CodeError::Damaged);
    }
    assert_eq!(format!("{:?}", code.token), "Token(…)");
}

#[tokio::test]
async fn results_come_again_after_a_reconnect_until_acked() {
    let (listener, mut sat_events, main, mut main_events) = paired().await;
    let first = listener.push_result("web", 1, json!({"up": true})).unwrap();
    let got = next(&mut main_events, |e| match e {
        MainEvent::Results(items) => Some(items),
        _ => None,
    })
    .await;
    assert_eq!(got.iter().map(|i| i.seq).collect::<Vec<_>>(), [first]);

    // Lost before the ack; more results while away.
    listener.disconnect();
    next(&mut sat_events, |e| matches!(e, SatelliteEvent::Lost { .. }).then_some(())).await;
    let second = listener.push_result("web", 2, json!({"up": false})).unwrap();
    assert!(second > first);
    let mut seen = Vec::new();
    while !seen.contains(&second) {
        let items = next(&mut main_events, |e| match e {
            MainEvent::Results(items) => Some(items),
            _ => None,
        })
        .await;
        seen.extend(items.iter().map(|i| i.seq));
    }
    assert!(seen.contains(&first), "the unacknowledged one comes again: {seen:?}");
    assert_eq!(listener.buffered(), 2);

    assert!(main.ack(second));
    until(|| listener.buffered() == 0).await;

    // After the ack a reconnect brings only what is new.
    listener.disconnect();
    next(&mut main_events, |e| matches!(e, MainEvent::Connected { .. }).then_some(())).await;
    let third = listener.push_result("web", 3, json!({})).unwrap();
    let items = next(&mut main_events, |e| match e {
        MainEvent::Results(items) => Some(items),
        _ => None,
    })
    .await;
    assert_eq!(items.iter().map(|i| i.seq).collect::<Vec<_>>(), [third]);
}

#[tokio::test]
async fn the_buffer_keeps_the_newest() {
    let mut settings = SatelliteSettings::new((Ipv4Addr::LOCALHOST, 0).into(), Identity::generate().unwrap());
    settings.token = Some(Token::generate());
    settings.timing = timing();
    settings.buffer_limit = 3;
    let (listener, _events) = SatelliteListener::bind(settings).unwrap();
    let seqs: Vec<u64> = (0..5).map(|i| listener.push_result("a", i, json!(i)).unwrap()).collect();
    assert_eq!(listener.buffered(), 3);
    assert!(listener.push_result("a", 9, json!("x".repeat(MAX_MESSAGE))).is_none(), "too big to ever send");

    let (_main, mut events) = main_from(MainSettings::from_code(&code_of(&listener), Identity::generate().unwrap()));
    let mut seen = Vec::new();
    while seen.len() < 3 {
        let items = next(&mut events, |e| match e {
            MainEvent::Results(items) => Some(items),
            _ => None,
        })
        .await;
        seen.extend(items.iter().map(|i| i.seq));
    }
    assert_eq!(seen, seqs[2..]);
}

#[tokio::test]
async fn broken_frames_are_errors() {
    // Too big: refused from the length alone.
    let mut frame = ((MAX_MESSAGE + 1) as u32).to_be_bytes().to_vec();
    frame.extend_from_slice(b"{}");
    let error = proto::read_message::<_, MainMessage>(&mut frame.as_slice(), MAX_MESSAGE).await.unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    // Not JSON, cut short, or JSON of no message.
    for body in [&b"\xff\xfe garbage"[..], b"{\"type\":\"hello\"", b"{\"type\":\"hello\",\"version\":\"one\"}"] {
        let mut frame = (body.len() as u32).to_be_bytes().to_vec();
        frame.extend_from_slice(body);
        assert!(proto::read_message::<_, MainMessage>(&mut frame.as_slice(), MAX_MESSAGE).await.is_err());
    }
    let mut short = 10u32.to_be_bytes().to_vec();
    short.extend_from_slice(b"{}");
    assert!(proto::read_message::<_, MainMessage>(&mut short.as_slice(), MAX_MESSAGE).await.is_err());
    // A message from a newer version is skipped, not fatal.
    let mut unknown = Vec::new();
    proto::write_message(&mut unknown, &serde_json::json!({"type": "from_the_future"})).await.unwrap();
    assert_eq!(proto::read_known::<_, MainMessage>(&mut unknown.as_slice(), MAX_MESSAGE).await.unwrap(), None);
    assert!(proto::write_message(&mut Vec::new(), &"x".repeat(MAX_MESSAGE)).await.is_err());

    // Over the network: a stranger sending garbage or an oversized hello is hung up on, and the
    // satellite still pairs afterwards.
    let (listener, _events) = satellite(Some(Token::generate()));
    let address = listener.local_addr().unwrap();
    for frame in [vec![0xff; 4], [&8000u32.to_be_bytes()[..], &[b' '; 8000]].concat()] {
        let error = raw_hello(address, listener.fingerprint(), &frame).await;
        assert!(!error.is_empty());
    }
    let (_main, mut events) = main_from(MainSettings::from_code(&code_of(&listener), Identity::generate().unwrap()));
    next(&mut events, |e| matches!(e, MainEvent::Connected { .. }).then_some(())).await;
}

/// Sends `frame` as the hello and waits for the satellite to hang up.
async fn raw_hello(address: SocketAddr, satellite: uwustatus_tunnel::Fingerprint, frame: &[u8]) -> String {
    let endpoint = quinn::Endpoint::client((Ipv4Addr::LOCALHOST, 0).into()).unwrap();
    let config = uwustatus_tunnel::client_config(&Identity::generate().unwrap(), satellite, &timing()).unwrap();
    let connection = within(endpoint.connect_with(config, address, "uwustatus-satellite").unwrap()).await.unwrap();
    let (mut send, _recv) = connection.open_bi().await.unwrap();
    let _ = send.write_all(frame).await;
    within(connection.closed()).await.to_string()
}
