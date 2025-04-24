# -------- STAGE 1: BUILD --------
FROM rust:1.86.0 as builder

WORKDIR /app

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

# 👇 Match the inner Template folder
# Copy the actual crate code into /app/Template
# COPY Template/ ./Template/

COPY ./Template

# Move into the actual Rust project directory
WORKDIR /app/Template

RUN cargo build --release

# -------- STAGE 2: RUNTIME --------
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    libssl3 \
    ca-certificates \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the final built binary
COPY --from=builder /app/Template/target/release/Template .

CMD ["./Template"]
