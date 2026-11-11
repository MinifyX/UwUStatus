//! One integration binary: handlers run in-process, OIDC against a local fake, no sleeps.

mod alerts;
mod auth;
mod basics;
mod fakes;
mod history;
mod ingest;
mod monitors;
mod planner;
mod status_pages;
mod support;
