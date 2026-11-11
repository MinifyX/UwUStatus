//! HTML, feeds and badges for the public status pages. No templates: plain `format!` with
//! escaping, every string from the database goes through `esc`.

use crate::engine::State as MonState;
use crate::incidents::{self, Incident};
use crate::pages::Lang;
use crate::status::{Day, MaintenanceView, Overall, View};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use std::fmt::Write as _;

pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

struct Texts {
    lang: &'static str,
    skip: &'static str,
    ok: &'static str,
    partial: &'static str,
    outage: &'static str,
    maintenance: &'static str,
    updated: &'static str,
    services: &'static str,
    incidents: &'static str,
    planned: &'static str,
    running: &'static str,
    scheduled: &'static str,
    history: &'static str,
    no_history: &'static str,
    since: &'static str,
    resolved_at: &'static str,
    bars: &'static str,
    days_ago: &'static str,
    days_ago_short: &'static str,
    today: &'static str,
    availability: &'static str,
    no_data: &'static str,
    down: &'static str,
    degraded: &'static str,
    latency: &'static str,
    back: &'static str,
    not_found: &'static str,
    state: fn(MonState) -> &'static str,
}

fn state_de(s: MonState) -> &'static str {
    match s {
        MonState::Up => "Betriebsbereit",
        MonState::Degraded => "Eingeschränkt",
        MonState::Down => "Ausfall",
        MonState::Maintenance => "Wartung",
        MonState::Paused => "Pausiert",
        MonState::Pending => "Noch keine Daten",
    }
}

fn state_en(s: MonState) -> &'static str {
    match s {
        MonState::Up => "Operational",
        MonState::Degraded => "Degraded",
        MonState::Down => "Outage",
        MonState::Maintenance => "Maintenance",
        MonState::Paused => "Paused",
        MonState::Pending => "No data yet",
    }
}

const DE: Texts = Texts {
    lang: "de",
    skip: "Zum Inhalt",
    ok: "Alle Systeme laufen",
    partial: "Teilweise gestört",
    outage: "Größere Störung",
    maintenance: "Wartungsarbeiten",
    updated: "Stand",
    services: "Dienste",
    incidents: "Aktuelle Vorfälle",
    planned: "Geplante Wartung",
    running: "Läuft gerade",
    scheduled: "Geplant",
    history: "Vorfall-Archiv",
    no_history: "Keine Vorfälle in diesem Zeitraum.",
    since: "seit",
    resolved_at: "behoben",
    bars: "Verfügbarkeit der letzten 90 Tage",
    days_ago: "vor 90 Tagen",
    days_ago_short: "vor 30 Tagen",
    today: "Heute",
    availability: "Verfügbarkeit",
    no_data: "Keine Daten",
    down: "Ausfall",
    degraded: "Eingeschränkt",
    latency: "Antwortzeit",
    back: "Zur Statusseite",
    not_found: "Keine Statusseite unter dieser Adresse.",
    state: state_de,
};

const EN: Texts = Texts {
    lang: "en",
    skip: "Skip to content",
    ok: "All systems operational",
    partial: "Partial outage",
    outage: "Major outage",
    maintenance: "Under maintenance",
    updated: "Updated",
    services: "Services",
    incidents: "Current incidents",
    planned: "Planned maintenance",
    running: "In progress",
    scheduled: "Scheduled",
    history: "Incident history",
    no_history: "No incidents in this period.",
    since: "since",
    resolved_at: "resolved",
    bars: "Availability over the last 90 days",
    days_ago: "90 days ago",
    days_ago_short: "30 days ago",
    today: "Today",
    availability: "Availability",
    no_data: "No data",
    down: "Outage",
    degraded: "Degraded",
    latency: "Response time",
    back: "Back to status page",
    not_found: "No status page at this address.",
    state: state_en,
};

fn texts(lang: Lang) -> &'static Texts {
    match lang {
        Lang::De => &DE,
        Lang::En => &EN,
    }
}

fn time(v: &View, at: i64) -> String {
    let Ok(t) = jiff::Timestamp::from_second(at) else { return String::new() };
    let fmt = if v.lang == Lang::De { "%d.%m.%Y, %H:%M %Z" } else { "%Y-%m-%d %H:%M %Z" };
    t.to_zoned(v.tz.clone()).strftime(fmt).to_string()
}

