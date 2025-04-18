# ---------- STAGE 1: Builder ----------
FROM rust:1.76 as builder

# Set working directory
WORKDIR /app

# Copy manifest files early for better Docker cache
COPY Cargo.toml .
COPY Cargo.lock .

# Create dummy main to warm up dependencies cache
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release || true

# Copy actual source files
COPY . .

# Build for release (with debug symbols if needed)
RUN cargo build --release

# ---------- STAGE 2: Runtime ----------
FROM debian:bookworm-slim

# Install required shared libs
RUN apt-get update && apt-get install -y \
    libssl3 \
    ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# Set working dir
WORKDIR /app

# Copy compiled binary from builder stage
COPY --from=builder /app/target/release/BaseBuster ./BaseBuster

# Copy contract folder (for ABI/bytecode access at runtime)
COPY --from=builder /app/contract ./contract

# Enable backtrace logging
ENV RUST_BACKTRACE=1

# Run it
CMD ["./BaseBuster"]
