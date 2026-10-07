# Release channels

A tagged release reaches users through three channels: the GitHub release assets
(built by the `build` and `checksums` jobs), the Homebrew tap
`pragmatic-engineer/homebrew-playbook`, and the plugin marketplace
`pragmatic-engineer/marketplace`. The `publish-channels` job in
`.github/workflows/release.yml` updates the last two after `checksums` succeeds.

What it does, on a tag push only and only when the tag is the repo's latest
release (so a backport tag cannot roll users back):

1. Renders `Formula/playbook.rb` from `shell/formula.rb.tmpl` and the release's
   `SHA256SUMS` (`shell/render-formula.sh`), and pushes it to the tap.
2. Pins the `playbook` entry of the marketplace's `marketplace.json` to the tag
   with `ref` (`shell/pin-marketplace.sh`), and pushes it.
3. Reads both repos back and fails with a separate `::error::` per channel if
   either does not show the released version.

## The secret

The workflow token cannot write to other repos, so the job needs the secret
`RELEASE_PUSH_TOKEN`: a fine-grained personal access token with
`contents: write` on both `homebrew-playbook` and `marketplace`. Writes go
through the contents API, so GitHub signs the commits.

Store it as an environment secret on the `release` environment (Settings,
Environments, `release`), not as a repository secret. The job runs in that
environment, and the environment only accepts deployments from `v*` tags, so a
workflow on any other branch or tag cannot read the token.

Without the secret the job prints a warning and passes. The release is still
valid, but the tap and marketplace stay on the previous version until updated
by hand.
