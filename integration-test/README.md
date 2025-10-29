# Integration tests

Written in Elixir.

Get the dependencies. Make sure the providers are compiled.

```
mix deps.get
```

you can just run:

```
mix test
```

This will use testcontainers to startup wasmcloud and run the tests.

For quicker debugging and writing tests you can start wasmcloud with `wash up` and then you run (this will not use testcontainers):

```
wash up -d
CI=1 mix test
```
