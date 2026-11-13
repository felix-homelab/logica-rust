# Answer key (spoilers!)

## 1_UART_easy — 115200 8N1, channel 0
Switch the decoder display format from Dec to ASCII.
- 0.080 s: `AT+LED=ON\r\n`
- 0.100 s: `OK\r\n`
- 0.450 s: `TEMP=23.5C\r\n`
- 0.800 s: `KEY: SWORDFISH\r\n`

## 2_UART_mystery_baud — 57600 8N1
The preset decoder (115200) shows garbage and frame errors.
Measure the narrowest pulse with two cursors: about 17.36 µs, and 1 / 17.36 µs = 57600 baud.
- 0.200 s: `BAUD RATE DETECTIVE\r\n`
- 0.600 s: `CODE=7391\r\n`

## 3_I2C_bus — 100 kHz, SCL = ch 1, SDA = ch 2
- 0.10 s: write to address 0x27, **NACK** (nobody home), stop.
- 0.25 s: write to 0x50 (EEPROM style): memory address 0x0010, then data
  `4F 50 45 4E 20 53 45 53 41 4D 45` = `OPEN SESAME`.
- 0.50 s: write 0x50 + address 0x0010, **repeated start**, read 0x50: the same 11 bytes come back.
  The master NACKs the last byte. That is normal: it means "I'm done reading".
- 0.75 s: write 0x48, register 0x00, repeated start, read 0x48: `19 80`.
  LM75/TMP102-style temperature: 0x19 = 25, top bit of 0x80 = +0.5, so **25.5 °C**.

## 4_SPI_wrong_mode — 1 MHz, mode 1 (CPOL = 0, CPHA = 1)
CLK = ch 3, MISO = ch 4, CS# = ch 5, MOSI = ch 6.
The preset decoder is mode 0 (CPHA = 0) and shows garbage (4F 80 00 00 / 00 77 A0 0C ...).
Zoom in on one clock pulse: the data lines change just after the rising edge and are stable
at the falling edge, so the falling edge is the sampling edge. With clock idle low that is CPHA = 1.

| Time     | MOSI                     | MISO                         | Meaning                    |
|----------|--------------------------|------------------------------|----------------------------|
| 0.1500 s | 9F 00 00 00              | 00 EF 40 18                  | Read JEDEC ID              |
| 0.4000 s | 06                       | 00                           | Write enable               |
| 0.4002 s | 05 00                    | 00 02                        | Read status: WEL bit set   |
| 0.7000 s | 03 00 10 00 + 16 dummies | 4 dummies + `MODE 1 UNLOCKED!` | Read 16 bytes at 0x001000 |

(The command set is flash-style, but a real SPI flash would use mode 0 or 3. I used mode 1 on purpose.)
