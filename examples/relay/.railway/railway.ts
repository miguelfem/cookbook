import { defineRailway, github, project, service, volume } from "railway/iac";

export default defineRailway((ctx) => {
  const data = volume("relay-data", {});
  const relay = service("relay", {
    source: github("miguelfem/cookbook", { branch: "main", rootDirectory: "examples/relay" }),
    healthcheck: "/healthz",
    replicas: 1,
    volumeMounts: { "/data": data },
    env: {
      APR_RELAY_RECIPES: ctx.shared.APR_RELAY_RECIPES,
      APR_RELAY_SUBMIT_TOKEN: ctx.shared.APR_RELAY_SUBMIT_TOKEN,
      APR_RELAY_WORKER_TOKEN: ctx.shared.APR_RELAY_WORKER_TOKEN,
    },
  });
  return project("CookBook Relay", { resources: [relay, data] });
});
