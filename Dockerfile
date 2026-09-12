FROM rust:1.98.0-slim-bookworm AS builder

WORKDIR /build

# ring's build script compiles assembly/C sources; needs a C compiler.
RUN apt-get update && apt-get install -y --no-install-recommends gcc && rm -rf /var/lib/apt/lists/*

# Build deps against a throwaway main.rs first, so this layer only
# invalidates when Cargo.toml/Cargo.lock change, not on every source edit.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release && rm -rf src

COPY src ./src
RUN touch src/main.rs && cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends jq && rm -rf /var/lib/apt/lists/*

RUN useradd --system --create-home --uid 10001 --user-group ien

COPY --from=builder /build/target/release/ien-telegram-bot /usr/local/bin/ien-telegram-bot
COPY prompts /home/ien/prompts

USER ien
WORKDIR /home/ien
ENTRYPOINT ["ien-telegram-bot"]
