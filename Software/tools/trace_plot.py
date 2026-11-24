#!/usr/bin/env python3
"""Plot signals from a recorded trace session.

  python3 tools/trace_plot.py logs/<session> --list
  python3 tools/trace_plot.py logs/<session> moist0 pump valve0 delivered_pulses
  python3 tools/trace_plot.py logs/<session> "moist*" water_state --save watering.png

Signals are matched by name without the unit ("moist0" finds "moist0[%]");
wildcards like "valve*" pick several. Signals from different sample rates
(F/C/S files) can be mixed: each gets its own row, all on one time axis
(board time in seconds since power-on; if the board restarted during the
recording, the time keeps counting on and a red dashed line marks the restart).

Needs matplotlib (sudo apt install python3-matplotlib).
"""

import argparse
import csv
import fnmatch
import glob
import os
import sys


def load_session(folder):
    """{signal name: (unit, times_s, values)} from every CSV in the folder,
    plus the times the board restarted.

    If the board was reset during the recording its clock starts again at 0;
    later parts are shifted so time keeps going forward (one continuous axis).
    """
    signals = {}
    restarts = []
    for path in sorted(glob.glob(os.path.join(folder, "*.csv"))):
        with open(path, newline="") as f:
            reader = csv.reader(f)
            header = next(reader, None)
            if not header or "t_ms[ms]" not in header:
                continue
            t_col = header.index("t_ms[ms]")
            cols = [(i, h) for i, h in enumerate(header) if i > t_col]
            data = {h: [] for _, h in cols}
            times = []
            offset = 0.0
            for row in reader:
                if len(row) != len(header):
                    continue
                t = int(row[t_col]) / 1000.0 + offset
                if times and t < times[-1]:  # board restarted
                    offset = times[-1]
                    t = int(row[t_col]) / 1000.0 + offset
                    if not any(abs(r - offset) < 2 for r in restarts):
                        restarts.append(offset)
                times.append(t)
                for i, h in cols:
                    data[h].append(int(row[i]))
        rate = os.path.basename(path).split(".")[0]
        for _, h in cols:
            name, _, unit = h.partition("[")
            key = name if name not in signals else f"{name}@{rate}"
            signals[key] = (unit.rstrip("]"), times, data[h])
    return signals, restarts


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("session", help="session folder made by trace_record.py")
    p.add_argument("signals", nargs="*", help="signal names, wildcards allowed")
    p.add_argument("--list", action="store_true", help="list the recorded signals")
    p.add_argument("--save", metavar="PNG", help="save to a file instead of opening a window")
    args = p.parse_args()

    signals, restarts = load_session(args.session)
    if not signals:
        sys.exit(f"No trace CSVs in {args.session}")

    if args.list or not args.signals:
        for name, (unit, times, _) in signals.items():
            print(f"  {name:22} [{unit}]  {len(times)} samples")
        if not args.list:
            print("\nGive signal names to plot, e.g.: moist0 pump valve0")
        return

    chosen = []
    for pattern in args.signals:
        hits = [n for n in signals if fnmatch.fnmatch(n, pattern)]
        if not hits:
            sys.exit(f"No signal matches {pattern!r} (see --list)")
        chosen += [h for h in hits if h not in chosen]

    try:
        import matplotlib
        if args.save:
            matplotlib.use("Agg")
        import matplotlib.pyplot as plt
    except ImportError:
        sys.exit("matplotlib is missing: sudo apt install python3-matplotlib")

    fig, axes = plt.subplots(len(chosen), 1, sharex=True, squeeze=False,
                             figsize=(12, 1.6 + 1.4 * len(chosen)))
    for ax, name in zip(axes[:, 0], chosen):
        unit, times, values = signals[name]
        # On/off and state signals as steps, so they show exactly when they changed
        step = unit in ("bool", "enum", "idx", "count")
        ax.plot(times, values, drawstyle="steps-post" if step else "default", linewidth=1)
        ax.set_ylabel(f"{name}\n[{unit}]", rotation=0, ha="right", va="center", fontsize=9)
        ax.grid(True, alpha=0.3)
        for r in restarts:
            ax.axvline(r, color="red", linestyle="--", linewidth=1)
        if unit == "bool":
            ax.set_ylim(-0.2, 1.2)
            ax.set_yticks([0, 1])
    axes[-1, 0].set_xlabel("board time [s since power-on]"
                           + ("   (red dashes = board restarted)" if restarts else ""))
    fig.suptitle(os.path.basename(os.path.normpath(args.session)))
    fig.tight_layout()
    if args.save:
        fig.savefig(args.save, dpi=110)
        print(f"Saved {args.save}")
    else:
        plt.show()


if __name__ == "__main__":
    main()
