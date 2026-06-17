# Stream Resolve Scoping

Status: deferred from PR #2 to keep the Rust port aligned with upstream
`perforce/p4mcp-server`.

## Reason

The Rust port currently mirrors upstream stream spec resolve checks:

- Upstream `check_stream_spec_resolve_needed(stream_name)` verifies that the
  named stream exists, then runs `p4 stream resolve -n` without a stream
  argument.
- Upstream comments state that `p4 stream resolve -n` operates on the
  workspace's stream and does not take a stream name argument.
- Upstream still labels the response with the requested `stream_name`.
- The Rust port follows the same shape: it validates the requested stream, runs
  `p4 stream resolve -n`, and reports the records under the requested stream.

The PR review feedback is valid as a context-scoping issue, but it is not a
Rust-port parity bug. For non-current streams, the command can report pending
stream-spec conflicts for the active workspace stream while the MCP response is
labeled with the requested stream.

## Future Design

- Prefer rejecting `query_streams.check_resolve` for a non-current stream unless
  the request also supplies a workspace bound to that stream.
- If workspace-scoped checks are added, run `p4 stream resolve -n` with
  `P4CLIENT=<workspace>` or an equivalent temporary client context.
- Consider applying the same scoping policy to other stream-spec operations that
  depend on the active workspace stream, including private spec edit/resolve
  workflows.
- Keep the current no-argument `p4 stream resolve -n` invocation for the active
  workspace stream.

## Reintroduction Checklist

- Add tests for current-stream checks, non-current-stream rejection, and
  workspace-bound checks if a workspace parameter is added.
- Prove the response cannot label active-workspace records as a different
  requested stream.
- Document this as an intentional upstream divergence in the commit or PR notes.
