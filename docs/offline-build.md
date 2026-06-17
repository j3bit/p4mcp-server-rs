# Offline Build Workflow

Prepare dependencies on a networked machine first:

```bash
cargo fetch --locked
cargo vendor vendor
mkdir -p .cargo
cat > .cargo/config.toml <<'EOF'
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
EOF
```

Move the repository, `vendor/`, and `.cargo/config.toml` into the closed network.

Build and package inside the closed network:

```bash
cargo build --release --locked --offline
./scripts/package.sh
```

The runtime machine must already have a working `p4` CLI and Perforce authentication context:

```bash
p4 info
./p4-mcp-server --readonly
```
