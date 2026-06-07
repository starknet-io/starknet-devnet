# Version release

To release a new Devnet version, follow these steps:

1. Increment the semver in Cargo.toml of those Devnet crates that have changed. Use `scripts/check_crate_changes.sh` for this.

2. Update `website/docs/compatibility.md` by adding the new Devnet version row and adjusting any needed row spans.

3. Add a documentation entry for the incoming version (without the v- prefix) by running:

   ```
   $ npm --prefix website run docusaurus docs:version <VERSION>
   ```

   - Feel free to delete documentation entries of old release candidates. E.g. if releasing 0.5.0, delete all files containing in them or in their names references to 0.5.0-rc.2. Or if releasing 0.5.0-rc.2, delete the entry of 0.5.0-rc.1.

4. Create a PR styled after [this one](https://github.com/starknet-io/starknet-devnet-rs/pull/473).

5. Adapt [starknet-devnet-js](https://github.com/starknet-io/starknet-devnet-js) to the newly released Devnet. Check out one of the [old adaptation PRs](https://github.com/starknet-io/starknet-devnet-js/pulls?q=is%3Apr+is%3Aclosed) for reference.

6. Update `starknet-foundry` to use the latest Devnet, if possible. Use [this PR](https://github.com/foundry-rs/starknet-foundry/pull/3434) for reference.
