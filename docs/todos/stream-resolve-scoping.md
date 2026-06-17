# Stream Resolve Scoping

Status: deferred follow-up from PR #2. The review finding is a real
context-scoping risk, but the current Rust behavior matches upstream
`perforce/p4mcp-server` at `a64efb07511b2a62db41aeed110ab96744c4076a`
(`v2026.2.2955897`). Rechecked against upstream `origin/main` on 2026-06-18.

## Reviewer Claim

`query_streams.check_resolve` accepts a `stream_name`, but the generated P4
command is `p4 stream resolve -n` with no stream argument. Perforce evaluates
that command against the active workspace stream, so a request for a non-current
stream can return active-workspace resolve records while labeling them as the
requested stream.

## Current Rust Port

- The handler verifies that the requested stream exists.
- It then runs `p4 stream resolve -n`.
- The response includes the requested `stream_name` and the returned records.
- It does not verify that the active workspace is bound to the requested stream.

## Upstream Behavior

Upstream performs the same sequence:

- Verify the requested stream exists with `p4 streams -F Stream=<stream>`, then
  with `-a` if needed.
- Run `p4 stream resolve -n` without a stream argument.
- Comment in source that `p4 stream resolve -n` operates on the workspace's
  stream and does not take a stream name argument.
- Return a response labeled with the requested `stream_name`.

## Actual Threat

This is not a direct destructive write: the command is a preview/read operation.
The risk is decision corruption in agent workflows:

- A false positive can make an agent believe the requested stream has pending
  spec conflicts when only the active workspace stream does.
- A false negative can make an agent believe the requested stream is clear when
  the active workspace was clear but the requested stream would need checking
  from a different workspace context.
- Follow-up edit/resolve/shelve stream-spec actions may be planned against the
  wrong stream because the response label is trusted.

Severity: low to medium for direct safety, medium for automated workflow
correctness when agents chain stream operations from this query result.

## Follow-Up Design

- Treat the current behavior as upstream parity, not as a Rust-port bug.
- For a stricter Rust extension, reject non-current `stream_name` unless the
  request supplies a workspace bound to that stream.
- If a workspace is supplied, run the preview with `P4CLIENT=<workspace>` or an
  equivalent temporary client context.
- Keep the no-argument `p4 stream resolve -n` form; do not invent a stream
  argument that Perforce does not support.

## Verification Needed

- Test current-stream success.
- Test non-current stream rejection.
- If workspace scoping is added, test that the executor receives the temporary
  `P4CLIENT` context and the response cannot label another stream's records as
  the requested stream.
