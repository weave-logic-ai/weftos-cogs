# COG-106: Cog Host asks the daemon before a licensed cog starts

- **Status:** Accepted for the product host contract. Recorded 2026-10-06 from the extraction instruction (owner, relayed in that session). Not an implementation claim.
- **Predecessor:** WeftOS ADR-106, *The Cognitum Seed is the licence proxy for its WeftOS mesh* (2026-10-03, Proposed, review complete). This file does not renumber or amend that record, and it does not change that record's status.
- **OS and runtime:** the daemon, the mesh service, and the kernel gate stay in WeftOS ADR-106. This file is only what Cog Host does with the answer.
- **Audience:** someone running Cog Host against a Cognitum-origin cog
- **Question:** who decides whether a checked-out cog may start, and what does the host do when the answer is no?
- **Rests on:** WeftOS ADR-106, the host-visible portion (checkout grant, operator hash approval, revocation as a refusal). The socket contract below is the 2026-10-06 extraction instruction, not a behaviour read out of this checkout.

## Context

On a mesh that uses a Cognitum Seed as its licence proxy, a checkout is for the mesh, and running a checked-out cog takes both a valid checkout grant and an operator hash approval. Revocation of the artifact hash withdraws that permission. Cog Host is the process that starts the cog. It is not the licence proxy, and it must not become one.

WeftOS ADR-106 describes an earlier host check that evaluates the grant in-process. That is not the product contract. The product contract is the decision below.

## Decision

Cog Host asks the local daemon for `cog.check_run` on the Unix socket `weftos.cog.v1`. It does not decide the grant in-process.

The host sends the check before it starts a Cognitum-origin cog. The daemon decides whether a checkout grant covers that cog and that binary, whether an operator hash approval covers it, and whether the hash is revoked. The host applies the answer. It does not hold the grant key, it does not evaluate the grant, and it does not import licence records in order to reach its own verdict.

A permit means the daemon allows the start. Any refusal spelling means the host does not start the cog:

| Spelling | What the host is being told |
|---|---|
| `binding_inactive` | the mesh binding is not in force |
| `no_grant` | no checkout grant covers this cog |
| `grant_lapsed` | a grant is held but is not valid (expired, withdrawn, or its key revoked) |
| `not_in_grant` | a valid grant lists no binary with these hashes |
| `hash_revoked` | the artifact hash is revoked |
| `no_approval` | there is no operator hash approval covering this binary |
| `not_holder` | refusal spelling; the host does not start |
| `daemon_unavailable` | refusal spelling; the host does not start |

`not_holder` and `daemon_unavailable` are part of the host contract as instructed on 2026-10-06. This record does not add a definition for them beyond that: they are refusals, and the host does not start.

Transport failures fail closed. `malformed_reply`, `timeout`, and `version_mismatch` are not permits. The host does not start the cog.

Checkout grants, approvals, and revocation, as the host sees them, are those spellings. How a grant is signed, how it floods a mesh, how the Seed proxy stores it, and how the kernel gate reaches a verdict stay in WeftOS ADR-106. The host sees a permit or a spelling. It does not re-decide.

### Not in this checkout

This client does not already exist in weftos-cogs. It is being added on the WeftOS protocol branch and is not in this checkout. Searched at `056029567eb8` for `weftos.cog.v1`, `cog.check_run`, `daemon_unavailable`, `malformed_reply`, and `version_mismatch`: no matches in `*.rs`, `*.md`, or `*.toml`. Do not read a local path in this tree as the implementation of this decision. This record describes the decision, not a file to open.

## Consequences

- A Cog Host build cannot widen, forge, or skip a grant by deciding it locally. If the daemon cannot be asked, or the reply cannot be trusted, the cog stays stopped.
- The host reports the daemon's spelling. A remedy (bind, checkout, approve, renew) is a daemon and operator action, specified in WeftOS ADR-106, not a host-local edit.
- Until the protocol-branch client lands, this repository must not be documented as if the socket check were already wired.

## What remains WeftOS ADR-106

The daemon, the mesh service, and the kernel gate. Also the Seed-side licence proxy, mesh binding, checkout protocol between steward and Seed, flood and sync, and placement of a governed workload. This successor does not restate them.
