# Shared WIT dependency fetching, imported by every component Justfile.
#
# source_directory() is this file's directory (the repo root), not the importer's.

repo_root := source_directory()

# wkg reads its registry config from here. Exported, so nested `wkg` calls inherit it too.
export WKG_CONFIG_FILE := repo_root / "wkg-config.toml"

# Recipes run in the directory of the Justfile that imported them, so this acts on that
# component.
fetch-wit-deps: wkg-fetch

# Split out so a component can override fetch-wit-deps and still reuse this.
[private]
wkg-fetch:
    {{ repo_root }}/scripts/generate-wkg-toml.sh
    wkg wit fetch
