# Screenmate local RPC v1

The application binds an ephemeral port exclusively on `127.0.0.1` and creates `%LOCALAPPDATA%\Screenmate\session.json` containing `{ "port": 12345, "token": "..." }`. The token is regenerated on every application start. Restrict the session file to the current Windows user and delete it at shutdown. Never log the token or image contents.

Requests use `POST /rpc`, `Authorization: Bearer <token>`, and a JSON body `{ "method": "look", "params": {} }`. Responses are `{ "ok": true, "result": ... }` or `{ "ok": false, "error": "..." }`. A bearer token is a per-launch shared secret, not a per-request one-time code. It must be validated on every request. All errors must fail closed: do not return a frame while capture is paused, locked, or privacy protection cannot be applied.

- `look`: `params` may include string `monitor` and `region: { x, y, width, height }` in physical pixels relative to the selected monitor. Result: `{ image_base64, mime_type: "image/jpeg", timestamp, width, height, changed }`. `changed` compares this frame with the preceding look; the first frame is changed.
- `list_windows`: `{ monitors: [{ id, name, x, y, width, height, primary }], windows: [{ id, title, process, x, y, width, height }] }`.
- `inbox`: `{ messages: [{ text, timestamp }] }`; returned unread messages are marked read atomically.
- `say`: accepts `{ text }`. The application currently returns an error because the in-app Codex conversation has been removed; it must not claim a message was displayed.

`look_diff` and `watch` are implemented by the MCP process using `look`; no extra application RPC methods are needed. RPC errors are converted to MCP tool errors. No image is written to disk by the MCP server.

## Optional login daemon

`screenmate-mcp.exe --daemon` starts one local daemon per Windows user/session. It has no GUI or hotkey and never captures pixels itself. It creates an ephemeral listener exclusively on `127.0.0.1` and writes `%LOCALAPPDATA%\Screenmate\mcp\daemon.json` with `{ "version": 1, "port": ..., "token": "...", "pid": ... }`. The 32-byte random token is independent of the application RPC token and is regenerated on each daemon start. Before writing it, the daemon atomically replaces the directory DACL with a protected current-user-only DACL; newly created metadata inherits that restriction. A clean shutdown removes the matching metadata. Never log the file, token, image data, or IPC payloads.

Codex continues to launch `screenmate-mcp.exe` over stdio. When a daemon is available, this process authenticates to its loopback socket and forwards newline-delimited MCP JSON-RPC. The daemon bounds connections, request sizes and in-flight requests. The stdio process drains outstanding replies after stdin EOF. If daemon metadata is absent, stale, or authentication/connection fails before a session starts, it uses the original direct stdio implementation. A daemon failure during a connected session terminates that bridge; the client must reconnect. The daemon never auto-spawns from this fallback, and disabling login startup does not terminate an already running daemon. An explicit `screenmate-mcp.exe --stop` authenticates to the daemon and requests a clean stop; inaccessible or stale metadata is reported as an error rather than claiming success.

The daemon is a local transport implementation, not an unauthenticated MCP-over-HTTP endpoint. In either mode an actual screenshot still requires the separately running application and its own session token, and mask/pause failures remain closed. Same-user or administrator processes can read the daemon token, as with the application token.

