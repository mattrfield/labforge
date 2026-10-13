FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim
# Unprivileged user; /data/keys is a volume that inherits this ownership on first use.
RUN useradd --system --uid 10001 --home-dir /nonexistent --shell /usr/sbin/nologin app \
 && mkdir -p /data/keys /data/packs \
 && chown -R app:app /data
COPY --from=build /src/target/release/labforge-license-server /usr/local/bin/labforge-license-server
USER app
EXPOSE 8787
ENTRYPOINT ["labforge-license-server"]
CMD ["serve", "--bind", "0.0.0.0", "--keys", "/data/keys", "--packs", "/data/packs", "--trust-proxy"]
