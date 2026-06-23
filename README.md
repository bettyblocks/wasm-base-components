# Wasm base components

Repo that contains the shared components for the Betty Blocks platform. These components can be wasm assembly (WASI) or native plugins that run directly on the server.

The components currenly include:

- crud-component
- data-api
- key-vault
- smtp
- http-wrapper
- log-to-stdout
- logs-writer

What it doesn't include:

- the actual customer actions
- functions/components that can be imported in Betty Blocks

### note that wash providers break when using a rust workspace for some reason

## Release & Deployment

1. merge to main branch
2. wait until pipeline succeeds
3. wait until Publish WASM Components to GitHub Packages succeeds
4. go to Deploy WADM to wasmcloud, select the environment and click the button
   - if the run id is empty it will take the last succesfull merge pipeline to the main branch

### for Production

Run step 4. with a specific run id. The run id can be found by going to the Build WASM Components pipeline you want to deploy and look at the last part of the url.
Note: the artifacts are deleted after 90 days

## Local Setup

- install [rust](https://rust-lang.org/tools/install/)
- install [wash](https://wasmcloud.com/docs/installation/)
- install [just](https://github.com/casey/just)
- install [bun](https://bun.sh/) (for semantic-release)

## Local Build

- just build

## Local Test

See the [./integration-test](./integration-test) folder

## Repo Layout

- Justfile: contains commands to run commands
- components: contains wasm components that are not action steps
- deploy: contains the code/scripts to deploy the native application
- integration-test: Contains the tests to verify that the providers work in wasmcloud
- wit: contains shared WIT interface definitions used by the wasm components
- .github/workflows: CI/CD pipelines for building, releasing, and publishing
