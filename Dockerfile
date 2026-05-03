# Build stage
FROM rust:1.85-slim AS builder
WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY vendor ./vendor
RUN cargo build --release --locked

# Runtime stage
FROM debian:stable-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates wget && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/hashcards /usr/local/bin/hashcards
WORKDIR /cards
EXPOSE 8000
ENTRYPOINT ["hashcards"]
CMD ["serve", "/cards"]
