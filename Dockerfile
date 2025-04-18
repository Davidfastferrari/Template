# ------------ STAGE 1: BUILD ------------
FROM rust:1.77 as builder

WORKDIR /app

# Copy manifests first to cache dependencies
COPY Cargo.toml .
COPY Cargo.lock .

# Dummy main for caching
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release || true

# Now copy the full project
COPY . .

# Build full release
RUN cargo build --release

# ------------ STAGE 2: RUNTIME ------------
FROM debian:bookworm-slim

# Required system libraries (e.g., OpenSSL)
RUN apt-get update && apt-get install -y \
    libssl3 \
    ca-certificates \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the compiled binary and contract files
COPY --from=builder /app/target/release/BaseBuster ./BaseBuster
COPY --from=builder /app/contract ./contract

# Set runtime environment vars
ENV RUST_BACKTRACE=1

# Run the compiled binary
CMD ["./BaseBuster"]
