## CLI exit codes

`0` success · `1` any error (validation rejection, missing env var, failed
verification, refused resume, failed health probe…). Errors print as
`error: <chain>` on **stderr** — context kept (`verify out/audit.json
against …: HMAC verification FAILED: …`), secrets never included. stdout
carries only command data: tracing logs are written to stderr, so
`harvest … > file` captures clean JSON even on failing runs.

Pipe semantics are classic unix: a closed consumer (`| head`) kills the
process by SIGPIPE (shell status 141), never a broken-pipe panic — pinned
by `tests/cli.rs`.
