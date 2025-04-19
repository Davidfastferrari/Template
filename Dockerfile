# -------- STAGE 1: BUILD --------
FROM rust:1.76 as builder

# Set the working directory
WORKDIR /app

# Copy the entire workspace into container
COPY . .

# Build the workspace binary crate
# You MUST specify the binary crate name, since root Cargo.toml is a workspace
RUN cargo build -p BaseBuster --release

# -------- STAGE 2: RUNTIME --------
FROM debian:bookworm-slim

# Install runtime dependencies (OpenSSL, CA certs, etc.)
RUN apt-get update && apt-get install -y \
    libssl3 \
    ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# Copy the compiled binary
WORKDIR /app
COPY --from=builder /app/target/release/BaseBuster ./BaseBuster

# Copy any runtime files needed by the binary
COPY --from=builder /app/contract ./contract

# Runtime environment variables
ENV RUST_BACKTRACE=1
ENV RUST_LOG=info

# Launch the binary
CMD ["./BaseBuster"]
