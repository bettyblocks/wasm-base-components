import wadm from "./template.wadm.yaml"

export type Environment = "edge" | "acceptance" | "production";
export type Variables = {
  ENVIRONMENT: Environment;
  VERSION: string;
  REGISTRY: string;
  KEYVAULT_ENDPOINT: string;
}

type Json = string | number | boolean | null | Json[] | { [key: string]: Json };

function replaceVariables(config: Json, variables: Variables): Json {
  if (typeof config === "string") {
    for (const [key, value] of Object.entries(variables)) {
      config = config.replaceAll(`{{${key}}}`, value);
    }
    return config;
  }

  if (Array.isArray(config)) {
    return config.map((item) => replaceVariables(item, variables));
  }

  if (typeof config === "object" && config !== null) {
    for (const key in config) {
      const value = config[key];
      if (value !== undefined) {
        config[key] = replaceVariables(value, variables);
      }
    }
  }
  return config;
}

export const render = async (options: Variables): Promise<Json> => replaceVariables(wadm, options)

export const renderToFile = async (options: Variables, filePath: string): Promise<Bun.BunFile> => {
  const rendered = await render(options);
  const file = Bun.file(filePath);
  await file.write(Bun.YAML.stringify(rendered, null, 2));
  return file;
}
