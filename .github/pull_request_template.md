## Usage related changes

<!-- How the changes from this PR affect users. -->

## Development related changes

<!-- How these changes affect the developers of this project. E.g. changes in dev tools, testing, CI/CD... -->

## Checklist:

<!-- If you are not able to complete one of these steps, you can still create a PR, but note what caused you trouble. -->

- [ ] Checked the [contribution guide](https://github.com/starknet-io/starknet-devnet/blob/main/.github/CONTRIBUTING.md) and [agent guide](https://github.com/starknet-io/starknet-devnet/blob/main/AGENTS.md) where applicable
- [ ] Applied formatting - `npm --prefix website ci && ./scripts/format.sh`
- [ ] No routine validation errors - `./scripts/verify.sh`
- [ ] No unused dependencies - `./scripts/check_unused_deps.sh` (when Rust manifests changed)
- [ ] No spelling errors - `./scripts/check_spelling.sh`
- [ ] Ran integration tests - `./scripts/test_integration.sh` (when server, RPC, CLI, contracts, or integration tests changed)
- [ ] Regenerated packaged UI assets - `npm --prefix web-ui run build:devnet` (when `web-ui` changed)
- [ ] Performed code self-review
- [ ] Rebased to the latest commit of the target branch (or merged it into my branch)
    -   Once you make the PR reviewable, please avoid force-pushing
- [ ] Updated the docs if needed - `./website/README.md`
- [ ] Linked the [issues](https://github.com/starknet-io/starknet-devnet/issues) resolvable by this PR - [linking info](https://docs.github.com/en/issues/tracking-your-work-with-issues/linking-a-pull-request-to-an-issue#linking-a-pull-request-to-an-issue-using-a-keyword)
- [ ] Updated the tests if needed; all passing - [execution info](https://github.com/starknet-io/starknet-devnet/blob/main/.github/CONTRIBUTING.md#testing)
