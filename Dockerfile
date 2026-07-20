# syntax=docker/dockerfile:1
#
# Multi-stage build for the `smelt` CLI, linked against the system DuckDB
# shared library (avoids compiling DuckDB from C++ source; mirrors the
# DUCKDB_LIB_DIR setup documented in CLAUDE.md). Pinned to the DuckDB version
# in Cargo.toml (v1.5.4).

ARG DUCKDB_VERSION=1.5.4

FROM node:20-bookworm-slim AS ui-builder
WORKDIR /build/ui
COPY ui/package.json ui/package-lock.json ./
RUN npm ci
COPY ui/ ./
RUN npm run build

FROM rust:bookworm AS builder
ARG DUCKDB_VERSION

RUN apt-get update && apt-get install -y --no-install-recommends \
    unzip \
    && rm -rf /var/lib/apt/lists/*

RUN curl -sL "https://github.com/duckdb/duckdb/releases/download/v${DUCKDB_VERSION}/libduckdb-linux-amd64.zip" \
    -o /tmp/libduckdb.zip \
    && unzip -o /tmp/libduckdb.zip libduckdb.so duckdb.h -d /usr/local/lib \
    && rm /tmp/libduckdb.zip \
    && ldconfig

ENV DUCKDB_LIB_DIR=/usr/local/lib
ENV LD_LIBRARY_PATH=/usr/local/lib

WORKDIR /build
COPY . .
COPY --from=ui-builder /build/ui/dist ./ui/dist

RUN cargo build --release -p smelt-cli

FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    python3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/lib/libduckdb.so /usr/local/lib/libduckdb.so
RUN ldconfig
ENV LD_LIBRARY_PATH=/usr/local/lib

COPY --from=builder /build/target/release/smelt /usr/local/bin/smelt
COPY --from=builder /build/python/smelt /usr/local/share/smelt-python-sdk/smelt
ENV SMELT_PYTHON_SDK=/usr/local/share/smelt-python-sdk

ENTRYPOINT ["smelt"]
CMD ["--help"]
