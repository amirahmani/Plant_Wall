#!/usr/bin/env python3
"""Record the plant wall signal trace from the ST-LINK USB serial port.

Saves a session folder, e.g. logs/2026-11-23_14-05-12/, containing:
  F_10ms.csv     fast inputs      (joystick, button, level switch, flow pulses)
  C_100ms.csv    control loop     (states, PID, pump/valves, moisture, faults)
  S_1000ms.csv   slow values      (clock, flow rate, lights, user settings)
  raw.log        every line as received, with PC time (replay with --replay)

Columns come from the header lines the firmware sends (every 10 s), so this
script never needs changing when signals are added to signals.rs.
Each CSV row: pc_time (Unix seconds on this PC), t_ms (board time), values...

Usage:
  python3 tools/trace_record.py                       # /dev/ttyACM0, until Ctrl+C
  python3 tools/trace_record.py --port /dev/ttyACM1
  python3 tools/trace_record.py --minutes 30          # stop on its own
  python3 tools/trace_record.py --replay logs/<session>/raw.log   # rebuild CSVs

Needs pyserial (sudo apt install python3-serial).
"""

import argparse
import csv
import datetime
import os
import sys
import time

BAUD = 230400  # must match trace_out::BAUD in firmware/src/trace_out.rs


class Recorder:
    """Turns trace lines into CSV files. Knows nothing about serial ports."""

    def __init__(self, out_dir, quiet=False):
        self.out_dir = out_dir
        self.quiet = quiet
        self.groups = {}  # tag -> dict(columns, writer, file, path, rows, last_t)
        self.waiting = {}  # tag -> data lines seen before its header
        self.bad = 0  # lines that didn't match their header
        self.restarts = 0
        self.dropped = None  # firmware's log_dropped counter, if seen

    def line(self, text, pc_time):
        text = text.strip()
        if not text:
            return
        if text.startswith("#"):
            self._header(text, pc_time)
        else:
            self._data(text, pc_time)

    def _header(self, text, pc_time):
        parts = text[1:].split(",")
        if len(parts) < 3 or len(parts[0]) != 1:
            self.bad += 1
            return
        tag, period, columns = parts[0], parts[1], parts[2:]
        group = self.groups.get(tag)
        if group and group["columns"] == columns:
            return  # the periodic repeat
        if group:
            group["file"].close()
            self._say(f"{tag}: signal list changed (new firmware?), starting a new file")
        base = f"{tag}_{period}ms"
        path = os.path.join(self.out_dir, base + ".csv")
        n = 2
        while os.path.exists(path):
            path = os.path.join(self.out_dir, f"{base}_{n}.csv")
            n += 1
        f = open(path, "w", newline="")
        writer = csv.writer(f)
        writer.writerow(["pc_time[s]"] + columns)
        self.groups[tag] = dict(columns=columns, writer=writer, file=f, path=path, rows=0, last_t=None,
                                period=int(period) if period.isdigit() else 0)
        self._say(f"{tag}: {len(columns) - 1} signals every {period} ms -> {os.path.basename(path)}")

    def _data(self, text, pc_time):
        parts = text.split(",")
        tag = parts[0]
        group = self.groups.get(tag)
        if group is None:
            self.waiting[tag] = self.waiting.get(tag, 0) + 1  # header comes within 10 s
            return
        values = parts[1:]
        if len(values) != len(group["columns"]) or not all(v.isdigit() for v in values):
            self.bad += 1  # e.g. a line cut in half when the cable was plugged in
            return
        t = int(values[0])
        fastest = min(self.groups, key=lambda k: self.groups[k]["period"])
        if group["last_t"] is not None and t < group["last_t"] and tag == fastest:
            self.restarts += 1  # board time went backwards: it was reset or re-flashed
            self._say(f"board restarted (t_ms went from {group['last_t']} to {t})")
        group["last_t"] = t
        group["writer"].writerow([f"{pc_time:.3f}"] + values)
        group["rows"] += 1
        if "log_dropped[lines]" in group["columns"]:
            self.dropped = int(values[group["columns"].index("log_dropped[lines]")])

    def flush(self):
        for g in self.groups.values():
            g["file"].flush()

    def close(self):
        for g in self.groups.values():
            g["file"].close()

    def status(self):
        rows = "  ".join(f"{tag}:{g['rows']}" for tag, g in sorted(self.groups.items()))
        if not self.groups:
            rows = "waiting for headers (sent every 10 s)..."
        extra = []
        if self.bad:
            extra.append(f"bad lines {self.bad}")
        if self.dropped:
            extra.append(f"dropped on board {self.dropped}")
        if self.restarts:
            extra.append(f"restarts {self.restarts}")
        return rows + ("   " + ", ".join(extra) if extra else "")

    def _say(self, msg):
        if not self.quiet:
            print("\r" + msg + " " * 20)


