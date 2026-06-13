# p4mcp-server-rs

Rust port of Perforce P4 MCP Server.

This binary talks to the local `p4` CLI. It does not embed Python, P4Python, or a Perforce server.

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
