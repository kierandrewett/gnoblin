# Compositor bridge examples

Use `gnoblinctl` for a one-off action. Use the bridge socket when a shell needs
live snapshots, previews or a shortcut session. Its transport is one UTF-8
JSON object per line on a persistent Unix socket. The first line from the
server is a `hello` event. [The bridge reference](compositor-bridge.md)
describes the other operations and limits.

The examples below run **inside a Gnoblin session**. They use
`GNOBLIN_COMPOSITOR_SOCKET` when set, or
`$XDG_RUNTIME_DIR/gnoblin/compositor-v1.sock` otherwise. They send a request
ID and wait for the matching reply; unrelated events may arrive first.

## Shell script: focus a window

For occasional commands, let `gnoblinctl` handle the socket protocol. This
example requires `jq` and acts only when exactly one matching window exists:

```sh
ids=$(gnoblinctl window list --app-id org.example.Editor.desktop --json |
    jq -r '.windows[].id')
count=$(printf '%s\n' "$ids" | sed '/^$/d' | wc -l)
if [ "$count" -eq 1 ]; then
    gnoblinctl window focus "$ids"
fi
```

Replace the app ID with the exact value from `gnoblinctl window list --json`.
IDs expire when their windows close. If you need repeated updates, subscribe to
snapshots instead of polling the command.

## Python: watch windows

The [Python watcher](compositor-bridge.md#example-watch-the-window-list)
subscribes with `{"op":"windows"}` and prints each full snapshot. Keep its
connection open for as long as your dock or switcher needs updates. An empty
`windows` array means no eligible windows, not a request failure.

## Node.js: request a snapshot

Save this as `windows.mjs` and run `node windows.mjs`:

```javascript
import net from "node:net";
import path from "node:path";
import readline from "node:readline";

const socketPath = process.env.GNOBLIN_COMPOSITOR_SOCKET ??
    path.join(process.env.XDG_RUNTIME_DIR, "gnoblin/compositor-v1.sock");
const connection = net.createConnection(socketPath);
const lines = readline.createInterface({ input: connection });
connection.setTimeout(5000);
connection.on("connect", () => {
    connection.write(JSON.stringify({ op: "command", id: "list-1", command: "windows" }) + "\n");
});
connection.on("timeout", () => connection.destroy(new Error("Gnoblin did not reply")));
connection.on("error", (error) => {
    console.error(error.message);
    process.exitCode = 1;
});
lines.on("line", (line) => {
    const message = JSON.parse(line);
    if (message.id !== "list-1") return; // includes the initial hello event
    if (message.event === "error") {
        console.error(message.message);
        process.exitCode = 1;
    } else if (message.event === "reply") {
        console.log(JSON.stringify(message.result.windows, null, 2));
    } else {
        return;
    }
    connection.end();
});
```

`command` requests return one `reply` envelope. For live changes, send
`{"op":"windows"}` on the same connection and handle every `windows` event.
Each event replaces the prior array; it is not a delta.

## Go: request a snapshot

This version uses only Go's standard library. Save it as `windows.go` and run
`go run windows.go`:

```go
package main

import (
    "bufio"
    "encoding/json"
    "fmt"
    "log"
    "net"
    "os"
    "path/filepath"
    "time"
)

func main() {
    socket := os.Getenv("GNOBLIN_COMPOSITOR_SOCKET")
    if socket == "" {
        runtime := os.Getenv("XDG_RUNTIME_DIR")
        if runtime == "" { log.Fatal("XDG_RUNTIME_DIR is unset") }
        socket = filepath.Join(runtime, "gnoblin", "compositor-v1.sock")
    }
    conn, err := net.Dial("unix", socket)
    if err != nil { log.Fatal(err) }
    defer conn.Close()
    if err := conn.SetDeadline(time.Now().Add(5 * time.Second)); err != nil { log.Fatal(err) }
    if _, err := fmt.Fprintln(conn, `{"op":"command","id":"list-1","command":"windows"}`); err != nil {
        log.Fatal(err)
    }

    scanner := bufio.NewScanner(conn)
    scanner.Buffer(make([]byte, 4096), 4*1024*1024)
    for scanner.Scan() {
        var message struct {
            Event   string          `json:"event"`
            ID      string          `json:"id"`
            Result  json.RawMessage `json:"result"`
            Message string          `json:"message"`
        }
        if err := json.Unmarshal(scanner.Bytes(), &message); err != nil { log.Fatal(err) }
        if message.ID != "list-1" { continue }
        if message.Event == "error" { log.Fatal(message.Message) }
        if message.Event == "reply" {
            var pretty []byte
            pretty, err = json.MarshalIndent(message.Result, "", "  ")
            if err != nil { log.Fatal(err) }
            fmt.Println(string(pretty))
            return
        }
    }
    if err := scanner.Err(); err != nil { log.Fatal(err) }
    log.Fatal("connection closed before reply")
}
```

The reply contains a `windows` object, so Go prints the full result object.
The ID lets a client match replies when it has several requests in flight.

## Quickshell and other UI processes

A Qt or Quickshell shell can keep a Unix socket open and parse newline-delimited
JSON using its normal socket and JSON APIs. Register shortcuts on that same
connection, render a popup on `activated`, and send `end` when it closes. If
the connection drops, reconnect, request a fresh `windows` snapshot and
register bindings again: bindings belong to the connection that created them.

For an example of a Quickshell integration, Bingux is a **separate project**
that uses Gnoblin's bridge. The [shell integration guide](shell-integration.md)
describes the contract without depending on Bingux's implementation.
