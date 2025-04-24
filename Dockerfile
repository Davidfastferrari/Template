# -------- STAGE 1: BUILD --------
FROM rust:1.86.0 as builder

# 👇 Match the inner Template folder
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

# 👇 COPY the actual crate folder (the second Template)
COPY Template/ .      # << Important

RUN cargo build --release

# -------- STAGE 2: RUNTIME --------
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    libssl3 \
    ca-certificates \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/Template .
CMD ["./Template"]
