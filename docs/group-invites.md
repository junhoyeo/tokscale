# Group invitation limits

Invitations default to one join and expire after 14 days. Owners and admins can create links with 1–1000 successful joins and expiry within 1–30 days. An account-specific invitation always has one use. Existing permissions still apply: only owners can grant the admin role, and nobody can grant ownership through an invite.

A link can be used once per account. Leaving a group does not erase the redemption. Existing members receive HTTP 409 without changing their role or consuming capacity. A failed membership insert rolls back both the counter and redemption. When the last join commits, status becomes `accepted`; partially used links remain `pending`. Revoked, expired, and exhausted links cannot admit new members.

Creation and redemption lock the inviter's membership to serialize against role changes. Redemption atomically claims available capacity and checks wall-clock expiry in PostgreSQL, including after lock waits. Revocation follows the membership-before-invite lock order. A redemption racing with revocation either commits first or is rejected; revocation does not remove existing members.

The management panel shows the latest 100 invitations, their usage and expiry, and permits revocation of pending links. Raw tokens are returned only at creation; lists never return the token or its hash. Save a link before navigating away. Invite previews and management responses use private, nonpersistent HTTP caching.

## API

- `POST /api/groups/:slug/invite`: optional `role`, `invitedUsername`, `maxUses` (default 1), `expiresInDays` (default 14). Returns the newly created link.
- `GET /api/groups/:slug/invite`: recent management metadata; admins see member-role invitations only.
- `DELETE /api/groups/:slug/invite`: `{ "inviteId": "uuid" }`; checks current role inside the transaction.
- `GET /api/groups/join/:token`: preview includes `remainingUses`.
- `POST /api/groups/join/:token`: authenticated account redemption. Cookie mutations retain the existing Origin checks.

## Migration and rollout

`0032_late_nomad.sql` was generated with `drizzle-kit generate`. It adds bounded counters and an account/invitation redemption key. Previously accepted invitations are backfilled to one used slot; other legacy invitations remain single-use. No historical redemption accounts are inferred. The ALTER statements take normal PostgreSQL schema locks; schedule the migration accordingly for a busy database.

Apply the migration before deploying the application. Finish replacing old application instances before enabling reusable links: old code treats every invitation as single-use. For an application rollback, revoke pending invitations with `max_uses > 1` first so they are not unexpectedly consumed by the old code. Keep the additive schema and redemption history; reverting code is not a data rollback.

## Verification

Run the regular frontend tests and typecheck, then `GROUP_INVITES_DB_INTEGRATION=1 bun run test -- __tests__/integration/groupInvitesPostgres.test.ts` against a migrated disposable PostgreSQL database. The database suite uses multiple connections to exercise competing accounts, repeated requests, different inviters, rollback, revocation, demotion and expiry during a lock wait.
