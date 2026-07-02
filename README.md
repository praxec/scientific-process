# scientific-process

An append-only **scientific-process** MCP server — hypotheses, experiments, observations, conclusions, and verdicts for evidence-driven work. Part of [Praxec](https://github.com/praxec/praxec); spawned as a `kind: mcp` connection (see the [pack registry](https://github.com/praxec/packs)).

## Build
`cargo build --release` produces the `scientific-process` MCP stdio server.

## Using it with Praxec

This is an MCP tool used by [Praxec](https://github.com/praxec/praxec) packs. The easiest way to
get it — and a workflow pack that uses it — up and running is the one-command setup:

```bash
curl -fsSL https://raw.githubusercontent.com/praxec/packs/main/setup.sh | bash
```

See the [pack registry](https://github.com/praxec/packs) for this tool's provider coordinates
(container image / release binary) and which packs depend on it.

## License
[Apache-2.0](LICENSE).
