# Optional UART telemetry

The OUT pin is all you need. This page is only for the **optional, best-effort** serial telemetry,
which adds the module's internal amplitudes to each window. It is **off by default**.

Enable it by pointing `--uart` at the module's serial device:

```
cog-ld1040c-motion --interval 1 --gpio 17 --uart /dev/serial0 --baud 9600
```

When a frame parses, the report gains:

| Field | Meaning |
|---|---|
| `motion_amplitude` | mid-frequency AD average — higher with a moving target, lower when empty |
| `signal` | signal energy (SUM2 / 64) |
| `noise` | noise floor (SUM0 / 64) |

Any field that does not parse is reported as **null** — the cog never fabricates a value.

## ⚠️ Baud rate is a guess

The LD1040C datasheet does **not** document the UART baud rate. The cog defaults to **9600 8N1**,
which is unverified. If `--uart` produces no telemetry (fields stay null) while the OUT line works,
try other common rates with `--baud`: 115200, 57600, 38400, 19200.

## Frame format (for reference)

All multi-byte fields are **big-endian**:

```
0x3C 0x3A | LEN | command | work_pattern | version | radar_threshold[3] |
light_threshold | output_delay[2] | module_id[4] | output_mode | sensitivity_ad |
mid_freq_ad | noise_sum0[2] | signal_sum2[2] | 0x3A 0x3E
```

The parser locates a frame by its `0x3C 0x3A` header and `0x3A 0x3E` footer and reads the fields by
offset. Offsets are inferred from the datasheet field order and should be confirmed against a real
capture.
