# Fixtures

Test data only; no code. What each folder is for is in doc 10.

## `captures/`

Signal Detective's generated sample captures (`1_UART_easy`, `2_UART_mystery_baud`, `3_I2C_bus`,
`4_SPI_wrong_mode`, `mystery_A` to `mystery_D`) and their answer keys (`ANSWERS_spoilers.md`,
`ANSWERS_mystery_spoilers.md`), copied unchanged from `../signal-detective/samples/`. Signal
Detective's tests (`../signal-detective/tests/`) hold the detailed expectations.

`captures/local/` holds FNIRSI's example captures (`I2C.demo`, `SPI.demo`, `UART.demo`, shipped with
DLA_Logic). They are not ours to publish: the folder is git-ignored, and their tests skip with a
visible message when the files are absent.

## `formats/`

Small files for the `.sr` and VCD importer tests. All content is ours; none is FNIRSI's, sigrok's or
anyone else's capture.

The `.sr` files were made from raw packed samples with known content, turned into `.sr` with
`sigrok-cli` 0.7.2 (run as a separate program, which the clean-room rule allows); the first VCD is
sigrok-cli's VCD output of one of them. The generator scripts are not part of this repository; to
rebuild a file, generate raw samples with the content below and convert them the same way.

| File | Content |
|---|---|
| `sr/mixed-8ch-1mhz.sr` | 8 channels at 1 MHz, 30,000 samples. Channel 0 `TX`: UART 115200 8N1 "Hello, sigrok!\r\n" from sample 1000. Channel 1 `PWM`: 1 kHz, 25 %, from sample 100. Channel 2 `HIGH`: always high. Channels 3 to 7: low |
| `sr/walk-16ch.sr` | 16 channels at 1 kHz, 64 samples: a walking one (unit size 2) |
| `sr/walk-24ch.sr` | 24 channels at 2.5 MHz, 48 samples: a walking one (unit size 3) |
| `sr/selected-3of8.sr` | 8-bit walking one at 1 kHz with only channels 0, 2 and 5 captured, named A, B, C |
| `sr/counter-multifile.sr` | 8 channels at 10 MHz, 9,000,000 samples of a counter stepping every 4,096 samples; split into three data files |
| `vcd/sigrok-mixed-8ch.vcd` | `sigrok-cli -O vcd` of `sr/mixed-8ch-1mhz.sr` |
| `vcd/features.vcd` | Written by hand: nested scopes, a name used in two scopes, a shared identifier, a multi-character identifier, a 4-bit vector (with a short value that VCD extends on the left), x and z, a real variable, a comment between value changes, `$timescale 10 ns` with changes every 20 ns. The expected channels and edges are worked out in the test |

Malformed and truncated files are made inside the tests from these and from small hand-made
archives, so every error message is checked.

## `usb/`

USB transcripts for driver replay tests, one folder per driver (doc 10). Empty until spike S3.
