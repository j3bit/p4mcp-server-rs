# Review Comments API Alignment

Status: deferred from PR #2 to keep the Rust port aligned with upstream
`perforce/p4mcp-server`.

## Reason

The Rust port currently mirrors upstream `perforce/p4mcp-server`
`v2026.2.2955897` at commit `a64efb07511b2a62db41aeed110ab96744c4076a`.
Checked on 2026-06-17, `origin/main` for upstream still points at that same
commit.

The PR review feedback is valid as a Swarm API correctness issue, but not as a
Rust-port parity bug:

- Upstream `query_reviews.comments` exposes `comments_fields` and
  `max_results`, but the handler only passes `review_id` to
  `get_review_comments`.
- Upstream `get_review_comments` calls `GET /reviews/{id}/comments` without
  `max` or `fields` query parameters.
- Upstream `add_review_comment` and `reply_to_comment` call
  `POST /reviews/{id}/comments`.
- Swarm v11 documentation describes the first-class comments API as
  `GET /comments/reviews/{id}` and `POST /comments/reviews/{id}`, with
  `max` and `fields` supported for comment listing. Replying can also use
  `POST /comments/{comment_id}`.

Do not change the current PR solely to satisfy the review comments; that would
diverge from the upstream-parity baseline. Treat this as a deliberate follow-up
extension or upstream bugfix port.

## Future Design

### `query_reviews.comments`

- Route review comment listing through the first-class comments endpoint:
  `GET /comments/reviews/{review_id}`.
- Preserve `max_results` as the Swarm `max` query parameter.
- Preserve `comments_fields` as the Swarm `fields` query parameter.
- Consider whether `after`, `tasksOnly`, `taskState`, and `ignoreArchived`
  should be exposed later, but do not add them unless there is a clear product
  need.

### `modify_reviews.add_comment`

- Route add-comment writes through `POST /comments/reviews/{review_id}`.
- Preserve the existing body, `taskState`, `notify`, and comment context
  semantics.
- Keep the write approval gate unchanged: approval preview first, HTTP write
  only after approval.

### `modify_reviews.reply_comment`

- Prefer one reply form and document it in tests:
  - `POST /comments/reviews/{review_id}` with `context.comment`, or
  - `POST /comments/{comment_id}` with `topic`.
- Do not silently drop `review_id` unless the chosen endpoint no longer needs
  it and the public MCP schema is adjusted deliberately.

## Reintroduction Checklist

- Update request-mapping tests before implementation so the endpoint change is
  intentional.
- Add or update server-to-HTTP smoke tests for `query_reviews.comments`,
  `modify_reviews.add_comment`, and `modify_reviews.reply_comment`.
- Keep existing read-state endpoints unchanged unless separately justified:
  `POST /comments/{id}/read`, `POST /comments/{id}/unread`,
  `POST /reviews/{id}/comments/read`, and
  `POST /reviews/{id}/comments/unread`.
- Document the upstream divergence in the commit or PR notes.
