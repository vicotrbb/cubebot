FROM oven/bun:1.4.0 AS build
WORKDIR /app
COPY package.json bun.lock ./
COPY apps/web/package.json ./apps/web/package.json
RUN bun install --frozen-lockfile
COPY apps/web ./apps/web
RUN bun run build

FROM oven/bun:1.4.0-slim
WORKDIR /app
COPY --from=build --chown=bun:bun /app/apps/web/.output ./.output
USER bun
ENV NODE_ENV=production HOST=0.0.0.0 PORT=3000
EXPOSE 3000
CMD ["bun", ".output/server/index.mjs"]
