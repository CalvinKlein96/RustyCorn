# Changelog

All notable changes to RustyCorn are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [0.2.0] - 2026-11-13

### Added

- Every run information field (column, column volume, buffers, notes, as well
  as sample and user) can be given when a run starts and changed while it
  runs (Run info card: Edit). A method started from the instrument's own
  memory has no run setup, so its column volume can now be set during the
  run; **Plot vs column volumes** uses it at once.
- Fraction delay volume setting (`fraction_delay_ml`, default 580 µL):
  what passed the UV cell in the last delay volume before a run ends is still
  in the tubing when the pump stops and reaches no tube, so the last
  fractions are cut there, live and in opened logs; a tube that would only
  start after that point stays empty and is not drawn.
- **Fractions** switch in the run page toolbar: fraction shading and numbers
  on or off, on screen and in the PDF.

### Changed

- Log files start with the date: `<date>_<time>_<prefix>.csv` (was
  `<prefix>_<date>_<time>.csv`), so they sort by time whatever their prefix;
  the raw captures, method dumps and saved PDFs follow
  (`<date>_<time>_<prefix>_figure_<n>.pdf`). Existing files keep their names.

### Fixed

- The run setup (column, column volume, buffers) of a PC method was never
  recorded when the ÄKTAprime ran it: the instrument reports a loaded method
  as "PC Method" with its number where the file has "-", so the method never
  matched its file. These two settings say where a method is kept and are no
  longer compared; a "PC Method" N is PC method N.
- The injection valve at the start of a method was shown as Inject although
  the method started in Load: it was read from the run state message, whose
  byte an ÄKTAprime sets in every state, whatever the valve does. The valve
  now comes only from the method ("Method start: … inject Load") and the
  "Injection Valve …" messages, live and in opened logs (also in logs written
  before). Run state events are logged as "Run state: Run", without the
  misleading "inject=IV_INJECT".
- An opened log showed its fractions as one fraction without numbers: the log
  recorded only "Tubefeed event (10 bytes)", not the tube number the message
  carries. Tube feeds are now logged as "Feed Tube Nr N". Tube 0, which an
  ÄKTAprime sends when fractionation stops, no longer counts as a new tube,
  so the tubes after a restart are numbered as in the rack. In logs written
  before, the tubes are counted when the log is opened (the first feed of a
  run selects tube 1); the feed just before "Fraction Off" is taken as the
  stop's tube 0, as the instrument sends it.
- The run's end closes a fraction still being collected; it no longer runs on
  over data recorded after the run.
- "Fraction Off" in a method stops fractionation (as "Fraction size 0.0 ml"
  does), live, in opened logs, in the inspector and as FRAC STOP on the plot.
- The overlay plot shows the fraction numbers along its bottom edge, on screen
  and in the PDF export; before, only the %B panel showed them.
- A crash ("index out of bounds" in the run view) when the event display was
  changed to show fewer events: the event labels were laid out for the old
  set. After a change of x units the labels could also stay at their old
  positions.

## [0.1.0] - 2026-10-22

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
  folders, raw data capture, theme, refresh rate, panels, graphics backend).
  A settings file that cannot be read cleanly is kept as
  `settings.toml.broken-…` and reported, never silently overwritten.
- Graphics backend choice (`gpu_backend` in the settings file, or
  `--gpu auto|gl|dx12|vulkan|metal`, which is remembered). OpenGL is the
  default on Windows, where probing every graphics interface crashed in some
  graphics drivers.
- Windows bundle (a single `RustyCorn.exe` with README and licences) that
  keeps its settings, `crash.log` and, by default, its data in its own folder;
  a Linux desktop entry and installer.

[0.1.0]: https://github.com/CalvinKlein96/RustyCorn/releases/tag/v0.1.0
