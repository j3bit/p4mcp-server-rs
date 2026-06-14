# p4mcp-server-rs

Rust port of Perforce P4 MCP Server.

This binary talks to the local `p4` CLI. It does not embed Python, P4Python, or a Perforce server.

## Upstream Baseline

This port is compared against `perforce/p4mcp-server` `v2026.2.2955897` at commit
`a64efb07511b2a62db41aeed110ab96744c4076a`, checked on 2026-06-14.
The functional tool surface should follow that upstream baseline unless this README
or a file under `docs/` explicitly documents a Rust-port extension.

Documented Rust-port safety extension: when write tools are enabled, every
`modify_*` tool must pass the server-side write approval gate before any `p4`
command or review write request is executed. The preferred path is MCP
elicitation from the client UI; clients without elicitation support use a
same-request one-time fallback token flow. This is intentionally stricter than
the upstream Python server's destructive-operation prompts, and it replaces any
model-supplied confirmation field.

## Development

```bash
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
```

## Runtime

```bash
P4PORT=ssl:perforce.example.com:1666 \
P4USER=your_username \
P4CLIENT=your_workspace \
cargo run -- --readonly
```

## Release Packaging

```bash
./scripts/package.sh
```

The resulting archive contains the Rust binary and docs. It does not contain Python or P4Python.

## MCP Client Example

```json
{
  "mcpServers": {
    "perforce-p4-mcp": {
      "command": "/absolute/path/to/p4-mcp-server",
      "args": ["--readonly"],
      "env": {
        "P4PORT": "ssl:perforce.example.com:1666",
        "P4USER": "your_username",
        "P4CLIENT": "your_workspace"
      }
    }
  }
}
```
