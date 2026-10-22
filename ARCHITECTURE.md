# RustyCorn architecture

Notes for developers: how the code is organised, how data flow between the
instrument thread and the GUI, which file formats must not change, and how to
test and look at the program safely.

The instrument: an ÄKTAprime (firmware V2.01) or ÄKTAprime plus on RS-232 at
9600 baud, usually through an FTDI USB adaptor (`/dev/ttyUSB0` on Linux,
`COMn` on Windows).

## Crates

A Cargo workspace with one crate per area. Dependencies run from the bottom
up: core ← methods ← protocol; core ← plotdata ← export; theme ← panels ←
runview; everything ← app.

| Crate | What it does |
|---|---|
| `app` (binary `rustycorn`) | Command line, console banner and closing report, the workspace window (tabs, top bar, settings page, method editor tab, opened runs), the run-info forms, the read-only viewer, crash log and Ctrl-C handling, the Windows resources (`build.rs`). |
| `crates/rc-core` | The shared data model: `AppState` and `SharedApp`, instrument state, trend points, events and fractions; the CSV log writer and reader; run information; settings; decoders of instrument log texts; OPC constants; the demo data source. |
| `crates/rc-methods` | Method text: parsing method dumps, reading/writing/validating `PrimeMeth_NNN.txt`, the PC method store (save, load, list, delete, names, run setups, `old/`), matching a dump to a stored method, and the example method written into a new methods folder. |
| `crates/rc-protocol` | The serial link layer (frames, checksums, sequence numbers), OPC payload encoding/decoding, the handlers that apply messages to `AppState`, and `Acquisition`, which starts and stops the link or demo thread. |
| `crates/rc-plotdata` | The plot model shared by the screen and the PDF: x scales (time, volume, column volumes), downsampling, number and tick formatting, event labels, the instrument state at a moment of a run. |
| `crates/rc-export` | The PDF export: the figure is drawn as SVG and converted with svg2pdf, with the embedded fonts. |
| `crates/rc-theme` | Colours, fonts and small widgets (cards, chips, tabs, buttons). Trace colours live in the palette. |
| `crates/rc-panels` | The run page's side cards (run, readings, run info), the flow-path schematic, the run log list and the status bar. |
| `crates/rc-runview` | The run page: stacked and overlay plots, toolbar, time slider, inspector, marks, the export button. |
| `crates/rc-methodeditor` | The method editor page: method list, settings and breakpoint table, validation, run setup card. |

Other files: `assets/` (logo, icons, screenshots, fonts and their licences),
`packaging/linux/` (desktop entry and per-user installer),
`packaging/windows/` (the zip bundle script and its `README.txt`),
`RustyCorn.sh` (build and start from the source folder).

## Threading model

The whole run state is one `rc_core::AppState` behind
`Arc<parking_lot::Mutex<_>>`, called `SharedApp`.

- **The link thread** (or the demo thread) owns the serial port. It polls the
  instrument every 100 ms; the instrument resets the link when the PC stays
  silent for a few seconds, so this thread never waits for the GUI. It locks
  `AppState` briefly to apply each decoded message and never holds the lock
  while talking to the port. Method store requests (save, load, list, …) are
  answered on this thread too: the instrument gives up after about 2 s.
- **The GUI thread** locks `AppState` once per frame, copies out or computes
  what it draws, and unlocks. The lock is never held across slow work such as
  file dialogs or PDF rendering (the PDF is rendered on a worker thread).
- **Generation counter.** Every mutation ends with `AppState::touch()`, which
  bumps `AppState::generation`. GUI code caches derived data (downsampled
  traces, labels, the state at a moment) keyed by the generation and rebuilds
  it only when the number changed. While live, the GUI repaints at the
  display refresh rate from Settings (`request_repaint_after`).
- Panics on the link thread are caught and logged; the link goes on. A panic
  on the GUI thread is written to `crash.log` in the settings folder.

Shared values that both threads need also live in `AppState`: the raw
capture file (`raw`), the methods folder (`method_dir`, read by the link
thread for every method request, so changing it needs no restart), the
`methods_changed` counter, the method last loaded from the PC
(`last_loaded_method`), the pending "Method start" event, the volume unit and
the connection status.

## Run information

Run information is recorded in the log as `EVT_INFO` events with the text
`Key: value`. The fields (`rc_core::runinfo::RUN_INFO_FIELDS`) come from two
places:

- **Method setup** (Column, Column volume (mL), Buffer B, Port 1–8, Method
  notes) belongs to a PC method. It is stored next to the method as
  `<methods>/PrimeMeth_NNN_setup.txt` (rc-methods `method_setup` /
  `set_method_setup`), edited in the method editor's Run setup card, and moved
  to `old/` together with its method.
- **Per-run fields** (Sample, User, Notes) are asked for in a non-modal window
  when a run starts (`AppState::run_serial` counts runs) and recorded with
  `log_run_info(…, only_changed = true)`. Settings keep only `last_user`, to
  prefill the next run.

