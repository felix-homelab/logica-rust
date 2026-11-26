# logica-vla: the virtual logic analyser

A small separate program that behaves like a streaming logic analyser: Logica finds it, connects,
and captures from it live, the same way it will capture from a real streaming device (M3). It
generates configurable signals with random payloads and pauses: UART, I2C, SPI, PWM, clocks and
random noise. It exists so live capture and live decoding can be built, tried and tested without
hardware.

## Run it

```
cargo run -p logica-vla                          # the built-in mix on eleven channels, real time
cargo run -p logica-vla -- --config tools/virtual-analyser/examples/busy-sensors.json
cargo run -p logica-vla -- --seed 42             # the same signals in every capture
cargo run -p logica-vla -- --print-config        # the built-in configuration as a starting point
cargo run -p logica-vla -- --listen 127.0.0.1:7172 --name Second   # a second one beside it
```

It prints what it generates and listens on `127.0.0.1:7171`. In Logica, "Capture live…" lists it
and captures from it, alone or together with others. Logica looks at `127.0.0.1:7171` to `7174`,
so up to four can run side by side with `--listen`; if one listens elsewhere, start Logica (or
`logica mcp`) with `LOGICA_VLA=host:port`, several separated by commas.

## Configure it

A JSON file; everything but `signals` has a default:

| Field | Default | Meaning |
|---|---|---|
| `name` | `Bench` | Shown as "Virtual analyser: Bench" (also `--name`) |
| `listen` | `127.0.0.1:7171` | Address to listen on |
| `channelCount` | 8 | Channels 0 to n−1; those no signal uses stay low |
| `sampleRatesHz` | 1, 4, 10, 25, 100 MS/s | Rates offered; a rate too slow for a signal is left out |
| `seed` | none | The same seed gives the same signals; without one, each capture differs |
| `speed` | 1 | 1 is real time, 0 as fast as Logica takes the data |
| `tileMs` | 20 | Signal time per batch sent |
| `clockPpm` | 0 | How far its sample clock is off, in ppm (positive runs fast); shows in `shared` signals |

Signals (`kind`), each on its own channels:

| Kind | Fields (defaults) |
|---|---|
| `uart` | `channel`, `name` (TX), `baud` (115200), `dataBits` (8), `parity` (none, even, odd), `stopBits` (1), `invert`, `messages` (random printable text when empty), `randomBytes` ([4, 16]), `everyMs` ([20, 200]) |
| `i2c` | `scl`, `sda`, `clockHz` (100 kHz), `addresses` ([0x50]), `bytes` ([1, 4]), `readChance` (0.3), `nackChance` (0.05), `everyMs` |
| `spi` | `clk`, `cs`, `mosi`, `miso` (at least one of the two), `clockHz` (500 kHz), `mode` (0), `msbFirst` (true), `bytes` ([1, 4]), `everyMs` |
| `pwm` | `channel`, `name`, `frequencyHz`, `duty` (0.5) |
| `clock` | `channel`, `name`, `frequencyHz` |
| `noise` | `channel`, `name`, `meanUs` (200): random changes about that far apart |
| `shared` | `channel`, `name` (SYNC), `frequencyHz` (100): a clock on the computer's time, so analysers on one computer see the same edges, each at its own samples |

`everyMs` is the pause between bursts, picked at random between the two numbers. Mistakes are
explained before anything listens (`--check` only checks). Examples are in `examples/`.

## How it works

Each signal schedules its next burst a random pause after the last one and draws it with Logica's
own signal generators (`crates/protocols/src/synth.rs`), into the channel's lane; batches of
`tileMs` are sent as level changes, paced to real time. Each signal has its own random stream, so
a seed gives the same data however the batches fall. The wire protocol is in `PROTOCOL.md`; the
driver on Logica's side is `crates/drivers/src/vla`. One capture runs at a time, like on a real
device; listing does not disturb a running capture.

Because the signals come from the same generators the decoder tests use, this program checks the
live path (streaming, the store while it grows, live decoding, the window), not the decoders'
reading of imperfect signals: its timing is perfect.

The built-in configuration has a `shared` clock, SYNC on CH10. Two analysers (the second with
`--listen 127.0.0.1:7172 --name Second`) captured "All into one capture" in Logica can be lined
up by it; give one a configuration with `clockPpm` to see its drift corrected.
