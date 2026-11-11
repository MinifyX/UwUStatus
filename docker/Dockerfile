# syntax=docker/dockerfile:1.7
#
# Builds UwUStatus from source, the web app included:
#
#   docker build -f docker/Dockerfile -t uwustatus .

# The web app (web/dist is embedded into the binary).
FROM node:24-slim AS web
WORKDIR /web
RUN corepack enable
COPY web/package.json web/pnpm-lock.yaml web/pnpm-workspace.yaml ./
RUN --mount=type=cache,target=/root/.local/share/pnpm/store pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm build

FROM rust:1-slim-trixie AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ crates/
COPY --from=web /web/dist web/dist
# The cached target directory can hold an earlier build whose sources look unchanged to cargo, so
# the crate's own sources are touched first; the registry cache of everything else stays.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target \
    touch crates/uwustatus-server/src/main.rs crates/uwustatus-server/src/lib.rs \
 && cargo build --release --locked \
 && cp target/release/uwustatus /usr/local/bin/uwustatus

# Debian still has the tools distroless lacks: make /data, owned by the user the server runs as.
FROM debian:trixie-slim@sha256:a99cfc517144bc59b1978475ec53b46ecabec7e43635402ee5b77cc54cd1b20a AS prep
COPY --from=build /usr/local/bin/uwustatus /usr/local/bin/uwustatus
RUN chmod 755 /usr/local/bin/uwustatus && install -d -o 10001 -g 10001 /data

# Distroless: glibc and CA certificates, no shell. One port above 1024, one writable directory,
# so no capability and no root.
FROM gcr.io/distroless/cc-debian13@sha256:159783207c2cd44c2aa5715961d13c8612368ac9bd450f887e3f08fc8ea461e3
COPY --from=prep /usr/local/bin/uwustatus /usr/local/bin/uwustatus
COPY --from=prep --chown=10001:10001 /data /data
USER 10001:10001
ENV UWUSTATUS_DATA_DIR=/data \
    UWUSTATUS_BIND=0.0.0.0:8080 \
    HOME=/data
VOLUME /data
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --start-interval=2s --retries=3 \
    CMD ["/usr/local/bin/uwustatus", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/uwustatus"]
CMD ["serve"]
