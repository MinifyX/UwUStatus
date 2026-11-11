# UwUStatus

Self-hosted uptime monitoring and status pages for the UwU suite. One server watches your
services, alerts you and publishes status pages; probes and a tunnel for checks from outside
follow in later phases (see [docs/plan.md](docs/plan.md)).

> Early scaffold (0.1.0, unreleased): sign-in, settings and the web app shell. No checks yet.

## Run

```sh
cp .env.example .env            # set UWUSTATUS_PUBLIC_URL and OIDC or the local admin
mkdir -p secrets data && chmod 700 secrets
touch secrets/oidc_client_secret secrets/admin_password secrets/admin_totp   # empty = off
chmod 600 secrets/* && sudo chown 10001:10001 data
docker compose up -d
```

`UWUSTATUS_PUBLIC_URL` is the admin interface; every other host name that reaches the server gets
the public status pages. Sign-in is UwUAuth (OIDC, redirect URI
`${UWUSTATUS_PUBLIC_URL}/api/auth/callback`) and/or a local admin as a fallback. The local
admin's password file holds the password or an Argon2 hash; `uwustatus totp-secret` prints a
TOTP secret for it.

## Develop

```sh
cargo test --workspace                  # server, in-process tests with a temp DB each
cd web && pnpm install && pnpm dev      # web app on :5173, API proxied to 127.0.0.1:8080
```

For `pnpm dev`, run the server with `UWUSTATUS_PUBLIC_URL=http://localhost:5173`, so the
proxied requests count as the admin host. `pnpm build` writes `web/dist`, which the release
binary embeds.

## License

[AGPL-3.0](LICENSE)
