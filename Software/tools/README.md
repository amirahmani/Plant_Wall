# Signal trace: logging everything while the wall runs

The firmware sends every signal, with a time stamp, out of the Nucleo's USB
cable (the ST-LINK serial port). These scripts record it on the PC and plot it.
No extra wiring: the same USB cable you flash with.

| Rate   | File           | What                                                        |
|--------|----------------|-------------------------------------------------------------|
| 10 ms  | `F_10ms.csv`   | joystick X/Y + direction, button, level switch, flow pulses |
| 100 ms | `C_100ms.csv`  | HMI mode, watering state, PID dose + integral, pump, valves, LED, moisture raw + %, sensor detected/usable, faults, overruns |
| 1 s    | `S_1000ms.csv` | uptime, time of day, flow rate, total water, lights, user water/light settings |

The full list, with units and the meaning of every `enum` code, is in
[`plant_logic/src/signals.rs`](../plant_logic/src/signals.rs).

## One-time setup

```bash
sudo apt install python3-serial python3-matplotlib
sudo usermod -aG dialout $USER     # allows opening /dev/ttyACM0; log out and back in after
```

## Record

Flash the firmware (`cargo run --release`), then, with the board still plugged in:

```bash
cd Software
python3 tools/trace_record.py                 # until Ctrl+C
python3 tools/trace_record.py --minutes 60    # or for a set time
```

It prints a live count of lines received and makes a new folder per recording,
e.g. `logs/2026-11-23_14-05-12/`, with the three CSVs and a `raw.log`.
The CSVs open directly in LibreOffice Calc or any spreadsheet.

- Columns appear within 10 s of starting (that's how often the board repeats them).
- Each row has `pc_time` (this PC's clock) and `t_ms` (the board's, since power-on).
- `bad lines` in the status = lines that didn't match the columns. A couple when
  you plug in is normal; more means the baud rate is wrong.
- `dropped on board` = lines the board had to skip. Should stay 0.
- `board restarted` = the board was reset or re-flashed while recording.

`probe-rs run` (what `cargo run` uses) doesn't use the serial port, so you can
flash and record at the same time.

## Plot

```bash
python3 tools/trace_plot.py logs/<session> --list                  # what's in it
python3 tools/trace_plot.py logs/<session> moist0 water_state pump valve0 delivered_pulses
python3 tools/trace_plot.py logs/<session> "moist*" "valve*" --save watering.png
```

Names without the unit; `*` matches several. Each signal gets its own row on a
shared time axis, and on/off and state signals are drawn as steps.

Useful sets:
- **Watering / PID tuning:** `water_state water_pod dose_ml pid_i_ml delivered_pulses moist0 flow`
- **Sensor check:** `"raw*" "detected*" "usable*"`
- **Timing health:** `overruns log_dropped`

## Adding a signal

1. Add one line to the right group in `plant_logic/src/signals.rs`, e.g.
   `tank_temp: u16, "0.1C";`
2. `cargo build` then fails until you fill it in where that group is built in
   `firmware/src/main.rs` (search for `ControlTrace {` etc.). This is on purpose:
   a signal can't be listed but never logged.
3. `cargo test-logic` checks the line still fits and its header matches.
4. Nothing to change in these scripts.

## Rebuild CSVs from a raw log

```bash
python3 tools/trace_record.py --replay logs/<session>/raw.log
```

## Wire format

Plain text at 230400 baud, so you can also watch it in any serial terminal
(`screen /dev/ttyACM0 230400`):

```
#F,10,t_ms[ms],joy_x[raw],joy_y[raw],joy_dir[enum],button[bool],low_water_raw[bool],flow_pulses[pulses]
F,123450,2051,2047,0,0,0,1234
```

`#` lines are headers: tag, period in ms, then `name[unit]` per column. Data lines
are the tag, `t_ms`, then the values: all integers, bool = 0/1, 255 in an
`idx` column = none. Details in `plant_logic/src/trace.rs`.
