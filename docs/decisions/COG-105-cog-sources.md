# COG-105: Cog sources — a multi-repository catalog for a project

- **Status:** Accepted (product portion)
- **Decision date:** 2026-10-02 (Owner / platform, WeftOS ADR-105)
- **Recorded here:** 2026-10-06, as the public product successor. This file does not renumber or amend the WeftOS record.
- **Predecessor:** WeftOS ADR-105, *Cog sources, a multi-repository cog catalog for projects*
- **Provenance:** Imported from [weave-logic-ai/weftos](https://github.com/weave-logic-ai/weftos) tag [v0.8.3](https://github.com/weave-logic-ai/weftos/tree/v0.8.3) (`fafd6168f8f97280ee24e5c2d2813b6e34918e15`) on 2026-10-06, import commit `056029567eb8c0eea6e37f7b6958a370b3ff2385`. At that commit the paths were `crates/weftos-cog-sources` and `crates/weftos-cog-repo`. Canonical owner of the product portion: this repository. OS and runtime remain WeftOS ADR-105. See [PROVENANCE.md](PROVENANCE.md).
- **OS and runtime:** remain WeftOS ADR-105. Named below. They are not decided again here.
- **Audience:** someone configuring where a project installs cogs from, or reading the catalog
- **Question:** how does one project install cogs from WeftOS, from Cognitum, and from a registry of its own, and what is checked before a binary is trusted?
- **Rests on:** WeftOS ADR-105 sections 1–5 and 7 (product); `crates/cog-sources` (`config.rs`, `lib.rs`) and `crates/cog-repo/src/main.rs`
- **How-to:** [docs/cogs/cog-sources.md](../cogs/cog-sources.md)

## Context

A project needs more than one place to get cogs. WeftOS publishes a signed registry. Cognitum publishes a registry whose binaries are not signed and whose install is a licence question. A project also needs a registry of its own. Those three origins do not have the same trust rules, and a project file that arrives with a clone must not be able to widen the WeftOS trust anchors.

## Decision

A project pulls cogs from a configured list of sources. The catalog and the install path are per project. There are three kinds.

### Sources

A cog source is one `[[cog_source]]` entry.

| Field | Meaning |
|---|---|
| `name` | namespace, `[a-z0-9][a-z0-9_-]{0,31}`, never contains `:` |
| `kind` | `weftos`, `cognitum`, or `private` |
| `url` | a `registry.json` or `app-registry.json` URL or path, or a directory or URL prefix that holds `registry.json` |
| `pinned_keys` | Ed25519 public keys, 64 hex. Required for `private`. Refused on `weftos` (the WeftOS anchors are compiled in; a project file cannot add to them). Accepted but unused on `cognitum` |
| `allow_insecure` | default false. Development only, and only from the user file: a project file's value is dropped. Lets a `cognitum` source use `http://` or a local path |
| `priority` | integer, default 0, higher wins |
| `enabled` | default true. A disabled source stays in the file and is skipped |

The project file is `<project root>/.weftos/cog-sources.toml`. The user default is `~/.weftos/cog-sources.toml`. The effective list is the user default overlaid with the project file; a project entry with the same `name` replaces the user entry, including to disable it. Licence entries merge the same way, project first.

The file holds public keys, URLs, and licence declarations. It does not hold a secret, so it can be committed with the project. It is not inside `project.toml`: that file is the project's identity, and other writers already rewrite it.

This checkout publishes `COGNITUM_DEFAULT_URL` for a Cognitum source (`crates/cog-sources/src/config.rs`). It does not publish a matching constant for a WeftOS registry URL. The operator adds the `weftos` source explicitly.

### What each kind trusts

| Kind | Listing | Install requires | Binary check |
|---|---|---|---|
| `weftos` | signed `registry.json` (COG-008) | nothing further | size, sha256, and an Ed25519 signature by a compiled-in WeftOS anchor. Only these may label a cog WeaveLogic |
| `private` | the same format | nothing further | size, sha256, and an Ed25519 signature by a key in `pinned_keys`. The WeaveLogic key is not implicitly trusted |
| `cognitum` | Cognitum `app-registry.json` | a licence entitlement for that cog | the registry sha256 (an entry without a usable sha256 is refused). The source and the binary location must be `https://`, and a redirect from https to http is refused |

Signed-only is unchanged for `weftos` and `private`: unsigned, signed by another key, or a size or hash mismatch is refused, and the check runs before the binary is written. A `cognitum` binary is not signed. Installing it into a cog host is not the same thing as placing it as a governed workload. Governed placement stays in WeftOS ADR-105.

### Install and the declared licence

Cognitum cogs are always listable and searchable. Installing one checks, before any download, that a `[[cog_licence]]` names the source, covers the cog (a list, or `all`), and has not expired. Otherwise the install is refused with `cog_unlicensed` or `licence_expired`. An expired licence next to a valid one for the same cog does not block it.

The check is presence, coverage, and expiry. It does not verify payment and it is not a cryptographic proof. The licence is a declared local record. Commercial terms stay contractual. The record exists so a project does not install a licensed cog by mistake, and so provenance can show which declaration allowed the install.

```toml
[[cog_licence]]
source  = "cognitum"
cogs    = ["fall-detect", "baby-cry"]   # or  cogs = "all"
account = "acme"                         # informational
expires = "2027-01-31"                   # YYYY-MM-DD or RFC 3339; absent = no expiry
```

On a mesh where a Seed is the licence proxy, the host run check is [COG-106](COG-106-host-licence.md), not this declaration. Meshes without that proxy are unchanged: this declaration is the install gate.

### Private repositories

A private repo is the same signed registry format, signed by a key the project holds and pins. `weft-cog-repo` authors it: `init <dir> --name N`, `keygen --out key.pem`, `add`, `sign <dir> --key key.pem`, `verify`, `publish <dir> --to <out>`.

`keygen` writes a PKCS#8 PEM with mode 0600 and does not overwrite an existing key. It refuses to write the key inside the repo directory. `init` gitignores `*.pem`, `*.key`, and `*.seed`. `sign` refuses a key that does not match `repo.toml`. `publish` verifies every artifact, refuses to copy key material, copies to a directory, and never uploads. Hosting that directory is the operator's job.

### Resolution, install, catalog

Cog ids are namespaced `source:cog-id`. A namespaced id is looked up on that source only. A bare id is looked up on every enabled source: one match resolves; several resolve to the highest `priority`; two sources tied at the top are `ambiguous`, and the error names every namespaced form. There is no silent pick.

The resolved package records provenance: source name and kind, registry location, version, arch, binary sha256, trust (`ed25519-signed` or `cognitum-sha256`), signer public key and key id, and licence account. Install writes that to `provenance.json` beside the cog.

The catalog lists every cog of every enabled source: arch coverage, hardware requirement, resources, install access (`signed`, `licensed`, `needs-licence`, `licence-expired`), and which other sources list the same id. Counts of clean or excluded cogs are not restated here; they were not recomputed for this record.

## What remains WeftOS ADR-105

The OS and runtime portions stay in WeftOS ADR-105 and are not part of this successor:

- the kernel package-trust fixes, and the compiled-in WeftOS package-signer set
- emitting a `cog.source.resolved` event onto a chain (the provenance file is the record in the product install)
- packing a verified registry artifact into a governed workload

## Consequences

A project chooses and orders its sources. A private registry is a first-class source. An unlicensed Cognitum install is refused before any download. Every install records where it came from and what vouched for it. A licence declaration is not payment enforcement, and a registry install is not yet a governed placement.
