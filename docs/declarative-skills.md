# Declarative skills

Vox accepts a bounded subset of the [Agent Skills format](https://agentskills.io/specification): `SKILL.md` frontmatter and Markdown guidance, plus small text files under `references/` or `assets/`. Executable scripts, symlinks and escaping or unresolved resource links are rejected. `allowed-tools` expresses requested capability names; it never creates an agent grant.

Create `my-skill/SKILL.md` with `name`, `description`, and optional `license`, `compatibility`, `metadata`, and `allowed-tools`. Check it with `cargo run --bin vox -- skill check ./my-skill`. The CLI and Core import API use the same validator.

Web Library offers create, preview, and import. Its signed `POST /api/account/skills/import` forwards a bounded file map to Core's signed `POST /v1/skills/import`. Preview does not save. Saving a private skill does not enable it for an agent. Curated skills are deployment-reviewed and remain uninstalled until a user selects them. Installation may enable the exact reviewed skill version for one selected agent in a single transaction.

Core retains immutable title, summary, guidance, resources, capability requests, and digest per version. Installed versions stay pinned until the user reviews an update. At runtime, agents discover only enabled metadata and load the pinned guidance on demand. Disabling or revoking access stops the next load.
