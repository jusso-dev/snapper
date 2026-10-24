# Uploads

Add a target in Settings, on the Uploads tab. The config file stores the endpoint, bucket, and access key id. It does not store the secret access key, an Imgur token, or an `Authorization` header. Those go in the system keychain under the service name `snapper`.

## S3 and R2

An S3 target needs an endpoint, bucket, region, access key id, and a public base URL if you want a CDN link. Without one, the link is the object's path-style URL on the endpoint. The secret is read from the keychain account `s3:<id>`. Snapper signs a presigned PUT with `rusty-s3` for five minutes. The secret is not placed in the URL.

Cloudflare R2 uses the same target. Set the region to `auto` and the endpoint to the R2 S3 URL.

## Imgur

An Imgur target needs the application's Client-ID. Snapper posts to `https://api.imgur.com/3/image` with `Client-ID`. That identifier is not the same as a user token.

## Custom HTTP

A custom target sends a multipart form. The URL of the uploaded file is read from the JSON response with a pointer such as `/data/url`. Leave the pointer empty when the response body is the URL. A `keyring:` header value may only name that target's own `http:<id>:` keychain accounts, so the id of a custom target cannot be changed in Settings. Redirects are not followed. New targets get an id that no other target uses.

A ShareX `.sxcu` file can be imported from the Uploads tab. `$json:data.url$` and `{json:data.url}` become `/data/url`, and an empty `URL` or `{response}` uses the whole response body as the URL. A `URL` template that mixes text with a JSON value, `Arguments`, `Parameters`, and non-multipart bodies are rejected on import. A `DeletionURL` template like that is dropped, and the target has no delete link. A header that looks like a secret, including `Authorization`, `Cookie`, and API key headers, is stored as `keyring:http:<id>:<header>` instead of the raw value. Nothing is written to the keychain if the import fails.

## History

Each successful upload appends one JSON line to `history.jsonl` in the data directory: time, file, and URL.

## Plugins

External programs are described in `docs/plugins.md`.
