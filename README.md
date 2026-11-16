# Soltop

A Task Manager style system monitor for Linux, written in Rust with [egui](https://github.com/emilk/egui).

Soltop shows what is running and how hard the hardware is working: per-process CPU, memory, disk and
GPU usage, and live graphs for the CPU, memory, disks, network interfaces and GPUs.

## Features

**Processes**
- Sortable table with PID, name, user, CPU, RAM, disk I/O and GPU load per process
- Processes that share an executable (browser tabs, language servers) fold into one expandable row
- Hover a process for its full path, command line, parent PID, status, thread count and run time
- Per-process GPU load, which most Linux monitors don't show (NVIDIA only, see below)

**Performance**
- A device list with a mini graph per device, and a detail view with a large graph and tiles
- **CPU**: model, load, per-core graphs, clock speed, base/max clock, cache sizes, temperature, up time
- **Memory**: in use, available, cached, swap, temperature when a sensor exists
- **Disks**: model, type (NVMe/SSD/HDD), read/write speed, active time, response time, temperature, usage per volume
- **Network**: Ethernet and Wi-Fi interfaces with send/receive speed, link speed, status, IP and MAC addresses, Wi-Fi signal
- **GPU**: name, load, memory, temperature, power, fan, clocks, encoder/decoder load, PCIe link, driver version

**System**
- Host name, OS, kernel, architecture, up time, load average, and battery state on laptops

## Requirements

- Linux
- Rust with edition 2024 support (Rust 1.85 or newer)
- A desktop session (Wayland or X11) and the usual graphics libraries
- Optional: an NVIDIA GPU with its proprietary driver, for GPU data

### Nix / NixOS

The repository has a flake with a dev shell that provides everything above:

```bash
nix develop
```

If you use [direnv](https://direnv.net/), `.envrc` loads the shell automatically.

## Build and run

```bash
cargo run --release
```

## Tests

```bash
cargo test --workspace
```

Two tests run the real scanner against the machine they are on (a scan smoke test and a test of
the scan thread), so they need a normal Linux environment.

## Limitations

- **GPU data is NVIDIA only.** It is read through NVML. Without the NVIDIA driver Soltop still
  works and shows a note where the GPU data would be. AMD and Intel GPUs are not supported yet.
- **Per-process disk I/O** is only readable for your own processes unless Soltop runs as root.
  Other users' processes show 0 B/s.
- **Disks and network interfaces are detected at startup.** Hardware plugged in later is not picked up.
- **Virtual network interfaces** (docker, bridges, VPNs) are hidden on purpose.
- **Memory speed and type** are not shown, because reading them needs root.
- **Battery support** is covered by unit tests but has not been tried on many machines.

## Project layout

| Path | What it is |
|---|---|
| `src/` | The egui application: window, tabs and the scan thread |
| `crates/hardware_scanner/` | Reads CPU, memory, GPU, disk, network, battery and process data and exposes it as plain data types |

The scanner runs on its own thread and sends results to the UI once per second. Facts that never
change (CPU model, disk models, ...) are collected once and sent with the first message.
