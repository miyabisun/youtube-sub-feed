# syntax=docker/dockerfile:1

FROM node:22-slim AS frontend
WORKDIR /app/client
COPY client/package.json client/package-lock.json ./
RUN npm ci
COPY client/ .
RUN npx vite build

# Keep dependency and application builds on the same toolchain and Debian release.
FROM rust:1.96.0-bookworm AS chef
RUN apt-get update && apt-get install -y pkg-config libssl-dev curl && rm -rf /var/lib/apt/lists/*
WORKDIR /app
RUN cargo install cargo-chef --version 0.1.78 --locked

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS backend
# Recipes normalize the local package version so tags reuse dependency builds.
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --locked --release --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN cargo build --locked --release

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=backend /app/target/release/youtube-sub-feed /usr/local/bin/
COPY --from=frontend /app/client/build /app/client/build
WORKDIR /app
ENV PORT=3000
EXPOSE 3000
CMD ["youtube-sub-feed"]
