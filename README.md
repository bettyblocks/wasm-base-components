# Actions Providers

Repo for providers to give access to Betty Blocks services

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

