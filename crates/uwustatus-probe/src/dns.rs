//! DNS-Abfrage über hickory-resolver, ohne Cache.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use hickory_resolver::config::{NameServerConfig, ResolveHosts, ResolverConfig};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::proto::rr::RecordType;
use hickory_resolver::{Resolver, TokioResolver};

use crate::{Details, DnsCheck, Outcome, Status, err_chain, short};

pub(crate) async fn dns(c: &DnsCheck, timeout: Duration) -> Result<Outcome, String> {
    let rtype: RecordType = c
        .record_type
        .trim()
        .to_ascii_uppercase()
        .parse()
        .map_err(|_| format!("unknown record type {:?}", c.record_type))?;
    let mut builder = match c.resolver.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(r) => {
            let addr = r
                .parse::<SocketAddr>()
                .or_else(|_| r.parse::<IpAddr>().map(|ip| SocketAddr::new(ip, 53)))
                .map_err(|_| format!("invalid resolver {r:?}"))?;
            let mut ns = NameServerConfig::udp_and_tcp(addr.ip());
            for conn in &mut ns.connections {
                conn.port = addr.port();
            }
            let mut b = Resolver::builder_with_config(
                ResolverConfig::from_name_servers(vec![ns]),
                TokioRuntimeProvider::default(),
            );
            b.options_mut().use_hosts_file = ResolveHosts::Never;
            b
        }
        None => TokioResolver::builder_tokio().map_err(|e| format!("system resolver: {}", err_chain(&e)))?,
    };
    let opts = builder.options_mut();
    opts.cache_size = 0;
    opts.timeout = timeout;
    let resolver = builder.build().map_err(|e| err_chain(&e))?;

    // Vollqualifiziert, damit keine Suchdomänen angehängt werden.
    let name = if c.name.ends_with('.') { c.name.clone() } else { format!("{}.", c.name) };
    let lookup = resolver.lookup(name.as_str(), rtype).await.map_err(|e| short(&err_chain(&e)))?;
    let answers: Vec<String> = lookup
        .answers()
        .iter()
        .filter(|r| r.record_type() == rtype)
        .take(50)
        .map(|r| short(&r.data.to_string()))
        .collect();
    let norm = |s: &str| s.trim().trim_end_matches('.').trim_matches('"').to_ascii_lowercase();
    let have: Vec<String> = answers.iter().map(|a| norm(a)).collect();
    let missing: Vec<&str> = c.expected.iter().filter(|e| !have.contains(&norm(e))).map(String::as_str).collect();
    let details = Details { dns_answers: answers.clone(), ..Details::default() };
    let outcome = if answers.is_empty() {
        Outcome::new(Status::Down, format!("no {rtype} records"))
    } else if !missing.is_empty() {
        Outcome::new(Status::Down, format!("expected {} not in answer: {}", missing.join(", "), answers.join(", ")))
    } else {
        Outcome::new(Status::Up, answers.join(", "))
    };
    Ok(outcome.with(details))
}
