# SPDX-License-Identifier: BUSL-1.1
# Copyright (c) 2026 Alfred Jean LLC

FROM rust:1.92-bookworm AS builder
ARG TARGETARCH
ARG BUILD_GIT_HASH
RUN apt-get update && apt-get install -y musl-tools \
    && rustup target add x86_64-unknown-linux-musl \
    && rustup target add aarch64-unknown-linux-musl
WORKDIR /src
COPY . .
RUN case "$TARGETARCH" in \
      arm64) RUST_TARGET=aarch64-unknown-linux-musl ;; \
      *)     RUST_TARGET=x86_64-unknown-linux-musl ;; \
    esac \
    && BUILD_GIT_HASH="${BUILD_GIT_HASH}" \
       cargo build --release --target "$RUST_TARGET" -p oj-daemon \
    && strip "target/$RUST_TARGET/release/ojd" \
    && cp "target/$RUST_TARGET/release/ojd" /ojd-bin

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    git bash openssh-client ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /ojd-bin /usr/local/bin/ojd
ENV OJ_STATE_DIR=/var/lib/oj
ENV OJ_TCP_PORT=7777
EXPOSE 7777
ENTRYPOINT ["ojd"]
