# Upload and action plugins

A plugin is a program named `snapper-plugin-<id>`. Snapper does not load shared libraries. It starts the program, writes one JSON object to stdin, and reads one JSON line from stdout.

Snapper looks in the data directory `plugins` folder (next to history, from `directories` with qualifier `au`, organisation `jusso-dev`, application `snapper`) and on `PATH`.

## Describe

```sh
snapper-plugin-echo --describe
```

The program prints a JSON Schema and exits. Snapper shows the `title` and `properties` in Settings. Simple string fields are enough. The example crate is `examples/plugin-echo`.

## Upload request

Snapper writes this object and closes stdin. `config` is the table stored on the upload target. It is not a secret store. Put tokens in the plugin's own storage.

```json
{"version":1,"kind":"upload","file":"/tmp/x.png","mime":"image/png","config":{}}
```

`version` is `1`. `kind` is `upload` for a file upload. `file` is a path the plugin can read. Snapper deletes that temporary file after the plugin exits.

## Response

One line of JSON:

```json
{"ok":true,"url":"https://cdn.example/shot.png"}
```

or

```json
{"ok":false,"error":"plugin refused"}
```

Anything that is not one JSON object is an error. Snapper waits 60 seconds, then kills the process. The local screenshot is kept when an upload fails.

`examples/plugin-echo` returns a `file://` URL of the path it was given. Its `config.mode` can be `bad` (not JSON), `sleep` (pause so the timeout fires), or `fail` (`ok` is false).

## Settings

Add the plugin from the Uploads tab, or write an `external` target:

```toml
[[uploads]]
kind = "external"
id = "echo"
command = "snapper-plugin-echo"
```

`command` can be empty. Snapper then uses `snapper-plugin-<id>` from the plugins directory or `PATH`.
