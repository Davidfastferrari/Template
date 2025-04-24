# -------- STAGE 1: BUILD --------
FROM rust:1.86.0 as builder

WORKDIR /app

# Install host build dependencies.
# RUN apk add --no-cache clang lld musl-dev git

# Install libclang for bindgen
#RUN apt-get update && apt-get install -y clang llvm-dev libclang-dev pkg-config cmake build-essential git curl ca-certificates

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

# Copy everything
COPY . .

# Build app
RUN cargo build --release

# -------- STAGE 2: RUNTIME --------
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/BaseBuster .

CMD ["./Template"]