**Matching the dump to a stored method.** At run start the instrument sends
the method it is about to run as a dump (OPC 0x2007 … 0x2008). The dump is
compared with the stored methods by content (rc-methods
`run_setup_for_dump`): settings and breakpoint values after normalisation, so
padding, line endings and number spellings ("1" / "1.0") do not matter. Of
several matches, the method last loaded from the PC wins, else the lowest
number. A match sets `AppState::run_info_for_new_log` to that method's setup
plus `PC method: NNN – name`; `AppState::start_run` records it in the new
log and clears it. No match (or no dump): no setup, so a run is never given
another method's setup.

"Method notes" (`runinfo::METHOD_NOTES_KEY`) is distinct from the per-run
"Notes"; both may span several lines.

**Adding run information later.** An opened log can be given run
information afterwards: new `EVT_INFO` rows are appended to the end of the
CSV (`AppState::append_run_info`). Readers take the latest value of a field,
and nothing earlier in the file is rewritten.

## File formats

These formats are shared with other programs and with older files; they must
stay compatible in both directions. The reference tests (below) guard them.

### CSV logs

Written by `rc_core::csvlog`, flushed after every row (one write per row, so
a crash never leaves half a row).

```text
# Akta PRIME data log started 2026-10-20T15:52:04.754
wall_time,cum_time_ms,cum_vol_mL,type,uv_AU,cond_mScm,ph,pctb_pct,pres_MPa,temp_degC,message
2026-10-20T15:52:06.120,2000.0,0.016667,TREND,0.02000000,10.0000,7.00,5.00,0.4000,21.50,
2026-10-20T15:52:06.130,2000.0,0.016667,EVT_LOG,,,,,,,"Flow 0.5 ml/min"
```

- The first line is `# Akta PRIME data log started ` and the start time.
- `wall_time` is `yyyy-mm-ddTHH:MM:SS[.sss]`: local time, milliseconds only
  when non-zero.
- `cum_time_ms` has 1 decimal, `cum_vol_mL` 6. TREND rows: uv 8, cond 4,
  pH 2, %B 2, pressure 4, temperature 2 decimals, empty message. Numbers are
  rounded like C `printf("%.Nf")`; infinities are written `Inf` / `-Inf`.
- Event rows (`EVT_CONNECT`, `EVT_DISCONNECT`, `EVT_RUN_STATE`, `EVT_INJECT`,
  `EVT_TUBEFEED`, `EVT_LOG`, `EVT_STEP`, `EVT_MARK`, `EVT_INFO`) leave the
  value columns empty and put the text in double quotes, with commas turned
  into semicolons and line breaks into spaces; quotes inside are not escaped.
- Readers skip the comment line, the header and blank lines, accept `\n` or
  `\r\n`, and split each row into at most 11 fields.
- Files are named `<prefix>_<yyyymmdd_HHMMSS>.csv`; a second start within
  the same second gets `_2`, `_3`, … rather than overwriting a log. Side files
  replace `.csv`: `_raw.txt` (raw capture), `_method_N.met` (method dumps).
  A log that never received data is deleted on close.

### Method files `PrimeMeth_NNN.txt`

One file per stored method, NNN = 001–999, in the methods folder; the same
layout the instrument's dump format and PrimeView use. Plain ASCII with CRLF
line endings: a fixed header, a `Method Settings` block of
`Name<padded to 19>,value` lines, and a `Method Breakpoints` table whose rows
start with `,` and pad each value to its column width. `MethodFile::read` and
`MethodFile::format_text` read and write it; `MethodFile::problems` lists
what the instrument would reject. A method the instrument overwrites or
deletes is first moved to `old/PrimeMeth_NNN_<yyyymmdd-HHMMSS>.txt`.

On the wire, method text is escaped (`zy` → `#`, `z<c>` → c, `~` → CR, `}` →
CR LF, `|<n>` → n spaces, `{<n>` → n `*`) and sent in chunks of 34
characters; a load aborts on a line longer than 99 characters. See the module
docs of `rc_methods::store` for the message sequence.

### `names.txt`

Names given to methods in the method editor; never sent to the instrument.
UTF-8, a `#` comment header, then one `NNN<TAB>name` line per method. Readers
take only lines matching `^[0-9]{1,3}\t.+$`; everything else is ignored.

### `PrimeMeth_NNN_setup.txt`

The run setup of method NNN, next to it in the methods folder. A `#` comment
header (and, when the notes span lines, a second comment line explaining the
escapes), then one `Key: value` line per field set, in the order of
`METHOD_SETUP_FIELDS`. Only the value of Method notes is escaped: `\n` is a
line break and `\\` a backslash, so every field stays on one line and files
without notes read the same. Unknown keys and empty values are ignored.

### Settings

