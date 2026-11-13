# Answer key for mystery A to D (spoilers!)

## mystery_A — two UART lines, 38400 baud, 8N1
Both lines idle high and have no clock, so each is an asynchronous serial line.
Narrowest pulse is about 26.04 µs, and 1 / 26.04 µs = 38400 baud.
Channel 0 is the host, channel 1 is the module (it answers about 6 ms later).

| Time   | Channel 0 (host) | Channel 1 (module)        |
|--------|------------------|---------------------------|
| 0.10 s | `AT\r\n`         | `OK\r\n`                  |
| 0.30 s | `AT+NAME?\r\n`   | `+NAME:HC-05\r\nOK\r\n`   |
| 0.60 s | `AT+PSWD?\r\n`   | `+PSWD:4821\r\nOK\r\n`    |

It is an HC-05 Bluetooth module in AT mode; the PIN is **4821**.

## mystery_B — Modbus RTU, 19200 baud, 8E1, channel 7
Narrowest pulse is about 52.08 µs, so 19200 baud. Back-to-back bytes are 11 bit times apart
(start + 8 data + parity + stop), which is the clue for a parity bit. With 8N1 you get frame errors;
with even parity everything is clean. The data is binary, not text: request and response share
one line, as on an RS-485 bus.

| Time    | Bytes                          | Meaning                                              |
|---------|--------------------------------|------------------------------------------------------|
| 0.200 s | `01 03 00 00 00 02 C4 0B`      | Master asks slave 1: read 2 holding registers from 0 |
| 0.212 s | `01 03 04 00 EB 01 F6 0B D1`   | Slave 1 answers: 0x00EB = 235, 0x01F6 = 502          |
| 0.700 s | `01 03 00 00 00 02 C4 0B`      | Same request again                                   |
| 0.712 s | `01 03 04 00 EE 01 EF DA 1A`   | 0x00EE = 238, 0x01EF = 495                           |

The last two bytes of each frame are the CRC-16. Read as a temperature/humidity sensor in tenths:
23.5 °C / 50.2 %RH, then 23.8 °C / 49.5 %RH. If the software has a Modbus decoder, stack it on the UART decoder.

## mystery_C — I2C, 400 kHz, SDA = channel 4, SCL = channel 5
The channel with the regular pulses is the clock (5); the other is data (4).

1. 0.12 s: write 0x68, register 0x00, repeated start, read 7 bytes: `56 34 12 04 12 11 26`.
   Address 0x68 with BCD time registers is a DS3231/DS1307 real-time clock:
   **12:34:56**, weekday 4, **12 November (20)26**.
2. 0.40 s: write 0x68, register 0x11, repeated start, read `18 40`.
   DS3231 temperature: 0x18 = 24, top two bits of 0x40 = 0.25, so **24.25 °C**.
3. 0.66 s: write 0x57, memory address 0x0000, repeated start, read 15 bytes:
   `CALIBRATED 2026` (0x57 is the EEPROM found on DS3231 modules).

## mystery_D — three things in one capture
**Channel 0: PWM.** Period 1 ms (1 kHz), high for 300 µs: **30 % duty**. Not part of any bus.

**Channels 2, 3, 5, 6: SPI, mode 3 (CPOL = 1, CPHA = 1), 1 MHz, MSB first.**
CLK = 5 (bursts of 8 pulses, idles high), CS# = 3 (low for whole frames), MOSI = 6, MISO = 2.
Mode 0 settings happen to give the same bytes, because both sample on the rising edge.

| Time    | MOSI                      | MISO                      | Meaning                                         |
|---------|---------------------------|---------------------------|-------------------------------------------------|
| 0.100 s | `8F FF`                   | `FF 33`                   | Read register 0x0F (WHO_AM_I) = 0x33: LIS3DH accelerometer |
| 0.150 s | `20 57`                   | `FF FF`                   | Write CTRL_REG1 = 0x57: 100 Hz, X/Y/Z on        |
| 0.151 s | `23 08`                   | `FF FF`                   | Write CTRL_REG4 = 0x08: high-resolution, ±2 g   |
| 0.500 s | `E8` + 6 dummies          | `FF 00 01 00 FE 80 3E`    | Read 6 bytes from 0x28 (X, Y, Z, low byte first) |

Top bit of the first MOSI byte = read, next bit = auto-increment. Values are 12-bit left-justified, 1 mg per step:
X = 0x0100 >> 4 = 16 mg, Y = 0xFE00 >> 4 = -32 mg, Z = 0x3E80 >> 4 = **1000 mg**. The board is lying flat: gravity is on Z.

**Channel 4: UART, 115200 baud, 8N1** (debug log from the same firmware):
- 0.16 s: `accel init ok\r\n`
- 0.51 s: `Z=1000mg\r\n`
