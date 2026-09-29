# GitHub repository reader candidate

This package declares three reads: file/directory contents, branches, and commits. Its optional bundled skill helps an agent explain repository structure and recent changes. The user authorizes a GitHub App installation for selected repositories and separately grants individual capabilities to the selected Vox agent.

`schema-source.json` records the pinned public upstream snapshots used to prepare these declarations. They must match the live MCP inventory before operator publication. Provider annotations are source metadata, not behavioral certification. This directory intentionally has no passing `review.json`; it cannot be published by the CLI until independent review and live evidence are recorded.

From the repository root:

```sh
cargo run --bin vox -- package check examples/packages/github-repository-reader
# Use a test account credential supplied through a secure environment.
cargo run --bin vox -- package test examples/packages/github-repository-reader --sandbox
```

For live readiness, follow https://github.com/vox-suite/vox-deploy/blob/main/docs/github-mcp-setup.md and https://github.com/vox-suite/vox-deploy/issues/19. Publish the pinned skill version through the operator skill route before publishing the package. GitHub's current endpoint can report tools beyond these declarations; only the declared, granted tools may be exposed to the agent.
