# Wasm base components

Repo that contains the shared components for the Betty Blocks platform. These components can be wasm assembly (WASI) or native plugins that run directly on the server.

The components currenly include:
- crud-component
- data-api
- key-vault
- http-wrapper
- types
- http-server

What it doesn't include:
- the actual customer actions
- functions/components that can be imported in Betty Blocks

### note that wash providers break when using a rust workspace for some reason

## Setup

- install [rust](https://rust-lang.org/tools/install/)
- install [wash](https://wasmcloud.com/docs/installation/)
- install [just](https://github.com/casey/just)

## Build

- just build

## Test locally

See the [./integration-test](./integration-test) folder

## Layout

- Justfile: contains commands to run commands
- providers: contains code that needs state and/or os-level access
- integration-test: Contains the tests to verify that the providers work in wasmcloud

