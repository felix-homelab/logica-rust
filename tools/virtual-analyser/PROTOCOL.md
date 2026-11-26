# The virtual analyser's wire protocol, version 1

Logica's own design, for `logica-vla` and the `vla` driver (`crates/drivers/src/vla/wire.rs`, the
one implementation both use). TCP; numbers little-endian. Logica connects out to the analyser as a
client; nothing connects into Logica (ADR 0006 is about ways into the engine and is unaffected).

## Greeting

As soon as a client connects, the analyser sends one JSON line:

```
{"protocol":"logica-vla","version":1,"model":"Bench","serial":"vla-7171",
 "channels":[{"index":0,"name":"TX"},…],"sampleRatesHz":[4000000.0,…]}
```

A client that only lists devices closes the connection here. Lines are at most 64 KiB.

## Requests

One JSON line each:

- `{"start":{"channels":[0,3],"sampleRateHz":4000000.0,"samples":240000000}}`: capture the
  channels (ascending, each once) at one of the offered rates for `samples` samples per channel.
- `{"stop":true}`: end the capture early. Closing the connection does the same.

## Messages while capturing

Each message is a kind byte, a `u32` payload length (at most 16 MiB) and the payload:

| Kind | Name | Payload |
|---|---|---|
| 1 | begin | one byte per enabled channel, in slot order: its level at sample 0 (0 or 1) |
| 2 | edges | `u16` slot, then `u64` samples where that channel changes level, ascending, after its earlier edges |
| 3 | advance | `u64` end: every enabled channel is known up to this sample (exclusive) |
| 4 | note | UTF-8 text worth telling the person (the seed, for example) |
| 5 | end | `u64` samples: the capture is over; the analyser closes the connection |
| 6 | error | UTF-8 text: the request was refused or the capture failed; the analyser closes the connection |

A capture is `begin`, then batches of `edges` and one `advance` each, then `end`. A refused start
(an unknown rate, a missing channel, the analyser busy with another capture) is a single `error`
with the reason, written for the person.
