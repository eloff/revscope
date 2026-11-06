# syntax=docker/dockerfile:1

# A self-hosted RevScope: the backend with the tracker and web app compiled in.
# See "Run your own instance with Docker" in README.md; compose.yaml builds this.

# --- Web app ------------------------------------------------------
FROM node:24.22.2-bookworm-slim@sha256:c86ecead9e55a5210ac1727de38a4ca840bf03dcaf34dc9749addbf383b1920c AS web
RUN npm install -g pnpm@8.12.1
WORKDIR /src

# Dependencies first, so editing sources doesn't reinstall them.
COPY web/package.json web/pnpm-lock.yaml web/pnpm-workspace.yaml web/
COPY web/app/package.json web/app/
COPY web/shared/package.json web/shared/
RUN cd web && pnpm install --frozen-lockfile

COPY web web
RUN cd web && pnpm build:bundle

# --- Backend ------------------------------------------------------------------
FROM rust:1.99.1-bookworm@sha256:9181b56c2a21ab1d5e73cffe04fc762426d30aa7fac3c91141dd033f49a7fc8b AS backend
WORKDIR /src
COPY crates crates
COPY revscope revscope
COPY scripts/download-maxmind.sh scripts/
COPY tracker/tracker-min.js tracker/tracker-min.js.gz tracker/tracker-min.js.br tracker/
COPY --from=web /src/web/app/dist-ui web/app/dist-ui

# Geolocation is embedded when revscope/GeoLite2-City.mmdb is in the build
# context, or downloaded here when MaxMind credentials are passed as secrets.
RUN --mount=type=secret,id=maxmind_account_id,env=MAXMIND_ACCOUNT_ID \
    --mount=type=secret,id=maxmind_license_key,env=MAXMIND_LICENSE_KEY \
    if [ ! -f revscope/GeoLite2-City.mmdb ] && [ -n "${MAXMIND_LICENSE_KEY:-}" ]; then \
        scripts/download-maxmind.sh; \
    fi

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/revscope/target \
    cd revscope \
    && cargo build --release --locked --bin revscope --features self-hosted \
    && cp target/release/revscope /usr/local/bin/revscope

# --- Runtime ------------------------------------------------------------------
FROM debian:bookworm-20261027-slim@sha256:99c0b25c46d0214d5a2091092e7908907dcaecd6a962cde0ac700acee222b6f3
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
RUN useradd --system --uid 10001 --home-dir /app revscope \
    && mkdir -p /app/data \
    && chown revscope:revscope /app/data
COPY --from=backend /usr/local/bin/revscope /usr/local/bin/revscope

# Project databases and ACME certificates go in data/ under the working directory.
WORKDIR /app
VOLUME /app/data
USER revscope
ENV REVSCOPE_LISTEN=0.0.0.0:4320 \
    RUST_LOG=info
EXPOSE 4320
CMD ["revscope"]
