# ---------- STAGE 1: Build ----------
FROM rust:1.77 as builder

WORKDIR /app

# Copy manifest files first for caching
COPY Cargo.toml .
COPY Cargo.lock .

# Create dummy src/main.rs to warm cache
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release || true

# Now copy the full source
COPY . .

# Build the actual release binary
RUN cargo build --release

# ---------- STAGE 2: Runtime ----------
FROM debian:bookworm-slim

# Install required system libs (especially OpenSSL)
RUN apt-get update && apt-get install -y \
    libssl3 \
    ca-certificates \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary and any runtime dependencies (like contract files)
COPY --from=builder /app/target/release/BaseBuster ./BaseBuster
COPY --from=builder /app/contract ./contract

ENV RUST_BACKTRACE=1

CMD ["./BaseBuster"]
