# Stage 1: Frontend build
FROM node:22-slim AS frontend
WORKDIR /app/client
COPY client/package.json client/package-lock.json* ./
RUN npm ci || npm install
COPY client/ .
RUN npx vite build

# Stage 2: Rust build
FROM rust:1-slim AS backend
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs
RUN cargo build --release
RUN rm -rf src
COPY src/ src/
RUN touch src/main.rs && cargo build --release

# Stage 3: Production runtime
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=backend /app/target/release/youtube-sub-feed /usr/local/bin/
COPY --from=frontend /app/client/build /app/client/build
WORKDIR /app
ENV PORT=3000
EXPOSE 3000
CMD ["youtube-sub-feed"]
