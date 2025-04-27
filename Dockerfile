# Stage 1 - Build
FROM rust:1.86 as builder

WORKDIR /usr/src/Template

# Install minimal required system tools
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev build-essential clang cmake git && \
    rm -rf /var/lib/apt/lists/*

# Cache dependencies early
COPY Cargo.toml Cargo.lock .
COPY . .
# COPY src ./src
# COPY benches ./benches
# COPY abi ./abi
# COPY calculation ./calculation
# COPY state_db ./state_db

# Build your project in release mode
RUN cargo fetch
RUN cargo build --release

# Stage 2 - Minimal Runtime
FROM debian:buster-slim

# Install only runtime dependencies
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy compiled binary
COPY --from=builder /usr/src/Template/target/release/Template .

# Define binary entrypoint
ENTRYPOINT ["./Template"]
