# Select the helper component by the interface that it exports

## Status

**Accepted** on 3 November 2026. We did not implement this decision yet.

The comparison of all options is in
[helper-component-version-decision-matrix.xlsx](helper-component-version-decision-matrix.xlsx).
[helper-component-version-explainer.html](helper-component-version-explainer.html) explains the
options with diagrams. Open the HTML file in a browser.

## Context

### One number for two versions

A helper component, for example `crud`, has two versions:

- The **interface version** is the version of the WIT package in `wit/`, for example
  `betty-blocks-types:crud@4.0.0`. It tells which functions a step can call. Only a WIT change
  increases it.
- The **component version** is the version in `components/<c>/wit/world.wit`, for example
  `betty-blocks-utilities:crud@4.0.1`. It tells which code operates. Each release increases it.

A native step, for example create, imports the interface. It does not import the component.
This is how WIT operates: an `import` line names an interface, not the component that exports it.
Two systems select the component:

- actions-compiler selects the components that go into the workload of an app.
- wasmCloud links each import to the component in the same workload that exports the same
  interface version (`wash-runtime`, `link_components`). If two components in the workload
  export the same interface, wasmCloud cannot select one. Thus a workload has one build for each
  helper interface.

actions-compiler uses the interface version that the step imports as the image tag of the
helper (`lib/actions_compiler/wasm.ex`, `image_tag/2`). For create, it deploys `crud:4.0.0`. The
crud component 4.0.1 also exports `betty-blocks-types:crud/crud@4.0.0`, but no app operates it.
Thus a helper release without a WIT change does not go to the apps.

### The two workarounds of today

- **Increase the version of the types package** for each helper release. Then each step that
  imports the helper must have a new release with the new import. For data-api, a database
  migration is also necessary (see the next section). A bump in the same major version also
  starts the semver merge of [ADR 001](001-major-only-wit-version-bumps.md).
- **Pin the component in actions-compiler.** `config/runtime.exs` sets a `component_version`
  next to the pinned WIT `version`. Commit `18ccd50d` in actions-compiler does this for data-api.
  Each helper and each interface version that steps import must have its own pin. The team keeps
  the pins in sync manually. The list only grows, because old steps keep their old interface
  versions. The compiler also deploys each pin in each workload, also when no step uses it.

### The data-api resource

All steps of an action borrow one data-api resource. A resource type is nominal: the interface
version that defines the type gives its identity. Thus each step that borrows the resource must
import the same data-api interface version. A different version, also a different minor version,
stops the call with `mismatched resource types`. This error occurred on edge on 16 October 2026.

When the data-api interface version changes, a database migration must change the old steps that
borrow the resource (crud, create, update, delete) to the latest step version. Other steps keep
their old version and operate correctly. Only the shared resource makes this migration
necessary.

This ADR does not solve this problem. After this change, a change to the data-api interface
still needs the migration. A different decision must solve it before the next change to the
data-api interface.

## Decision

We use option A of the matrix: actions-compiler selects the helper build from the registry. An
OCI annotation on each image names the interfaces that the component exports.

The interface version that the step imports stays the start point. The compiler uses this
version as a filter, not as the image tag.

### Publish

`scripts/publish-components.sh` adds the annotation `com.bettyblocks.exports` to each component
image. The value is the list of interfaces in the `export` lines of the `world.wit` of the
component:

```bash
# world.wit of crud 4.0.1:  export betty-blocks-types:crud/crud@4.0.0;
exports=$(grep -oP '^\s*export\s+\K[^;]+' "$world" | paste -sd, -)

for tag in "${tags[@]}"; do
  wkg oci push "$image:$tag" "$wasm" --annotation "com.bettyblocks.exports=$exports"
done
```

`wkg oci push` has the `--annotation` option (wkg 0.16.1).

Example: the publish of crud 4.0.1 from commit `aa6ca30` pushes these tags:

```bash
wkg oci push ghcr.io/bettyblocks/crud-component:4.0.1-aa6ca30 crud.wasm \
  --annotation "com.bettyblocks.exports=betty-blocks-types:crud/crud@4.0.0"
wkg oci push ghcr.io/bettyblocks/crud-component:4.0.1 crud.wasm \
  --annotation "com.bettyblocks.exports=betty-blocks-types:crud/crud@4.0.0"
```

The manifest of each tag then contains the annotation:

```json
{
  "schemaVersion": 2,
  "mediaType": "application/vnd.oci.image.manifest.v1+json",
  "annotations": {
    "com.bettyblocks.exports": "betty-blocks-types:crud/crud@4.0.0"
  }
}
```

A component that exports more than one interface has a comma-separated list.

### Compile

Many steps import the same helper interface. The compiler reads the registry one time for each
helper, not one time for each step:

1. Collect the different helper interfaces that the steps of the app import.
2. For each helper, read the tags of the component repository one time.
3. Keep only the release tags `X.Y.Z`. Ignore `latest` and `X.Y.Z-<sha>`.
4. Read the digest and the annotation of each tag one time.
5. For each imported interface version, keep the tags that export it. Select the tag with the
   highest component version.
6. Deploy that image by digest, not by tag.

The example below shows the selection for one interface.

