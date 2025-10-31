import { $ } from "bun";

const BETTY_PREFIX = "betty-";
const ISSUER = "github";

async function getKeyvaults(accountId) {
  const keyvaults = await $`az keyvault list --subscription ${accountId}`.json();
  return keyvaults.map(keyvault => keyvault.name).filter(name => name.startsWith(BETTY_PREFIX));
}

async function getKeyvaultSecrets(accountId, keyvault) {
  if (keyvault.includes("backup")) {
    return [];
  }
  const secrets = await $`az keyvault secret show --name actions-compiler-secrets --subscription ${accountId} --vault-name ${keyvault}`.json();
  const zone = keyvault.split("-")[1];
  return [zone, JSON.parse(secrets.value)["ACTIONS_COMPILER_GITHUB_SECRET"]];
}

function formatSecrets(secrets) {
  let services = Object.entries(secrets).map(([zone, secret]) => {
    return {
      [zone]: { secret },
    };
  });

  return {
    issuer: ISSUER,
    services,
  }
}

const accounts = await $`az account list`.json();
const accountIds = accounts.map(account => account.id);
const keyvaultsWithAccountId = {};
const tasks = [];
for (const accountId of accountIds) {
  tasks.push(getKeyvaults(accountId).then(keyvaults => {
    keyvaults.forEach(keyvault => {
      keyvaultsWithAccountId[keyvault] = accountId;
    });
  }));
}
await Promise.all(tasks);

const tasks2 = [];
for (const [keyvault, accountId] of Object.entries(keyvaultsWithAccountId)) {
  tasks2.push(getKeyvaultSecrets(accountId, keyvault))
}
let results = await Promise.all(tasks2)
const secretsLookup = Object.fromEntries(results.filter(([key]) => key).toSorted());
console.log(JSON.stringify(formatSecrets(secretsLookup), null, 2));
