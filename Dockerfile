FROM rust:1.98.0-slim-bookworm AS builder

WORKDIR /build

# ring's build script compiles assembly/C sources; needs a C compiler.
RUN apt-get update && apt-get install -y --no-install-recommends gcc && rm -rf /var/lib/apt/lists/*

# Build deps against a throwaway main.rs first, so this layer only
# invalidates when Cargo.toml/Cargo.lock change, not on every source edit.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release && rm -rf src

COPY src ./src
# prompts/routing.md is pulled in via include_str! at compile time, so it must be
# present in the build context here, not just copied into the final image below.
COPY prompts/routing.md ./prompts/routing.md
RUN touch src/main.rs && cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends jq && rm -rf /var/lib/apt/lists/*

RUN useradd --system --create-home --uid 10001 --user-group ien

COPY --from=builder /build/target/release/ien-telegram-bot /usr/local/bin/ien-telegram-bot
# routing.md's content is compiled into the binary (see ROUTING_SYSTEM_PROMPT_BASE in
# src/main.rs); only reply.md is still read at runtime, so only it ships here. An
# optional routing overlay is read from ROUTING_PROMPT_PATH if the operator mounts one.
COPY prompts/reply.md /home/ien/prompts/reply.md

USER ien
WORKDIR /home/ien
ENTRYPOINT ["ien-telegram-bot"]