fn time_tag(v: &View, at: i64) -> String {
    format!("<time datetime=\"{}\">{}</time>", crate::iso(at), esc(&time(v, at)))
}

fn duration(seconds: i64) -> String {
    let m = (seconds + 59) / 60;
    match (m / 60, m % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

fn percent(lang: Lang, value: f64) -> String {
    let s = if value >= 100.0 { "100".into() } else { format!("{:.2}", (value * 100.0).floor() / 100.0) };
    let s = if lang == Lang::De { s.replace('.', ",") } else { s };
    format!("{s}\u{a0}%")
}

fn shell(v: &View, title: &str, main: &str) -> String {
    let t = texts(v.lang);
    let p = &v.page.config;
    let base = esc(&v.base);
    let logo = if v.page.has_logo { format!("<img src=\"{base}/logo\" alt=\"\">") } else { String::new() };
    let description = if p.description.is_empty() || !main.contains("class=\"card hero") {
        String::new()
    } else {
        format!("<p class=\"desc\">{}</p>", esc(&p.description))
    };
    format!(
        "<!doctype html>\n<html lang=\"{lang}\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>{title}</title><meta name=\"robots\" content=\"noindex\">\
<link rel=\"icon\" href=\"data:,\"><link rel=\"stylesheet\" href=\"{base}/_status/status.css\">\
<link rel=\"alternate\" type=\"application/atom+xml\" title=\"Atom\" href=\"{base}/feed.atom\">\
<link rel=\"alternate\" type=\"application/rss+xml\" title=\"RSS\" href=\"{base}/feed.rss\">\
<script src=\"{base}/_status/status.js\" defer></script></head>\
<body class=\"accent-{accent}\"><a class=\"skip\" href=\"#main\">{skip}</a><div class=\"wrap\">\
<header class=\"top\">{logo}<h1><a href=\"{base}/\">{page_title}</a></h1></header>{description}\
<main id=\"main\">{main}</main><footer><a href=\"{base}/history\">{history}</a>\
<a href=\"{base}/feed.atom\">Atom</a><a href=\"{base}/feed.rss\">RSS</a><span>UwUStatus</span></footer></div></body></html>\n",
        lang = t.lang,
        title = esc(title),
        accent = p.accent.as_str(),
        skip = t.skip,
        page_title = esc(&p.title),
        history = t.history,
    )
}

const NYU_HAPPY: &str = include_str!("../assets/status/nyu-happy.svg");
// The Suite has no "worried" Nyu; puzzled is the closest.
const NYU_PUZZLED: &str = include_str!("../assets/status/nyu-puzzled.svg");
const NYU_SAD: &str = include_str!("../assets/status/nyu-sad.svg");
const NYU_SLEEPY: &str = include_str!("../assets/status/nyu-sleepy.svg");

fn day_bar(v: &View, day: &Day, last: bool) -> String {
    let t = texts(v.lang);
    let class = match day {
        Day { uptime: None, .. } => "none",
        Day { down_s, .. } if *down_s > 0 => "down",
        Day { degraded_s, .. } if *degraded_s > 0 => "degraded",
        _ => "up",
    };
    let date = jiff::civil::Date::strptime("%Y-%m-%d", &day.date)
        .map(|d| d.strftime(if v.lang == Lang::De { "%d.%m.%Y" } else { "%b %d, %Y" }).to_string())
        .unwrap_or_else(|_| day.date.clone());
    let mut tip = date;
    match day.uptime {
        None => {
            let _ = write!(tip, "<br>{}", t.no_data);
        }
        Some(u) => {
            let _ = write!(tip, "<br>{}: {}", t.availability, percent(v.lang, u));
            if day.down_s > 0 {
                let _ = write!(tip, "<br>{}: {}", t.down, duration(day.down_s));
            }
            if day.degraded_s > 0 {
                let _ = write!(tip, "<br>{}: {}", t.degraded, duration(day.degraded_s));
            }
        }
    }
    let tabindex = if last { 0 } else { -1 };
    format!("<li class=\"d-{class}\" tabindex=\"{tabindex}\"><span class=\"tip\">{tip}</span></li>")
}

fn hero(v: &View) -> String {
    let t = texts(v.lang);
    let (class, text, nyu) = match v.overall {
        Overall::Ok => ("ok", t.ok, NYU_HAPPY),
        Overall::Partial => ("partial", t.partial, NYU_PUZZLED),
        Overall::Outage => ("outage", t.outage, NYU_SAD),
        Overall::Maintenance => ("maintenance", t.maintenance, NYU_SLEEPY),
    };
    let figure = if v.page.config.nyu { nyu.trim().to_owned() } else { "<span class=\"dot\"></span>".into() };
    format!(
        "<section class=\"card hero {class}\">{figure}<div><h2>{text}</h2><p>{}: {}</p></div></section>",
        t.updated,
        time_tag(v, v.now)
    )
}

fn incident_html(v: &View, incident: &Incident) -> String {
    let t = texts(v.lang);
    let name = |id: i64| v.name(id);
    let title = incident.display_title(v.lang, &name);
    let mut updates = String::new();
    for u in incident.updates.iter().rev() {
        let body = incident.display_body(u, v.lang, &name);
        let _ = write!(
            updates,
            "<li><strong>{}</strong>{}{}</li>",
            incidents::status_text(v.lang, &u.status),
            time_tag(v, u.at),
            if body.is_empty() { String::new() } else { format!("<p>{}</p>", esc(&body)) }
        );
    }
    let when = match incident.resolved {
        Some(r) => format!("{} – {} {}", time_tag(v, incident.started), t.resolved_at, time_tag(v, r)),
        None => format!("{} {}", t.since, time_tag(v, incident.started)),
    };
    format!(
        "<article class=\"card incident\" id=\"incident-{id}\"><h3>{title}</h3><p class=\"when\">\
<span class=\"pill s-{status}\">{label}</span> {when}</p><ol class=\"updates\">{updates}</ol></article>",
        id = incident.id,
        title = esc(&title),
        status = esc(&incident.status),
        label = incidents::status_text(v.lang, &incident.status),
    )
}

fn maintenance_html(v: &View, m: &MaintenanceView) -> String {
    let t = texts(v.lang);
    let label = if m.running { t.running } else { t.scheduled };
    let description = if m.description.is_empty() { String::new() } else { format!("<p>{}</p>", esc(&m.description)) };
    format!(
        "<article class=\"card incident\"><h3>{}</h3><p class=\"when\"><span class=\"pill s-maintenance\">{label}</span> \
{} – {}</p>{description}</article>",
        esc(&m.title),
        time_tag(v, m.start),
        time_tag(v, m.end)
    )
}

pub fn index(v: &View) -> String {
    let t = texts(v.lang);
    let mut main = hero(v);
    if !v.open.is_empty() {
        let _ = write!(main, "<h2 class=\"section\">{}</h2>", t.incidents);
        for incident in &v.open {
            main.push_str(&incident_html(v, incident));
        }
    }
    if !v.maintenance.is_empty() {
        let _ = write!(main, "<h2 class=\"section\">{}</h2>", t.planned);
        for m in &v.maintenance {
            main.push_str(&maintenance_html(v, m));
        }
    }
    let _ = write!(
        main,
        "<h2 class=\"section\">{}</h2><ul class=\"legend\"><li><i class=\"d-up\"></i>{}</li>\
<li><i class=\"d-degraded\"></i>{}</li><li><i class=\"d-down\"></i>{}</li><li><i class=\"d-none\"></i>{}</li></ul>",
        t.services,
        (t.state)(MonState::Up),
        t.degraded,
        t.down,
        t.no_data
    );
    for group in &v.groups {
        main.push_str("<section class=\"card group\">");
        if !group.name.is_empty() {
            let _ = write!(main, "<h3>{}</h3>", esc(&group.name));
        }
        for m in &group.monitors {
            let latency = m
                .latency_ms
                .map(|ms| format!("<span class=\"mon-meta\">{}: {ms} ms</span>", t.latency))
                .unwrap_or_default();
            let bars: String = m.days.iter().enumerate().map(|(i, d)| day_bar(v, d, i + 1 == m.days.len())).collect();
            let uptime = m.uptime.map(|u| percent(v.lang, u)).unwrap_or_else(|| t.no_data.into());
            let _ = write!(
                main,
                "<div class=\"mon\"><div class=\"mon-head\"><span class=\"mon-name\">{name}</span>{latency}\
<span class=\"pill s-{state}\">{label}</span></div><ol class=\"bars\" aria-label=\"{bars_label}: {name}\">{bars}</ol>\
<div class=\"axis\" aria-hidden=\"true\"><span class=\"long\">{ago}</span><span class=\"short\">{ago_short}</span>\
<span>{uptime}</span><span>{today}</span></div><p class=\"sr\">{availability}: {uptime}</p></div>",
                name = esc(&m.name),
                state = m.state.as_str(),
                label = (t.state)(m.state),
                bars_label = t.bars,
                ago = t.days_ago,
                ago_short = t.days_ago_short,
                today = t.today,
                availability = t.bars,
            );
        }
        main.push_str("</section>");
    }
    shell(v, &v.page.config.title, &main)
}

pub fn history(v: &View, all: &[Incident]) -> String {
    let t = texts(v.lang);
    let mut main =
        format!("<p><a href=\"{}/\">← {}</a></p><h2 class=\"section\">{}</h2>", esc(&v.base), t.back, t.history);
    if all.is_empty() {
        let _ = write!(main, "<p class=\"empty\">{}</p>", t.no_history);
    }
    for incident in all {
        main.push_str(&incident_html(v, incident));
    }
    shell(v, &format!("{} – {}", t.history, v.page.config.title), &main)
}

pub fn not_found(lang: Lang) -> Response {
    let t = texts(lang);
    (
        StatusCode::NOT_FOUND,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CONTENT_SECURITY_POLICY, crate::status::PAGE_CSP),
        ],
        format!(
            "<!doctype html>\n<html lang=\"{}\"><meta charset=\"utf-8\"><title>UwUStatus</title><p>{}</p></html>\n",
            t.lang, t.not_found
        ),
    )
        .into_response()
}

