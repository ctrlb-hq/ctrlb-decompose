# syntax=docker/dockerfile:1

# --- Build stage -------------------------------------------------------
FROM rust:1-slim-bookworm AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY benches ./benches

RUN cargo build --release --locked --bin ctrlb-decompose

# --- Runtime stage -------------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN useradd --no-create-home --uid 10001 decompose
COPY --from=builder /app/target/release/ctrlb-decompose /usr/local/bin/ctrlb-decompose

USER decompose
WORKDIR /logs
ENTRYPOINT ["ctrlb-decompose"]
CMD ["--help"]
