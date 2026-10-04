# API reference

> Agent endpoints, the cog's signal export, report fields, config keys, flags and the store vector.

Replace `<seed>` with `169.254.42.1` (USB), or the Seed's LAN or tailnet address.

## Agent API (port 80)

Plain HTTP on port 80, with `Access-Control-Allow-Origin: *`. These worked without a token on our Seed. If yours refuses writes, add `Authorization: Bearer <pairing token>`. Port 8443 is self-signed TLS and browsers reject it.

| Method and path | Does |
|---|---|
| `GET /api/v1/apps` | Installed apps, with `running` |
| `GET /api/v1/apps/sen0213-ecg/config` | Current config |
| `PUT /api/v1/apps/sen0213-ecg/config` | **Replaces the whole config and restarts the cog.** Send every key. |
| `POST /api/v1/apps/sen0213-ecg/start` | Start in continuous mode |
| `POST /api/v1/apps/sen0213-ecg/stop` | Stop |
| `POST /api/v1/apps/sen0213-ecg/console` | Run one allowed command: `{"command":"--once --simulate"}` |
| `GET /api/v1/apps/sen0213-ecg/logs` | The cog's stdout, one report per line |

## Cog export (port 8046)

Read-only, the last 60 s, GET only, `Access-Control-Allow-Origin: *`. It binds `0.0.0.0` by default and starts before the ADC is found.

| Route | Returns |
|---|---|
| `/` | Plain-text index |
| `/status` | The latest report (JSON). `{"status":"starting"}` before the first one. |
| `/raw?seconds=N` | JSON samples and R-peak times. N up to 60; default 10. |
| `/raw.csv?seconds=N` | CSV: `t_ms,raw_v,filtered_mv,r_peak` |
| `/guide` | This guide as JSON, served by the cog for companion apps |

An abridged `/status` while running (values illustrative):

```json
{"status":"ok","source":"ads1115@/dev/i2c-1:0x48/A0","sample_rate_hz":250,"window_s":5,
 "lead_state":"ok","heart_rate_bpm":71.9,"beats":6,"quality":0.97,
 "sdnn_ms":22.1,"rmssd_ms":18.4,"late_samples":0,"read_errors":0,"medical":false}
```

And a shortened `/raw?seconds=1`:

```json
{"source":"ads1115@/dev/i2c-1:0x48/A0","sample_rate_hz":250,
 "columns":["t_ms","raw_v","filtered_mv"],
 "samples":[[1790000000000,1.64875,0.012],[1790000000004,1.64881,0.015]],
 "r_peaks_ms":[1790000000320]}
```

## Report fields

| Field | Meaning |
|---|---|
| `status` | `ok`, `no_beats`, `leads_off`, `flat`, `no_source` (with `error`), or `no_samples` (nothing read yet) |
| `source`, `sample_rate_hz`, `window_s` | Where the data came from, rate, and report window |
| `lead_state` | The electrode state behind `status` |
| `heart_rate_bpm` | 60000 / median RR over the last 10 s. `null` unless `ok`. |
| `beats`, `r_peaks_ms`, `rr_ms` | Beats, R-peak times (Unix ms) and RR intervals in this window |
| `sdnn_ms`, `rmssd_ms`, `hrv_window_s` | Variability over up to 60 s; need 3 or more intervals |
| `quality` | 0-1. Zero unless leads are on and beats found; lowered by RR irregularity |
| `raw` | `min_v`, `max_v`, `mean_v`, `std_v` of the raw signal |
| `samples`, `late_samples`, `max_late_ms`, `read_errors` | Sampling health |
| `samples_raw_v`, `samples_filtered_mv` | Waveforms, with `--emit-samples` only; at most 3000 each |
| `export`, `medical`, `timestamp` | Export URL; always `false`; Unix seconds |

## Config keys

| Key | Default | Range | Meaning |
|---|---|---|---|
| `interval` | 5 | 1-60 s | Seconds between reports |
| `window` | 10 | 3-12 s | Capture length of a `--once` run |
| `sample_rate` | 250 | 100-500 Hz | ECG samples per second |
| `i2c_bus` | 1 | 0-9 | `/dev/i2c-N`; 1 is the header bus |
| `i2c_addr` | 72 | 72-75 | 72=0x48, 73=0x49, 74=0x4A, 75=0x4B |
| `channel` | 0 | 0-3 | ADS1115 input (0 = `A0`) |
| `mains_hz` | 60 | 0-60 | Notch: 60, 50, or 0 for none |
| `api_bind` | `0.0.0.0:8046` | host:port | Export address. `127.0.0.1:8046` keeps it on the Seed |
| `emit_samples` | false | bool | Add waveforms to each report |
| `simulate` | false | bool | Use a synthetic 72 bpm ECG |

## CLI flags

`--once`, `--interval`, `--window`, `--sample-rate`, `--i2c-bus`, `--i2c-addr` (72-75 or `0x48`), `--channel`, `--mains-hz`, `--api-bind`, `--emit-samples`, `--simulate`, `--help`. Each config key maps to the flag of the same name with dashes.

## Console

Only these commands are accepted. The run is capped at 15 s and 64 KiB of output.

`--once`, `--once --emit-samples`, `--once --simulate`, `--once --simulate --emit-samples`, `--help`

## Store vector

Each report is sent by the cog to the agent, loopback only: `POST 127.0.0.1:80/api/v1/store/ingest` with `{"vectors":[[21,[8 floats]]],"dedup":true}`. Each float is clamped to 0-1:

| # | Value |
|---|---|
| 0 | `hr / 200` |
| 1 | `quality` |
| 2 | leads OK (1 or 0) |
| 3 | `sdnn / 200` |
| 4 | `rmssd / 200` |
| 5 | `beats / 20` |
| 6 | median RR `/ 2000` |
| 7 | max raw volts `/ 3.3` |

## RuView reference CSV

`timestamp_ms,value`: beat-to-beat bpm, RuView ADR-293's reference-series format. It grades `wifi-densepose-vitals` heart rate as MEASURED. The app writes it from **Record**.

## Signal flow

```diagram
flow
```