`settings.toml` in `rc_core::settings::config_dir()`: `$RUSTYCORN_CONFIG_DIR`,
else the folder of the program file (`portable_dir`: always on Windows,
elsewhere only when a `settings.toml` is already there; not when it cannot be
written to), else `<config>/RustyCorn`. In the program folder, an empty data
folder setting also means that folder (`default_data_dir`, otherwise the
current folder), and the first start copies an earlier
`<config>/RustyCorn/settings.toml` in (`adopt_user_settings`). Written via a
temporary file and a rename. `gpu_backend` (also `--gpu`) picks the wgpu
backends in `app/src/main.rs` (`native_options`); `auto` leaves wgpu's
default, which probes every backend and honours `WGPU_BACKEND`.
Missing fields take their defaults; an invalid field takes its default, so one
bad value never resets the rest. A file that cannot be read, is not valid TOML
or has invalid values is first copied to `settings.toml.broken-<stamp>`
(`rc_core::settings::load_settings_checked`), so saving the defaults never
destroys it; the problem is kept for the GUI (`take_load_problem`) and shown
as the top-bar notice.

## Testing

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

- **Never open a real serial port in a test.** The instrument may be
  connected and running. The link layer is tested against a simulated
  instrument (`crates/rc-protocol/tests/sim`, an in-memory `Transport`), and
  the end-to-end test runs acquisition, method store and CSV writing against
  it. On Linux the app lists ports from sysfs and never opens a port to list
  it; do not call `serialport::available_ports()` there (its udev
  enumeration opens serial8250 ports).
- **Never touch the real settings.** Tests call
  `rc_core::settings::set_config_dir_override(Some(temp_dir))`; setting the
  `RUSTYCORN_CONFIG_DIR` environment variable from a test would need `unsafe`
  in Rust 2024. The environment variable is for starting the app by hand.
- **Reference outputs.** `crates/rc-core/tests/data`,
  `crates/rc-methods/tests/data` and `crates/rc-plotdata/testdata` hold fixed,
  known-good outputs (CSV files byte for byte, reader summaries, method files,
  decoder results, plot labels). They guard the file formats: when a test
  against them fails, the change broke compatibility, and the data must not
  simply be regenerated. `RC_REFERENCE_DIR` points the rc-core reader test at
  further `<log>.csv` + `<log>.csv.summary.txt` pairs; `RC_EXAMPLE_LOGS`
  enables the rc-export tests that render real logs from a folder.

## Running the GUI safely, screenshots and demos

Use a temporary settings folder with demo mode on, so neither the real
settings nor a serial port are touched:

```bash
export RUSTYCORN_CONFIG_DIR=$(mktemp -d)
printf 'demo_mode = true\nserial_port = "/nonexistent"\ndata_dir = "%s"\n' "$(mktemp -d)" \
    > "$RUSTYCORN_CONFIG_DIR/settings.toml"
cargo run --release
```

Or open a log read-only: `cargo run --release -- run.csv`.

Environment variables for screenshots and layout checks:

- `RUSTYCORN_WINDOW_SIZE=1440x880` — window size at start (never below the
  minimum, 1100×680).
- `RUSTYCORN_SHOW_PAGES="Settings,/path/run.csv,Edit run info"` — open these
  pages at start (`Settings`, `Methods`, a `.csv` as an opened run), open the
  shown page's run-info form (`Edit run info`), switch the shown run page to
  the overlay plot (`Overlay`) or open its run log (`Run log`), or never ask
  for run information (`No run prompt`).

Take the screenshot of the window only, e.g. on X11 with ImageMagick:
`import -window "$(xwininfo -name RustyCorn | awk '/Window id/{print $4}')" shot.png`
(with another RustyCorn window open, pick the window whose `_NET_WM_PID`, from
`xprop`, is the process you started). `assets/screenshot.png` and
`assets/screenshots/` are made this way in demo mode, light theme, at
`RUSTYCORN_WINDOW_SIZE=1600x950` (Settings 1200x1120, method editor
1600x1120), with a data folder such as `/tmp/akta-data` so no personal path
shows. The example method of a new methods folder is in
`rc_methods::examples` (`install_example_methods`).

Single-crate demos (never open a serial port, never write the settings):

```bash
cargo run -p rc-runview --example runview_demo -- --demo            # live demo source
cargo run -p rc-runview --example runview_demo -- --csv run.csv     # a copied log, viewer mode
cargo run -p rc-panels --example panels_demo -- --preset 2 --light
cargo run -p rc-methodeditor --example editor_demo -- [methods folder] --bad
```

The module docs at the top of each example list all their flags (for example
`--size WxH`, `--overlay`, `--hover X,Y`, `--quit-after S` for scripted
screenshots of the run view).

## Style

Idiomatic Rust 2024, no `unsafe`, no `unwrap()` on I/O or user data outside
tests. Comments are short and explain why, especially instrument quirks and
format rules. Messages go through `log::info!` / `warn!` / `error!`; the
console shows them as `[ Info: …`.
