import { $ } from "bun";

const BETTY_PREFIX = "betty-";

async function getKeyvaults(accountId) {
  const keyvaults = await $`az keyvault list --subscription ${accountId}`.json();
  return keyvaults.map(keyvault => keyvault.name).filter(name => name.startsWith(BETTY_PREFIX));
}

async function getKeyvaultInfo(accountId, keyvault) {
  let keyvaultInfo = await $`az keyvault show --subscription ${accountId} --name ${keyvault}`.json();
  const zone = keyvault.split("-")[1];
  return [zone, keyvaultInfo.properties.vaultUri];
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
  tasks2.push(getKeyvaultInfo(accountId, keyvault))
}
let results = await Promise.all(tasks2)
const zoneLookup = Object.fromEntries(results.filter(([key]) => key !== "backup").toSorted());
console.log(JSON.stringify(zoneLookup, null, 2));