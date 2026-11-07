FROM oven/bun:1.4.0 AS build
WORKDIR /app
COPY package.json bun.lock ./
COPY apps/web/package.json ./apps/web/package.json
RUN bun install --frozen-lockfile
COPY modules/brand ./modules/brand
COPY apps/web ./apps/web
RUN bun run build

FROM oven/bun:1.4.0-slim
WORKDIR /app
COPY --from=build --chown=bun:bun /app/apps/web/.output ./.output
USER bun
ENV NODE_ENV=production HOST=0.0.0.0 PORT=3000
EXPOSE 3000
CMD ["bun", ".output/server/index.mjs"]
# Last, so they never invalidate a cached layer.
ARG CUBEBOT_VERSION=dev
ARG CUBEBOT_REVISION=unknown
ARG CUBEBOT_CREATED=unknown
LABEL org.opencontainers.image.title="Cubebot web console" \
      org.opencontainers.image.description="The Cubebot console and installable PWA, where people work with their agents." \
      org.opencontainers.image.url="https://cubebot.io" \
      org.opencontainers.image.source="https://github.com/vicotrbb/cubebot" \
      org.opencontainers.image.documentation="https://github.com/vicotrbb/cubebot/blob/main/README.md" \
      org.opencontainers.image.vendor="Cubebot" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.base.name="docker.io/oven/bun:1.4.0-slim" \
      org.opencontainers.image.version="$CUBEBOT_VERSION" \
      org.opencontainers.image.revision="$CUBEBOT_REVISION" \
      org.opencontainers.image.created="$CUBEBOT_CREATED" \
      dev.orbstack.icon="https://raw.githubusercontent.com/vicotrbb/cubebot/main/apps/web/public/icons/container-web.png"
