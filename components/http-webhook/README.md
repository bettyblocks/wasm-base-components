# Http-webhook component

The http-webhook component runs an action of an app when it receives an HTTP request. One workload runs for each app. A config map gives the action for each webhook name.

## HTTP contract

- Endpoint: `GET` or `POST /api/webhook/{webhook-name}`
- The public URL is `https://<app-identifier>.<zone>.betty.app/api/webhook/{webhook-name}`. The gateway sends the request to the webhook workload of the app.
- Body of a `POST`: optional, of any format. When the body is a JSON object, each field goes to the input variable with the same name.
- A `GET` has no body. The component does not read the body of a `GET`.
- All webhooks are public. The component does not authenticate the caller, and it does not send a token to the action. The action must validate the request itself.

### The request in the input

The component also adds the input `__request`. It contains the request as the component received it:

```json
{
  "amount": 12,
  "__request": {
    "method": "POST",
    "headers": { "content-type": "application/json", "x-slack-signature": "v0=a2114d57..." },
    "body": "{\"amount\": 12}"
  }
}
```

- `method`: `GET` or `POST`.
- `headers`: an object with one property for each header. The names are lowercase. The component joins the values of a repeated header with `, `.
- `body`: the body as text, without a change. Use this text to check a signature, because a signature is calculated on the exact text.

To use the request, add an object input variable with the name `__request`. Make a schema model for it with the headers that the sender of the webhook uses, for example `x-slack-signature`. A body field with the name `__request` does not go to the action.

The response is the output of the action as JSON, with status `200`.

Errors have the body `{"error": "<message>"}`:

| Status | Cause |
| ------ | ----- |
| `400` | The body is not valid UTF-8 text. |
| `404` | The path is not `/api/webhook/{webhook-name}`, or the app has no webhook with this name. |
| `405` | The method is not `GET` or `POST`. |
| `413` | The body is larger than 5 MB. The action gets the body two times, and http-wrapper reads a maximum of 16 MB. |
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
