# -------- STAGE 1: BUILD --------
FROM rust: 1.85.0 as builder

WORKDIR /app
COPY . .

# Debug info
RUN ls -l /app && ls -l /app/src && cat /app/Cargo.toml

# Build the binary from the root crate
RUN cargo build --release

# -------- STAGE 2: RUNTIME --------
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
