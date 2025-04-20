# -------- STAGE 1: BUILD --------
FROM rust:1.85.0 as builder  # 🔧 Use the most stable & tested version (1.85 not released yet)

WORKDIR /app

# Preload only metadata to cache dependencies
#COPY Cargo.toml Cargo.lock ./

# Avoid full rebuild when only src changes
RUN mkdir -p src && echo "fn main() {}" > src/main.rs

# Fetch dependencies (WITHOUT resolving to newer versions)
RUN cargo fetch --locked

# Copy the full source code
COPY . .

# Show file structure and loaded manifest
RUN ls -la /app && cat Cargo.toml

# Build with locking to prevent unwanted updates
RUN cargo build --release --locked

# -------- STAGE 2: RUNTIME --------
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    libssl3 \
    ca-certificates \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy only the built binary and runtime essentials
COPY --from=builder /app/target/release/BaseBuster ./BaseBuster
COPY --from=builder /app/contract ./contract

# Set environment variables for runtime behavior
ENV RUST_BACKTRACE=1
ENV RUST_LOG=info

CMD ["./BaseBuster"]
