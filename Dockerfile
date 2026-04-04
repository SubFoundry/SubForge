# syntax=docker/dockerfile:1.7

FROM rust:1.94.1-bookworm AS builder
WORKDIR /work

COPY Cargo.toml Cargo.lock ./
COPY apps ./apps
COPY crates ./crates
COPY migrations ./migrations
COPY plugins ./plugins
COPY subforge.example.toml ./

RUN cargo build -p subforge-core --release

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tini \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd --system subforge \
    && useradd --system --gid subforge --create-home --home-dir /home/subforge --shell /usr/sbin/nologin subforge

WORKDIR /home/subforge

RUN mkdir -p /etc/subforge /var/lib/subforge \
    && chown -R subforge:subforge /etc/subforge /var/lib/subforge

COPY --from=builder /work/target/release/subforge-core /usr/local/bin/subforge-core
COPY subforge.example.toml /etc/subforge/config.toml
COPY plugins /etc/subforge/plugins

RUN chmod 0755 /usr/local/bin/subforge-core \
    && chown -R subforge:subforge /etc/subforge

USER subforge:subforge

EXPOSE 18118

ENTRYPOINT ["/usr/bin/tini", "--", "subforge-core"]
CMD ["run", "--host", "0.0.0.0", "--port", "18118", "--data-dir", "/var/lib/subforge", "--secrets-backend", "env"]
