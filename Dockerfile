# Stage 1: Build
FROM oven/bun:1 AS builder
WORKDIR /app

# Server dependencies
COPY package.json bun.lock ./
RUN bun install --frozen-lockfile

# Client build
COPY client/package.json client/bun.lock ./client/
RUN cd client && bun install --frozen-lockfile
COPY client/ ./client/
RUN cd client && bun run build

# Assemble /dist
RUN mkdir -p /dist/client && \
    cp package.json bun.lock /dist/ && \
    cp -r client/build /dist/client/
COPY src/ /dist/src/
RUN cd /dist && bun install --frozen-lockfile --production

# Stage 2: Production runtime
FROM oven/bun:1-slim
WORKDIR /app
COPY --from=builder /dist ./
ENV NODE_ENV=production
ENV PORT=3000
EXPOSE 3000
CMD ["bun", "run", "src/index.ts"]
