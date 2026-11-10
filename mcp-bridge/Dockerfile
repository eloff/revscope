# syntax=docker/dockerfile:1

# A stdio MCP server that relays to RevScope's hosted MCP endpoint. See README.md.

FROM node:24.22.2-bookworm-slim@sha256:c86ecead9e55a5210ac1727de38a4ca840bf03dcaf34dc9749addbf383b1920c AS deps
RUN npm install -g pnpm@8.12.1
WORKDIR /bridge
COPY package.json pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile --prod

FROM node:24.22.2-bookworm-slim@sha256:c86ecead9e55a5210ac1727de38a4ca840bf03dcaf34dc9749addbf383b1920c
# mcp-remote handles only SIGINT, and the kernel drops signals PID 1 has no handler for, so without
# an init the SIGTERM an MCP client stops the bridge with would be ignored.
RUN apt-get update \
    && apt-get install -y --no-install-recommends tini \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /bridge
COPY --from=deps /bridge/node_modules node_modules
COPY entrypoint.sh forward.mjs preflight.mjs ./
# A named volume mounted here takes this directory's owner, so the node user can write its tokens.
RUN mkdir /data && chown node:node /data
USER node
ENV MCP_REMOTE_CONFIG_DIR=/data
VOLUME /data
ENTRYPOINT ["/usr/bin/tini", "--", "/bridge/entrypoint.sh"]
