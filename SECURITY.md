# Security

Report a vulnerability through a GitHub private advisory:

<https://github.com/weave-logic-ai/weftos-cogs/security/advisories/new>

Do not open a public issue for a security report.

## What does not belong in this repository

Do not commit device keys, `.dev.vars`, licence seeds, tailnet addresses, or client data.

`WEAVELOGIC_RELEASE_KEY` is not stored here. The matching public key is compiled into `cog-repo`. Publishing a signed WeaveLogic registry happens on a controlled host. CI does not sign releases.

A private repository key from `weft-cog-repo keygen` stays outside the tree. The command refuses a key path inside the repository and does not overwrite an existing key file.
