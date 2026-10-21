# Changelog

All notable changes to RustyCorn are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [0.1.0] - 2026-10-21

First public release.

### Added

- Live logging of ÄKTAprime and ÄKTAprime plus runs over RS-232: UV,
  conductivity, pH, %B, pressure, flow, run state, valve positions, fractions
  and log messages, one CSV file per run, flushed after every row.
- Automatic reconnection when the cable is pulled or the instrument restarts.
- Live plots in stacked panels or one overlay plot, against time, volume or
  column volumes; zoom, pan, inspector, marks and a filterable run log.
- Flow-path schematic and reading cards, live or at the inspected moment.
- PC method store: the instrument can save methods to the PC and run stored
  methods from it (`PrimeMeth_NNN.txt`); replaced and deleted methods are kept
  in `old/`.
- Method editor (also standalone with `--methods`): name, view, write and
  validate methods.
- An example method (His-tag purification on a 1 ml HisTrap column, with
  its run setup) written into a new, empty methods folder.
- Run information: a run setup (column, column volume, buffers, notes) per PC
  method, recorded when the instrument runs a matching method; sample, user
  and notes asked for at each run start; run information can be added to
  finished logs.
- Analyze Run: open saved logs in their own tabs or a read-only viewer
  (`rustycorn run.csv`), and export the plot as PDF.
- Demo mode with simulated data.
- Settings page (serial port, volume unit 1 or 10 µL, data and methods
  folders, raw data capture, theme, refresh rate, panels). A settings file
  that cannot be read cleanly is kept as `settings.toml.broken-…` and
  reported, never silently overwritten.
- Windows bundle (a single `RustyCorn.exe` with README and licences) and a
  Linux desktop entry and installer.

[0.1.0]: https://github.com/CalvinKlein96/RustyCorn/releases/tag/v0.1.0
