import {parseArgs} from 'node:util';
import type { Environment, Variables } from "./render";
import { renderToFile } from "./render";

const REGISTRY = "ghcr.io/bettyblocks";
const KEYVAULTS = JSON.parse(process.env.KEYVAULTS || "{}");

const determineKeyvaultEndpoint = (env: Environment, zone: string) => {
	switch (env) {
		case "edge":
			return "https://betty-edge-keyvault.vault.azure.net/";
		case "acceptance":
			return "https://betty-acc-keyvault.vault.azure.net/";
		case "production": {
			const keyvault = KEYVAULTS[zone];
			if (!keyvault) {
				throw new Error(`Zone not found, for zone: ${zone}`);
			}

			return keyvault;
		}
		default:
			throw new Error("Invalid environment");
	}
};

function formatUrl(zone: string) {
	return `https://${zone}.betty.zone/api/actions-compiler/internal/wadm/deploy_native_app`;
}

async function renderAndDeploy(
	env: Environment,
	version: string,
	zone: string,
	dryRun: boolean | undefined,
) {
	const wadmPath = `wadm/wadm.${env}.${zone}.yaml`;
	const variables: Variables = {
		ENVIRONMENT: env,
		REGISTRY: REGISTRY,
		VERSION: version,
		KEYVAULT_ENDPOINT: determineKeyvaultEndpoint(env, zone),
	};
	await renderToFile(variables, wadmPath);
	let url = formatUrl(zone);

	const formData = new FormData();
	formData.append("file", Bun.file(wadmPath), "wadm.yaml");
	formData.append("version", version);

  if (dryRun) {
    console.log("Dry run, skipping deployment, request:");
    console.log({url, options: { method: "POST", body: formData }});
    url = "https://httpbin.org/anything";
  }

	const response = await fetch(url, { method: "POST", body: formData });
	if (!response.ok) {
		throw new Error(
			`Failed to upload to registry: ${response.statusText}, ${await response.text()}`,
		);
	}
	const data = await response.text();
	console.log(data);
}

async function main() {
  const {values: {dryRun}, positionals: [,,env, version]} = parseArgs({
    args: Bun.argv,
    options: {
      dryRun: {
        type: 'boolean',
      },
    },
    strict: true,
    allowPositionals: true,
  })

  if (!env) {
		throw new Error("Environment is required");
	}
	if (!version) {
		throw new Error("Version is required");
	}

  if (env !== "edge" && env !== "acceptance" && env !== "production") {
		throw new Error(`Invalid environment, got: ${env}`);
	}

	if (env === "production") {
		const tasks = [];
		for (const zone of Object.keys(KEYVAULTS)) {
			tasks.push(renderAndDeploy(env, version, zone, dryRun));
		}
		await Promise.all(tasks);
	} else {
		await renderAndDeploy(env, version, env, dryRun);
	}
}

main();
