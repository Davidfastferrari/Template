# -------- STAGE 1: BUILD --------
FROM rust:1.86.0 as builder

WORKDIR /home
RUN USER=root cargo new --bin rust-docker-web
WORKDIR /home

# Cache dependencies
COPY ./Cargo.toml ./Cargo.toml
RUN cargo build --release

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
# Full build
COPY . ./
RUN cargo build --release

# -------- STAGE 2: RUNTIME --------
FROM debian:bookworm-slim

ARG APP=/usr/src/app

EXPOSE 6767

ENV TZ=Etc/UTC \
    APP_USER=appuser

RUN addgroup -S $APP_USER \
    && adduser -S -g $APP_USER $APP_USER

RUN apk update \
    && apk add --no-cache ca-certificates tzdata \
    && rm -rf /var/cache/apk/*

COPY --from=builder /home/target/release/rust-docker-web ${APP}/rust-docker-web
COPY ./static ${APP}/static
COPY --from=builder /app/target/release/Template ./Template
COPY --from=builder /app/contract ./contract
COPY --from=builder /app/src ./src

RUN chown -R $APP_USER:$APP_USER ${APP}
USER $APP_USER
WORKDIR ${APP}

RUN apt-get update && apt-get install -y \
    libssl3 \
    ca-certificates \
 && rm -rf /var/lib/apt/lists/*


CMD ["./BaseBuster"]
