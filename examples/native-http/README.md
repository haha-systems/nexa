# Offline HTTP/1.1 exchange in Nexa

This example encodes a GET request and decodes a response from caller-provided byte chunks. Each codec is Nexa source and seals as its own Core1 memory. The exchange runs without a network or transport binding.

| Source | Export | Imports |
| --- | --- | --- |
| `decimal.nexa` | Checked ASCII decimal scan | |
| `headers.nexa` | Header field parsing and framing classification | `decimal` |
| `request.nexa` | GET request encoding | `decimal` |
| `response.nexa` | Incremental response decoding | `headers` |
| `exchange.nexa` | Request encoding and fixture-driven response decoding | `request`, `response` |

The encoder accepts a dotted-decimal IPv4 address with an optional port from 1 to 65535. Octets and port use canonical decimal spelling. The target is an ASCII origin-form path with an optional query, up to 4,096 bytes, with valid percent escapes. The encoder preserves the supplied authority and target bytes and emits `Host` and `Connection: close` fields.

The decoder supports fixed-length HTTP/1.1 responses, including 4xx and 5xx statuses, and bodyless 204 and 304 responses. A 205 requires a length of zero. It consumes up to eight interim heads before the final response. A head can contain up to 64 fields within 8,192 bytes, and a fixed-length body can contain up to 65,536 bytes. Repeated ordinary fields retain their order and original name spelling. Field values are bytes with leading and trailing SP and HTAB removed. The body and bytes beyond its boundary retain their original values. Duplicate `Content-Length`, transfer codings, protocol switching, and close-delimited bodies produce typed failures.

The response decoder expects `State.start()` on the first call and the `needs_more.next` value it returned on the previous call thereafter. The head state retains CRLF scan progress across chunks. The body state retains the parsed fields, collected bytes, and remaining byte count. Each state holds bounded buffers. Appending to them copies the current prefix and consumes the Core1 execution budget. A call can signal EOF independently of its chunk. The fixture harness signals EOF after feeding the supplied chunks and includes any subsequent fixture bytes in the returned remainder.

## Run the example

From the repository root in Bash, build Nexa and seal the example with its exact imports.

```bash
cargo build --locked
nexa=./target/debug/nexa
example=examples/native-http

seal_exchange() {
  local decimal_digest headers_digest request_digest response_digest
  decimal_digest=$("$nexa" core1 seal "$example/$1") || return
  headers_digest=$("$nexa" core1 seal "$example/headers.nexa" --import "decimal=$decimal_digest") || return
  request_digest=$("$nexa" core1 seal "$example/request.nexa" --import "decimal=$decimal_digest") || return
  response_digest=$("$nexa" core1 seal "$example/response.nexa" --import "headers=$headers_digest") || return
  "$nexa" core1 seal "$example/exchange.nexa" --import "request=$request_digest" --import "response=$response_digest"
}

exchange_digest=$(seal_exchange decimal.nexa)
"$nexa" core1 inspect "$exchange_digest" --closure
"$nexa" core1 run "$exchange_digest" --input-file "$example/input.json"
```

The input in `input.json` contains a fragmented 404 with a six-byte binary body and trailing bytes. The result uses lowercase hexadecimal for `Bytes` and a decimal string for the `Int` status. `fixtures.json` records additional inputs and their expected values in this wire format.

`decimal-alternate.nexa` implements the same scan contract with a different overflow check. Reseal the import chain with the substitute and compare the exchange results using the same source and input.

```bash
alternate_digest=$(seal_exchange decimal-alternate.nexa)
test "$exchange_digest" != "$alternate_digest"
diff <("$nexa" core1 run "$exchange_digest" --input-file "$example/input.json") \
     <("$nexa" core1 run "$alternate_digest" --input-file "$example/input.json")
```
