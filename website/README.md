# Website

This website is built using [Docusaurus](https://docusaurus.io/), a modern static website generator.

### New content

Check out the existing pages and categories under `website/docs/`. If the change you wish to make in the documentation is not suitable for any of the existing markdown (.md) files, feel free to create a new one.

### Documentation versioning

A new version of the documentation should be created when releasing a new crate. Check [the release guidance](../.github/CONTRIBUTING.md#releasing) for more information.

#### Updating versioned documentation

Generally, you should only be making changes to the pages in `website/versioned_docs/` if something important is applicable to those historic versions, but is missing in the documentation.

### Installation

```
$ npm ci
```

### Format

Format the website code by running

```
$ npm run format
```

### Local development

```
$ npm run start
```

This command starts a local development server and opens up a browser window. Most changes are reflected live without having to restart the server.

### Build

```
$ npm run build
```

This command generates static content into the `build` directory and can be served using any static contents hosting service.

### Validation

From the repository root, run `npm --prefix website ci && ./scripts/check_website.sh` to type-check and build the site. Run `npm --prefix website ci && ./scripts/format_check.sh` to check Rust and website formatting together.

### Deployment

Using SSH:

```
$ USE_SSH=true npm run deploy
```

Not using SSH:

```
$ GIT_USER=<Your GitHub username> npm run deploy
```

If you are using GitHub pages for hosting, this command is a convenient way to build the website and push to the `gh-pages` branch.
