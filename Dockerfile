FROM rust:1.98.0-slim-bookworm AS chef

WORKDIR /build

# ring's build script compiles assembly/C sources; needs a C compiler.
RUN apt-get update && apt-get install -y --no-install-recommends gcc && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef --locked

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
# Builds only the dependency graph from the recipe, so this layer only
# invalidates when Cargo.toml/Cargo.lock change, not on every source edit.
COPY --from=planner /build/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim

RUN useradd --system --create-home --uid 10001 --user-group ien

COPY --from=builder /build/target/release/ien-telegram-bot /usr/local/bin/ien-telegram-bot

USER ien
ENTRYPOINT ["ien-telegram-bot"]
