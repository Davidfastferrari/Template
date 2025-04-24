# ---------------------
# 🚧 STAGE 1: BUILD
# ---------------------
FROM rust:1.76 as builder

WORKDIR /app

# Install build deps for FFI/BPF/smart contract tooling
RUN apt-get update && apt-get install -y \
    clang \
    llvm-dev \
    libclang-dev \
    pkg-config \
    build-essential \
    cmake \
    curl \
    git \
    ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# Cache dependencies first for incremental builds
COPY ./Cargo.toml ./Cargo.lock
RUN mkdir -p src && echo 'fn main() {}' > src/main.rs
WORKDIR /app
RUN cargo build --release || true

# Actual code copy
WORKDIR /app
COPY . .

# Full optimized build
WORKDIR /app
RUN cargo build --release

# ---------------------
# 🚀 STAGE 2: RUNTIME
# ---------------------
FROM debian:bookworm-slim

# Install runtime dependencies (OpenSSL, Certs)
RUN apt-get update && apt-get install -y \
    libssl3 \
    ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# Set working directory
WORKDIR /app

# Copy only release binary
COPY --from=builder /app/Template/target/release/Template .

# Inject .env (optional)
COPY Template/.env .env

# Log output + backtrace in production
ENV RUST_BACKTRACE=1
ENV RUST_LOG=info

# Entrypoint
CMD ["./Template"]
