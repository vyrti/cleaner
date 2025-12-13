# Build stage - multi-arch support
FROM --platform=$BUILDPLATFORM rust:alpine AS builder

ARG TARGETPLATFORM
ARG BUILDPLATFORM

# Install build dependencies
RUN apk add --no-cache musl-dev

WORKDIR /app

# Copy source
COPY Cargo.toml ./
COPY src ./src

# Build for the target platform
RUN case "$TARGETPLATFORM" in \
      "linux/amd64") TARGET="x86_64-unknown-linux-musl" ;; \
      "linux/arm64") TARGET="aarch64-unknown-linux-musl" ;; \
      *) TARGET="x86_64-unknown-linux-musl" ;; \
    esac && \
    rustup target add $TARGET 2>/dev/null || true && \
    cargo build --release --target $TARGET && \
    cp target/$TARGET/release/cleaner /app/cleaner || \
    (cargo build --release && cp target/release/cleaner /app/cleaner)

# Runtime stage
FROM alpine:latest

# Install ca-certificates for HTTPS (if needed in future)
RUN apk add --no-cache ca-certificates

# Copy binary from builder
COPY --from=builder /app/cleaner /usr/local/bin/cleaner

# Create non-root user
RUN adduser -D -u 1000 cleaner
USER cleaner

# Default to showing help
ENTRYPOINT ["cleaner"]
CMD ["--help"]
