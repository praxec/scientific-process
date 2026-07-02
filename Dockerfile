# scientific-process — MCP server (stdio). Multi-stage build → slim runtime.
FROM rust:1-slim AS build
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo build --release --bin scientific-process && cp target/release/scientific-process /scientific-process

FROM debian:stable-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* \
    && useradd -m -u 10001 app
COPY --from=build /scientific-process /usr/local/bin/scientific-process
USER app
# The gateway spawns this container with `docker run -i` and speaks MCP over stdio.
ENTRYPOINT ["scientific-process"]
