# -------- STAGE 1: BUILD --------
FROM rust:1.86.0 as builder

WORKDIR /app

# Install host build dependencies.
# RUN apk add --no-cache clang lld musl-dev git

# Required libs for bindgen + FFI
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

# Optional: Explicitly tell bindgen where to find libclang (sometimes needed)
ENV LIBCLANG_PATH=/usr/lib/llvm-14/lib
ENV CLANG_PATH=/usr/bin/clang

# Copy full project
COPY . ./

# Build without relying on Cargo.lock
RUN cargo build --release

# -------- STAGE 2: RUNTIME --------
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    libssl3 \
    ca-certificates \
 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/src ./src

ENV RUST_BACKTRACE=1
ENV RUST_LOG=info

CMD ["./Template"]
