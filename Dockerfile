# -------- STAGE 1: BUILD --------
FROM rust:latest AS builder

WORKDIR /app

# Copy everything — make sure Cargo.toml from root is included!
COPY . .

# DEBUG (optional sanity check)
RUN ls -l /app && ls -l /app/src && cat /app/Cargo.toml

# Build the binary crate inside the workspace
RUN cargo build -p BaseBuster --release

# Runtime image
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    libssl3 \
    ca-certificates \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/BaseBuster ./BaseBuster
COPY --from=builder /app/contract ./contract

ENV RUST_BACKTRACE=1
ENV RUST_LOG=info

CMD ["./BaseBuster"]
