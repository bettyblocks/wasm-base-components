# deploy

To install dependencies:

```bash
bun install
```

To run:

```bash
bun run publish $env $version
```

For example:

```bash
bun run publish acceptance 0.1.0
```

For debugging it is also possible to run, this will not send a request to Betty Blocks, only to httpbin.org/anything for reflection:

```bash
bun run publish --dryRun acceptance 0.0.1
```

For publishing to zones, set the KEYVAULTS environment variable.
