# syntax=docker/dockerfile:1

FROM rust:1-bookworm AS builder
WORKDIR /app

# 先只复制清单，以便 Docker 缓存依赖编译层。
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src/bin \
    && printf 'fn main() {}\n' > src/bin/exloli.rs \
    && printf 'pub fn placeholder() {}\n' > src/lib.rs \
    && cargo build --release --bin exloli \
    && rm -rf src

COPY . .
RUN cargo build --release --bin exloli

FROM debian:bookworm-slim
WORKDIR /app
ENV RUST_BACKTRACE=1

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/exloli /usr/local/bin/exloli

ENTRYPOINT ["exloli"]
