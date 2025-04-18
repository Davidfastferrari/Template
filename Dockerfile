# -------- STAGE 1: BUILD --------
FROM rust:1.77 as builder

# Set working directory inside container
WORKDIR /app

# Copy manifest files first (for build caching)
COPY Cargo.toml .
COPY Cargo.lock .

# Warm up build cache with dummy main.rs
RUN mkdir -p src && echo "fn main() {}" > src/main.rs
RUN cargo build --release || true

# Copy your entire project source (excluding unneeded files)
COPY . .

# Final optimized build
RUN cargo build --release

# -------- STAGE 2: RUN --------
FROM debian:bookworm-slim

# Install runtime dependencies for compiled binary
RUN apt-get update && apt-get install -y \
    libssl3 \
    ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# Create app directory
WORKDIR /app

# Copy the built binary and any required runtime assets
COPY --from=builder /app/target/release/BaseBuster ./BaseBuster
COPY --from=builder /app/contract ./contract

# Enable better error messages and logging
ENV RUST_BACKTRACE=1
ENV RUST_LOG=info

# Start your app
CMD ["./BaseBuster"]
