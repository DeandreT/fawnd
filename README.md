# fawnd

A userspace driver and CLI for the **DrunkDeer A75** Hall-effect (magnetic
switch) keyboard. Lets you configure per-key actuation points,
rapid trigger / turbo (snap-tap), and lighting from the command line or a TOML
profile.

## What it talks to

| | |
|---|---|
| USB vendor | `0x352D` (DrunkDeer) |
| Product (A75 ANSI) | `0x2383` (also 0x2382/2384/2386 for siblings) |
| Config interface | HID usage page `0xFF00`, usage `0x00` (vendor, **not** the boot-keyboard interface) |
| Report | 64 bytes: `[0x04 report-id][63-byte payload]` |

On Linux you need read/write access to the `hidraw` node. Either run as root or
install a udev rule:

```
# /etc/udev/rules.d/99-drunkdeer.rules
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="352d", MODE="0660", TAG+="uaccess"
```

## Protocol summary

All commands are sent as the 63-byte payload after the report ID. Inbound
reports use the same framing.

| Command | Byte | Notes |
|---|---|---|
| Identity | `0xA0` | `[A0 02]`; reply carries model sig (bytes 3..6), firmware (LE u16 at 6..8), turbo (14), rapid-trigger (15) |
| LED mode | `0xAE` | `[AE 01 turbo dir seq speed brightness rgb]` |
| Modify key | `0xB6` | `[B6 sub 00 row <keys…>]`; sub: `01`=actuation, `04`=downstroke, `05`=upstroke, `03`=key-tracking toggle |
| Rapid trigger | `0xB5` | `[B5 00 1E 01 00 00 01 turbo rt]` |
| Key tracking | `0xB7` | inbound live key-depth stream (see below) |

**Live key depth.** Send `0xB6 03 01` to request one round; the keyboard replies
with three `0xB7` packets (`[B7 00 00 row <values…>]`, `row` = 0/1/2 at payload
byte 3, depth values from byte 4 — 59/59/8 keys). Each value is current travel in
~0.1 mm units (0 = released, ~40 = bottomed out). Re-send the request to keep the
stream flowing. Verified against hardware.

**Key addressing.** Keys are a flat array of 126 slots in visual reading order
(6 matrix rows × 21 columns; gaps are empty). The modify-key packet slices this
into three *protocol rows*: row 0 = slots 0..59, row 1 = 59..118, row 2 =
118..126 (only 8 keys). See `src/protocol/layout.rs`.

**Actuation encoding.** One byte = `mm × 10`. 2.0 mm = `0x14` (default). Range
0.2 mm (`0x02`) – 3.8 mm (`0x26`).

**Read-back caveat.** The device echoes per-key settings but does not let you
*query* them, so the controller keeps the desired state in memory as the source
of truth.

## GUI

```sh
cargo run --bin fawnd-gui
```

An [egui](https://github.com/emilk/egui)/eframe app: a visual key grid (click to
select keys, colour = actuation depth) with side-panel controls for global and
per-key actuation, rapid trigger / turbo, lighting, and profile load/save. All
device I/O runs on a background worker thread so the UI never blocks.

## CLI

```sh
fawnd info                      # model / firmware / toggle state
fawnd actuation 1.5             # set all keys to 1.5 mm
fawnd actuation 1.2 W A S D     # set specific keys
fawnd rapid-trigger on --turbo  # enable rapid trigger + snap-tap
fawnd apply profile.toml        # apply a saved profile
fawnd reset                     # restore defaults
```

## Profile format

```toml
actuation = 1.5        # global default, mm
rapid_trigger = true
turbo = false

[keys]                 # per-key overrides, mm
W = 1.2
A = 1.2
S = 1.2
D = 1.2
```

## Architecture

```
src/
├── protocol/        wire format — no I/O
│   ├── consts.rs    VID/PID, report framing, command bytes, enums
│   ├── layout.rs    126-slot key map + name<->index
│   ├── packet.rs    payload builders + mm<->byte codec
│   └── mod.rs       Model / Identity parsing
├── device.rs        HID discovery + raw report read/write + handshake
├── controller.rs    high-level per-key + global config, state mirror
├── config.rs        TOML profiles (load/save/apply)
├── error.rs         error type
├── gui/             egui UI
│   ├── worker.rs    background device thread (Command/Event channels)
│   ├── app.rs       egui App: key grid + side-panel controls
│   └── mod.rs       window setup / run()
├── lib.rs           library root
├── main.rs          clap CLI         (bin: fawnd)
└── bin/fawnd-gui.rs GUI entry point  (bin: fawnd-gui)
```

### Planned: daemon & IPC

Today the GUI opens the keyboard directly. To support dynamic behavior (per-app
switching, hotkeys, hotplug) without two processes fighting over the device's
request/response stream, ownership moves behind a daemon:

```
                 ┌────────────┐   Unix socket    ┌──────────────┐
   GUI / CLI ───▶│ IPC client │ ───────────────▶ │ fawnd-daemon │──▶ HID device
                 └────────────┘  (commands /      │  - profiles  │
                                  state / depth)   │  - rules     │
                                                   │  - watchers  │
                                                   └──────────────┘
                                                          ▲
                          focus watcher (KWin/sway/X11) ──┘
                          hotkeys · hotplug (udev/HID)
```

- `daemon`: device owner, profile store, rule engine, IPC server.
- `ipc`: shared request/response types + client used by CLI and GUI.
- `watch`: pluggable focus/hotkey/hotplug sources that emit profile-switch events.

The existing `Controller` is reused unchanged as the daemon's device layer.

## Roadmap

- [x] Write path verified on hardware (actuation, rapid trigger, lighting, reset)
- [x] Live key-depth visualization (`0xB7` stream decode) — toggle in the GUI
- [ ] Rapid-trigger down/up curve in the profile format
- [ ] Per-key RGB (custom lighting) — `0xAE` custom mode, 13 keys/packet

### Daemon & automation

The keyboard stores its config in firmware (settings persist across unplug), so a
daemon is **not** needed to keep a profile applied — it exists for *dynamic*
behavior. See [Planned: daemon & IPC](#planned-daemon--ipc).

- [ ] `fawnd-daemon`: background process that owns the HID device; CLI/GUI become
      IPC clients over a Unix socket (single owner avoids contention on the
      request/response stream)
- [ ] Per-app auto profile switching — push a profile based on the focused window
      (KWin via D-Bus on Wayland; sway/Hyprland via their IPC; X11 via
      `_NET_ACTIVE_WINDOW`)
- [ ] Global hotkey profile cycling
- [ ] Apply-on-hotplug — re-assert the active profile when the keyboard reconnects
      (wake, dock, KVM switch)
- [ ] IPC API exposing the live key-depth stream + current state to other tools
