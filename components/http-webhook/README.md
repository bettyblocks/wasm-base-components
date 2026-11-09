# Http-webhook component

The http-webhook component runs an action of an app when it receives an HTTP request. One workload runs for each app. A config map gives the action for each webhook name.

## HTTP contract

- Endpoint: `GET` or `POST /api/webhooks/{webhook-name}`
- The public URL is `https://<app-identifier>.<zone>.betty.app/api/webhooks/{webhook-name}`. The gateway sends the request to the webhook workload of the app.
- Body of a `POST`: optional. Use a JSON object with the input variables of the action. An empty body gives `{}`.
- A `GET` has no body. The action gets the input `{}`.
- Header: `Authorization: Bearer <token>` when the action has an authentication profile.

The response is the output of the action as JSON, with status `200`.

Errors have the body `{"error": "<message>"}`:

| Status | Cause |
| ------ | ----- |
| `400` | The body is not a JSON object. |
| `401` | There is no token, or the token is not valid. The response has `WWW-Authenticate: Bearer`. |
| `403` | The token is valid, but it is for a different authentication profile. |
| `404` | The path is not `/api/webhooks/{webhook-name}`, or the app has no webhook with this name. |
| `405` | The method is not `GET` or `POST`. |
| `413` | The body is larger than 10 MB. |
| `500` | The configuration of the workload is not correct. |
| `502` | The action failed. |
| `503` | The token could not be validated. |

## Configuration

The component reads these values:

- `webhooks` (wasi:config): a JSON object that gives the action ID for each webhook name.
- `actions` (wasi:config): a JSON object that gives the authentication profile for each action ID. The auth component reads the same key.
- `betty_config` (wasi:config): the configurations that the action receives, as a JSON object.
- `APPLICATION_ID` (environment): the app of the actions.
- `WASMCLOUD_HOST` (environment): the gateway that runs the actions.

```json
{
  "webhooks": { "send-invoice": "4d1f...", "ping": "9a7c..." },
  "actions": {
    "4d1f...": { "authentication-profile-id": "profile-1", "kind": "api-key" },
    "9a7c...": { "authentication-profile-id": null }
  }
}
```

The values of the keys are JSON strings. The example shows them as objects to make them easy to read.

An action with no authentication profile is public. A public action does not receive the token of the caller.

The auth component (`betty-blocks-types:auth/jwt`) validates the token with `allowed-to-call`.

## Build and test

Run these commands in the component directory:

- `just test`: run the unit tests.
- `just integration-test`: build the fixtures and run the webhook, auth and data-api components in a wash host.
- `just build`: build the WASM component.
