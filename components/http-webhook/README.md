# Http-webhook component

The http-webhook component runs an action of an app when it receives an HTTP request. One workload runs for each app. A config map gives the action for each webhook name.

## HTTP contract

- Endpoint: `GET` or `POST /api/webhook/{webhook-name}`
- The public URL is `https://<app-identifier>.<zone>.betty.app/api/webhook/{webhook-name}`. The gateway sends the request to the webhook workload of the app.
- Body of a `POST`: optional. Use a JSON object with the input variables of the action. An empty body gives `{}`.
- A `GET` has no body. The action gets the input `{}`.
- All webhooks are public. The component does not authenticate the caller, and it does not send a token to the action. The action must validate the request itself.

The response is the output of the action as JSON, with status `200`.

Errors have the body `{"error": "<message>"}`:

| Status | Cause |
| ------ | ----- |
| `400` | The body is not a JSON object. |
| `404` | The path is not `/api/webhook/{webhook-name}`, or the app has no webhook with this name. |
| `405` | The method is not `GET` or `POST`. |
| `413` | The body is larger than 10 MB. |
| `500` | The configuration of the workload is not correct. |
| `502` | The action failed. |

## Configuration

The component reads these values:

- `webhooks` (wasi:config): a JSON object that gives the action ID for each webhook name.
- `betty_config` (wasi:config): the configurations that the action receives, as a JSON object.
- `APPLICATION_ID` (environment): the app of the actions.
- `WASMCLOUD_HOST` (environment): the gateway that runs the actions.

```json
{
  "webhooks": { "send-invoice": "4d1f...", "ping": "9a7c..." }
}
```

The value of the key is a JSON string. The example shows it as an object to make it easy to read.

## Build and test

Run these commands in the component directory:

- `just test`: run the unit tests.
- `just integration-test`: build the fixture and run the webhook component in a wash host.
- `just build`: build the WASM component.