def new_session_dir(base):
    name = datetime.datetime.now().strftime("%Y-%m-%d_%H-%M-%S")
    path = os.path.join(base, name)
    os.makedirs(path, exist_ok=True)
    return path


def record_serial(args):
    try:
        import serial  # pyserial
    except ImportError:
        sys.exit("pyserial is missing: sudo apt install python3-serial")

    try:
        port = serial.Serial(args.port, args.baud, timeout=0.5)
    except serial.SerialException as e:
        msg = f"Can't open {args.port}: {e}"
        if "ermission" in str(e):
            msg += ("\nYour user needs to be in the 'dialout' group:"
                    "\n  sudo usermod -aG dialout $USER     (then log out and back in)")
        elif "No such file" in str(e):
            msg += "\nIs the Nucleo plugged in? Check `ls /dev/ttyACM*`."
        sys.exit(msg)

    out = new_session_dir(args.out)
    print(f"Recording {args.port} @ {args.baud} baud into {out}/   (Ctrl+C to stop)")
    rec = Recorder(out)
    raw = open(os.path.join(out, "raw.log"), "w")
    stop_at = time.time() + args.minutes * 60 if args.minutes else None
    last_status = 0.0
    port.reset_input_buffer()  # start on fresh data, not what queued up before
    pending = b""
    try:
        while stop_at is None or time.time() < stop_at:
            chunk = port.read(port.in_waiting or 1)
            now = time.time()
            if chunk:
                pending += chunk
                *lines, pending = pending.split(b"\n")
                for b in lines:
                    text = b.decode("ascii", errors="replace").strip()
                    if text:
                        raw.write(f"{now:.3f}\t{text}\n")
                        rec.line(text, now)
            if now - last_status >= 1.0:
                last_status = now
                rec.flush()
                raw.flush()
                print("\r" + rec.status(), end="", flush=True)
    except KeyboardInterrupt:
        pass
    finally:
        port.close()
        raw.close()
        rec.close()
    print("\n" + rec.status())
    print(f"Saved in {out}/")


def replay(args):
    out = new_session_dir(args.out)
    rec = Recorder(out)
    with open(args.replay) as f:
        for raw in f:
            pc_time, _, text = raw.rstrip("\n").rpartition("\t")
            rec.line(text, float(pc_time) if pc_time else 0.0)
    rec.close()
    print(rec.status())
    print(f"Saved in {out}/")


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--port", default="/dev/ttyACM0", help="serial port (default /dev/ttyACM0)")
    p.add_argument("--baud", type=int, default=BAUD, help=f"default {BAUD}")
    p.add_argument("--out", default="logs", help="folder for session folders (default ./logs)")
    p.add_argument("--minutes", type=float, help="stop after this many minutes")
    p.add_argument("--replay", metavar="RAW_LOG", help="rebuild CSVs from a raw.log instead of recording")
    args = p.parse_args()
    if args.replay:
        replay(args)
    else:
        record_serial(args)


if __name__ == "__main__":
    main()
