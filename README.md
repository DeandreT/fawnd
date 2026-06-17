# fawnd

A userspace driver for the **DrunkDeer A75** Hall-effect (magnetic switch)
keyboard, with a CLI, an egui GUI, and a background daemon. Configure per-key
actuation points, rapid trigger / turbo (snap-tap), and lighting; watch live key
depth; and switch profiles automatically based on the focused window.

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
| Identity | `0xA0` | `[A0 02]`; reply carries model sig (bytes 4..7), firmware (LE u16 at 7..9), turbo (15), rapid-trigger (16) |
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
per-key actuation, rapid trigger / turbo, lighting, and profile load/save, plus a
live key-depth view.

The GUI is a **daemon client** — it talks to `fawnd-daemon` over the socket (on a
background worker thread, so the UI never blocks) rather than opening the device
itself. Start `fawnd-daemon` first; if it isn't running the GUI shows "offline"
with a Reconnect button.

## CLI

```sh
fawnd info                      # model / firmware / toggle state
fawnd actuation 1.5             # set all keys to 1.5 mm
fawnd actuation 1.2 W A S D     # set specific keys
fawnd rapid-trigger on --turbo  # enable rapid trigger + snap-tap
fawnd apply profile.toml        # apply a saved profile
fawnd reset                     # restore defaults
```

These commands open the device directly. If `fawnd-daemon` is running, use the
`fawnd daemon …` subcommands (below) instead, so the two don't contend for the
device.

## Daemon & auto profile switching

The daemon owns the keyboard and serves clients over a Unix socket; it also
switches profiles automatically based on the focused window.

```sh
fawnd-daemon &                  # owns the device, serves the socket
fawnd daemon status             # model / firmware / active profile
fawnd daemon profiles           # list profiles in the store
fawnd daemon apply gaming       # apply a stored profile
```

Profiles live in `~/.config/fawnd/profiles/<name>.toml`. Auto-switching is
enabled by creating `~/.config/fawnd/rules.toml` (see
[`rules.example.toml`](rules.example.toml)):

```toml
default = "typing"              # used when no rule matches

[[rule]]
match = "steam_app_*"          # glob on the window app-id; first match wins
profile = "gaming"
```

On KDE/KWin (Wayland) there's no portable way to read the focused window's
app-id, so the daemon loads a small KWin script that reports each window
activation to a D-Bus service (`org.fawnd.Focus`); the daemon matches the app-id
to a rule and applies the profile. Without `rules.toml`, auto-switching is simply
off.

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
│   ├── worker.rs    background IPC client thread (Command/Event channels)
│   ├── app.rs       egui App: key grid + side-panel controls
│   └── mod.rs       window setup / run()
├── ipc.rs           daemon/client wire protocol (JSON over a Unix socket)
├── daemon.rs        device-owning thread + job channel + IPC server
├── rules.rs         app-id → profile rules (rules.toml)
├── watch.rs         KWin focus watcher (zbus D-Bus service + KWin script)
├── lib.rs           library root
├── main.rs          clap CLI            (bin: fawnd)
├── bin/fawnd-gui.rs    GUI entry point   (bin: fawnd-gui)
└── bin/fawnd-daemon.rs daemon entry      (bin: fawnd-daemon)
```

### Daemon & IPC

The daemon is the sole owner of the keyboard; the CLI and GUI are IPC clients.
A single owner avoids two processes fighting over the device's request/response
stream.

```
                 ┌────────────┐   Unix socket    ┌──────────────┐
   GUI / CLI ───▶│ IPC client │ ───────────────▶ │ fawnd-daemon │──▶ HID device
                 └────────────┘  (commands /      │  - profiles  │
                                  state / depth)   │  - rules     │
                                                   │  - watcher   │
                                                   └──────────────┘
                                                          ▲
                              focus watcher (KWin) ───────┘
```

- `daemon`: a device-owning thread processes jobs from a channel; per-connection
  handlers and the watcher are producers, so device access stays serialized.
- `ipc`: shared request/response types + the `Client` used by CLI and GUI.
- `watch`: focus watcher (KWin via D-Bus today; sway/Hyprland/X11 planned).
  Hotkey and hotplug sources are planned.

## Roadmap

- [x] Write path verified on hardware (actuation, rapid trigger, lighting, reset)
- [x] Live key-depth visualization (`0xB7` stream decode) — toggle in the GUI
- [ ] Rapid-trigger down/up curve in the profile format
- [ ] Per-key RGB (custom lighting) — `0xAE` custom mode, 13 keys/packet

### Daemon & automation

The keyboard stores its config in firmware (settings persist across unplug), so a
daemon is **not** needed to keep a profile applied — it exists for *dynamic*
behavior. See [Daemon & IPC](#daemon--ipc).

- [x] `fawnd-daemon`: owns the HID device with a Unix socket + profile store;
      the CLI (`fawnd daemon …`) and the GUI are both IPC clients.
- [x] Per-app auto profile switching — focused-window → profile rules
      (`rules.toml`), via a KWin script + D-Bus on KDE Wayland. sway/Hyprland and
      X11 backends still pending.
- [ ] Global hotkey profile cycling
- [ ] Apply-on-hotplug — re-assert the active profile when the keyboard reconnects
      (wake, dock, KVM switch)
- [ ] IPC API exposing the live key-depth stream + current state to other tools

## License

[MIT](LICENSE) © Deandre
