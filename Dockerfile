# syntax=docker/dockerfile:1
#
# Unlike the backend, the website's own Cargo.toml points its htmldiff
# dependency at `./htmldiff` (inside this same repo), so this image is
# self-contained: build context = the website repo itself.

########################
# 1) Build stage
########################
FROM rust:1.98-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    perl \
    make \
    clang \
    cmake \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .

# Keep target/ outside of /app so the next stage can copy the whole
# source tree back out cleanly (see note below) without dragging build
# artifacts along with it.
ENV CARGO_TARGET_DIR=/build
RUN cargo build --release

########################
# 2) Runtime stage
########################
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 app

WORKDIR /app

# The website uses Tera (which loads *.html templates from disk via a glob
# at startup) and tower-http's `fs` feature (which usually serves a
# static/public folder from disk too).
COPY --from=builder /app ./
COPY --from=builder /build/release/website ./website-bin
RUN chown -R app:app /app

USER app
EXPOSE 1337
ENV RUST_LOG=info

CMD ["./website-bin"]