const CSS: &str = concat!(include_str!("../assets/status/status.css"), include_str!("../assets/status/nyu.css"));

pub fn asset(file: &str) -> Response {
    let (kind, body): (&str, &'static [u8]) = match file {
        "status.css" => ("text/css; charset=utf-8", CSS.as_bytes()),
        "status.js" => ("text/javascript; charset=utf-8", include_bytes!("../assets/status/status.js")),
        "uwu-sans.woff2" => ("font/woff2", include_bytes!("../assets/status/uwu-sans.woff2")),
        "OFL.txt" => ("text/plain; charset=utf-8", include_bytes!("../assets/status/OFL.txt")),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    ([(header::CONTENT_TYPE, kind), (header::CACHE_CONTROL, "public, max-age=86400")], body).into_response()
}

// ---- Feeds ----

pub struct Feed<'a> {
    pub view: &'a View,
    pub incidents: &'a [Incident],
    /// `https://host` of the page (or the admin origin for previews).
    pub origin: &'a str,
}

struct Item {
    id: String,
    title: String,
    link: String,
    published: i64,
    updated: i64,
    text: String,
}

impl Feed<'_> {
    fn home(&self) -> String {
        format!("{}{}/", self.origin, self.view.base)
    }

    fn items(&self) -> Vec<Item> {
        let v = self.view;
        let t = texts(v.lang);
        let name = |id: i64| v.name(id);
        let mut items: Vec<Item> = self
            .incidents
            .iter()
            .map(|i| Item {
                id: format!("{}{}/history#incident-{}", self.origin, v.base, i.id),
                title: i.display_title(v.lang, &name),
                link: format!("{}{}/history#incident-{}", self.origin, v.base, i.id),
                published: i.started,
                updated: i.updates.last().map_or(i.started, |u| u.at),
                text: i
                    .updates
                    .iter()
                    .rev()
                    .map(|u| {
                        format!(
                            "{} ({}): {}",
                            incidents::status_text(v.lang, &u.status),
                            time(v, u.at),
                            i.display_body(u, v.lang, &name)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            })
            .collect();
        items.extend(v.maintenance.iter().map(|m| Item {
            id: format!("{}#maintenance-{}-{}", self.home(), m.id, m.start),
            title: format!("{}: {}", t.planned, m.title),
            link: self.home(),
            published: m.start.min(v.now),
            updated: m.start.min(v.now),
            text: format!("{} – {}\n{}", time(v, m.start), time(v, m.end), m.description),
        }));
        items.sort_by_key(|i| std::cmp::Reverse(i.updated));
        items
    }

    pub fn atom(&self) -> String {
        let v = self.view;
        let items = self.items();
        let updated = items.iter().map(|i| i.updated).max().unwrap_or(v.page.updated);
        let mut out = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<feed xmlns=\"http://www.w3.org/2005/Atom\" xml:lang=\"{lang}\">\
<title>{title}</title><id>{home}</id><link rel=\"alternate\" type=\"text/html\" href=\"{home}\"/>\
<link rel=\"self\" type=\"application/atom+xml\" href=\"{home}feed.atom\"/><updated>{updated}</updated>\
<author><name>{title}</name></author><generator>UwUStatus</generator>",
            lang = texts(v.lang).lang,
            title = esc(&v.page.config.title),
            home = esc(&self.home()),
            updated = crate::iso(updated),
        );
        for i in items {
            let _ = write!(
                out,
                "<entry><id>{id}</id><title>{title}</title><link rel=\"alternate\" type=\"text/html\" href=\"{link}\"/>\
<published>{published}</published><updated>{updated}</updated><content type=\"text\">{text}</content></entry>",
                id = esc(&i.id),
                title = esc(&i.title),
                link = esc(&i.link),
                published = crate::iso(i.published),
                updated = crate::iso(i.updated),
                text = esc(&i.text),
            );
        }
        out.push_str("</feed>\n");
        out
    }

    pub fn rss(&self) -> String {
        let v = self.view;
        let rfc2822 = |at: i64| {
            jiff::Timestamp::from_second(at)
                .map(|t| t.to_zoned(jiff::tz::TimeZone::UTC).strftime("%a, %d %b %Y %H:%M:%S %z").to_string())
                .unwrap_or_default()
        };
        let description =
            if v.page.config.description.is_empty() { &v.page.config.title } else { &v.page.config.description };
        let mut out = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<rss version=\"2.0\" xmlns:atom=\"http://www.w3.org/2005/Atom\"><channel>\
<title>{title}</title><link>{home}</link><description>{description}</description><language>{lang}</language>\
<atom:link href=\"{home}feed.rss\" rel=\"self\" type=\"application/rss+xml\"/><lastBuildDate>{built}</lastBuildDate>\
<generator>UwUStatus</generator>",
            title = esc(&v.page.config.title),
            home = esc(&self.home()),
            description = esc(description),
            lang = texts(v.lang).lang,
            built = rfc2822(v.now),
        );
        for i in self.items() {
            let _ = write!(
                out,
                "<item><title>{title}</title><link>{link}</link><guid isPermaLink=\"false\">{id}</guid>\
<pubDate>{date}</pubDate><description>{text}</description></item>",
                title = esc(&i.title),
                link = esc(&i.link),
                id = esc(&i.id),
                date = rfc2822(i.updated),
                text = esc(&i.text),
            );
        }
        out.push_str("</channel></rss>\n");
        out
    }
}