```elixir
# interface = "betty-blocks-types:crud/crud@4.0.0", the version that the step imports
def resolve_image(config, interface) do
  repo = config.component_repository

  candidates =
    Registry.list_tags(repo)
    |> Enum.filter(&release_tag?/1)
    |> Enum.map(fn tag ->
      digest = Registry.head_manifest(repo, tag)
      %{tag: tag, digest: digest, exports: exports_for(repo, digest)}
    end)
    |> Enum.filter(&(interface in &1.exports))

  case candidates do
    [] -> {:ok, "#{repo}:#{interface_version(interface)}"}
    _ ->
      newest = Enum.max_by(candidates, &Version.parse!(&1.tag), Version)
      {:ok, "#{repo}@#{newest.digest}"}
  end
end
```

Example for create, which imports `betty-blocks-types:crud/crud@4.0.0`:

| Tag | Annotation | Result |
| --- | --- | --- |
| `4.0.0` | `betty-blocks-types:crud/crud@4.0.0` | Candidate |
| `4.0.1` | `betty-blocks-types:crud/crud@4.0.0` | Selected: the highest candidate |
| `5.0.0` | `betty-blocks-types:crud/crud@5.0.0` | Ignored: a different interface |

### Rules

- **The interface version must match exactly.** wasmCloud links an import only to an export
  with the same interface version. There is no fallback to a different version. A step that
  imports `crud/crud@4.0.0` needs a component that exports `crud/crud@4.0.0`. The component
  version has no effect. A spike on 3 November 2026 confirmed this with wash-runtime 2.10.3.
- **Labelled imports.** wasmCloud also links a labelled import, for example
  `import crud: …/crud@4.0.0;`, to an export with the same major version. We do not use this
  now: each step must then have a new release.
- **Keep a cache by digest.** The bytes of a digest never change, so its annotation never
  changes. The compiler can keep the annotation of each digest in a cache. The tag `X.Y.Z` moves
  with each publish, so the compiler must read the digest of each tag at each compile.
- **Images without an annotation.** The images that we published before this change have no
  annotation. If no tag has a matching annotation, the compiler uses the image tag of today: the
  interface version.
- **data-api.** All steps import the pinned data-api interface version. The compiler selects one
  build for that version. Thus the action still has one data-api resource. The `component_version`
  pin in actions-compiler is not necessary after this change.

### Release and rollback

- **Without a WIT change:** increase only the component version (crud 4.0.0 → 4.0.1) and
  publish. Each app receives the new build at its next compile. A step release is not necessary.
  The component version can increase by a patch, a minor or a major version, for example crud
  4.0.0 → 4.1.0 for a new feature. The selection uses only the interface version that the
  component exports. Thus crud component 4.1.0 that exports `crud/crud@4.0.0` serves create.
- **With a WIT change:** increase the interface version and the component version. Release the
  steps that use the new interface with the new import. For data-api, also do the database
  migration of the steps that borrow the resource.
- **Rollback:** publish the correct code as a new release with a higher version, for example
  crud 4.0.2. The compiler then selects 4.0.2.

After this change, the rollback workflow in the
[README](../../README.md#republishing-an-old-build-and-rollback) is not sufficient. That workflow
moves the tags of the old build back. The tag of the bad build stays, and it has the highest
version. Thus the compiler continues to select the bad build.

## Consequences

### Advantages

- A helper fix, for example a security fix, goes to each app automatically at its next compile.
  A step release and a change in actions-compiler are not necessary.
- wasm-base-components owns the relation between builds and interfaces. actions-compiler has no
  list of pins.
- The compiler deploys by digest. The deployment of an app does not change when a tag moves.
- The design is simple. The rule is in one location in the compiler.

### Disadvantages

- Old steps also receive the new build, and nobody tested them with it. A patch release or a
  minor release can also contain a bug. Edge receives each release first, because production
  publishes only from `main`. We accept this risk. We continue to test the functionality of
  the helpers, and we want security fixes and other fixes to go to the apps automatically.
- A rollback is a new publish. Until that publish, each compile in the zone selects the bad
  build.
- Each compile makes more calls to the registry. This cost is small: the compile already reads
  the WIT packages from the registry. The pins and their manual updates go away, and with them
  the errors of manual work.

## Other options

The [matrix](helper-component-version-decision-matrix.xlsx) gives the full comparison of each
option and the reason why we did not select it.

## Work to implement

1. wasm-base-components: add the annotation in `scripts/publish-components.sh`.
2. actions-compiler: read the tags and the manifests from the registry. Select the build, deploy
   by digest, and keep the cache. Use the fallback for images without an annotation.
3. actions-compiler: remove `component_version` from `config/runtime.exs`.
4. wasm-base-components: when actions-compiler uses the new selection, update `AGENTS.md`. Add
   the release rules of [Release and rollback](#release-and-rollback): a helper release without
   a WIT change increases only the component version.
5. wasm-base-components: at the same time, update the rollback workflow in the
   [README](../../README.md#republishing-an-old-build-and-rollback). A rollback is then a new
   release with a higher version, not a move of the tags.

## References

- actions-compiler `lib/actions_compiler/wasm.ex`: `build_base_component_instances/1` and
  `image_tag/2`, at commit `18ccd50d`.
- actions-compiler `config/runtime.exs`: the `base_components` configuration.
- wasmCloud `crates/wash-runtime/src/engine/workload.rs`: `link_components` links the imports
  of a component to the exports of the other components in the workload.
- [scripts/publish-components.sh](../../scripts/publish-components.sh): the tags of each publish.
- [ADR 001](001-major-only-wit-version-bumps.md): the semver merge in one compatibility class.
