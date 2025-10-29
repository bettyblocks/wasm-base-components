# note that wash providers break when using a rust workspace for some reason

build:
  wash build --config-path providers/data-api
  wash build --config-path providers/key-vault

build-test:
  wash build --config-path integration-test/components/fetcher

deploy:
  wash app deploy --replace wadm.yaml

upload:
  wash push --insecure --registry registry.services.docker data-api:0.1.0 ./providers/data-api/build/data-api.par.gz
  wash push --insecure --registry registry.services.docker key-vault:0.1.0 ./providers/key-vault/build/key-vault.par.gz