// ---- Badges ----

/// Flat badge, colours from the Suite's text-safe inks (white text passes 4.5:1 on all of them).
pub fn badge(label: &str, message: &str, color: &str) -> String {
    // shortcut: width from the character count (Verdana 11px ≈ 7px per char); measure glyphs if it looks off.
    let width = |s: &str| s.chars().count() as u32 * 7 + 12;
    let (lw, mw) = (width(label), width(message));
    let total = lw + mw;
    let (label, message) = (esc(label), esc(message));
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{total}\" height=\"20\" role=\"img\" aria-label=\"{label}: {message}\">\
<title>{label}: {message}</title><clipPath id=\"r\"><rect width=\"{total}\" height=\"20\" rx=\"4\"/></clipPath>\
<g clip-path=\"url(#r)\"><rect width=\"{lw}\" height=\"20\" fill=\"#4b3f4b\"/><rect x=\"{lw}\" width=\"{mw}\" height=\"20\" fill=\"{color}\"/></g>\
<g fill=\"#fff\" text-anchor=\"middle\" font-family=\"Verdana,DejaVu Sans,sans-serif\" font-size=\"11\">\
<text x=\"{lx}\" y=\"14\">{label}</text><text x=\"{mx}\" y=\"14\">{message}</text></g></svg>\n",
        lx = lw / 2,
        mx = lw + mw / 2,
    )
}

pub fn state_color(state: MonState) -> &'static str {
    match state {
        MonState::Up => "#17796a",
        MonState::Degraded => "#8e5510",
        MonState::Down => "#b4233a",
        _ => "#716672",
    }
}
