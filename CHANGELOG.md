# Changelog

## 0.1.0 (unreleased)

- Scaffold: server (`uwustatus`) with SQLite migrations, settings, `/healthz` in the suite format.
- Sign-in via UwUAuth (OIDC) and a local admin (Argon2 password, optional TOTP) as a fallback.
- Host routing: the admin host serves the admin app, every other host the public status pages.
- Web app scaffold (German/English), container image for amd64 and arm64.
