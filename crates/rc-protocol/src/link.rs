//! The link-layer loop.
//!
//! Runs on its own OS thread so polling never waits for the GUI: the Akta
//! resets the link when the PC stays silent for a few seconds. Each decoded
//! message is applied to the shared AppState under its lock (short critical
//! sections); the lock is never held while talking to the port. Method store
//! requests are answered here too, so the reply never waits for a busy GUI
//! (the Akta gives up after about 2 s).

use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, unbounded};
use rc_core::opc::*;
use rc_core::{EventKind, SharedApp};
use rc_methods::MethodTransfer;

use crate::frame::*;
use crate::handlers::{capture_opc, dispatch_rx_opc, dump_run_setup, method_dump_received_with};
use crate::{BAUD_RATE, IDLE_TIMEOUT_MS, POLL_INTERVAL_MS, PROBE_INTERVAL_MS, REPLY_TIMEOUT_MS};

/// Wait before opening the port again after it was missing or failed.
pub const REOPEN_DELAY_MS: u64 = 2000;

/// Byte stream to the instrument: the serial port, or a simulated Akta in tests.
pub trait Transport {
    /// Write all bytes (the OS may still be sending them when this returns).
    fn write(&mut self, data: &[u8]) -> io::Result<()>;
    /// Discard received bytes not read yet.
    fn clear_input(&mut self) -> io::Result<()>;
    /// Number of received bytes waiting to be read.
    fn bytes_to_read(&mut self) -> io::Result<usize>;
    /// Read up to `buf.len()` of the waiting bytes without blocking for more.
    fn read_available(&mut self, buf: &mut [u8]) -> io::Result<usize>;
}

/// The real serial port: 9600 baud 8N1, no flow control, RTS and DTR on.
pub struct SerialTransport {
    port: Box<dyn serialport::SerialPort>,
}

impl SerialTransport {
    pub fn open(path: &str) -> io::Result<SerialTransport> {
        let mut port = serialport::new(path, BAUD_RATE)
            .data_bits(serialport::DataBits::Eight)
            .parity(serialport::Parity::None)
            .stop_bits(serialport::StopBits::One)
            .flow_control(serialport::FlowControl::None)
            .dtr_on_open(true)
            // only bounds writes and reads of bytes already waiting
            .timeout(Duration::from_millis(1000))
            .open()?;
        port.write_request_to_send(true)?;
        port.write_data_terminal_ready(true)?;
        Ok(SerialTransport { port })
    }
}

impl Transport for SerialTransport {
    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        io::Write::write_all(&mut self.port, data)
    }
    fn clear_input(&mut self) -> io::Result<()> {
        Ok(self.port.clear(serialport::ClearBuffer::Input)?)
    }
    fn bytes_to_read(&mut self) -> io::Result<usize> {
        Ok(self.port.bytes_to_read()? as usize)
    }
    fn read_available(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.bytes_to_read()?.min(buf.len());
        if n == 0 {
            return Ok(0);
        }
        match io::Read::read(&mut self.port, &mut buf[..n]) {
            Ok(n) => Ok(n),
            Err(err) if err.kind() == io::ErrorKind::TimedOut => Ok(0),
            Err(err) => Err(err),
        }
    }
}

/// Link timing; the defaults are the instrument's.
/// Tests shorten them to run quickly.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkConfig {
    pub poll_interval_ms: u64,
    /// silence before we warn and start probing (never disconnects)
    pub idle_timeout_ms: u64,
    /// while silent, send a poll frame this often to catch a link reset
    pub probe_interval_ms: u64,
    pub reply_timeout_ms: u64,
    /// wait before (re)opening the port
    pub reopen_delay_ms: u64,
}

impl Default for LinkConfig {
    fn default() -> Self {
        LinkConfig {
            poll_interval_ms: POLL_INTERVAL_MS,
            idle_timeout_ms: IDLE_TIMEOUT_MS,
            probe_interval_ms: PROBE_INTERVAL_MS,
            reply_timeout_ms: REPLY_TIMEOUT_MS,
            reopen_delay_ms: REOPEN_DELAY_MS,
        }
    }
}

/// LinkState — mirrors thread-local variables in thdLinklayer.
#[derive(Clone, Debug, Default)]
pub struct LinkState {
    pub connected: bool,
    pub sn: u8,
    pub rn: u8,
    pub pending_tx: bool,
    pub idle_ms: u64,
    pub driver_enabled: bool,
    pub silent: bool,
    pub probe_ms: u64,
    /// method dump or save in progress
    pub method: MethodTransfer,
}

/// Run `f`, logging a panic instead of ending the link thread: an error in one
/// handler must not drop the connection.
fn guarded(what: impl FnOnce() -> String, f: impl FnOnce()) {
    if let Err(err) = catch_unwind(AssertUnwindSafe(f)) {
        let msg = err
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| err.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        log::error!("{}: {msg}", what());
    }
}

/// Connection state for the GUI (the GUI colours it by prefix).
fn say(app: &SharedApp, txt: String) {
    let mut a = app.lock();
    a.status = txt;
    a.touch();
}

fn event(app: &SharedApp, kind: EventKind, text: &str) {
    app.lock().push_event(kind, text);
}

fn drain(rx: &Receiver<OpcMessage>) {
    while rx.try_recv().is_ok() {}
}

/// (Re)initialise the link after a connect handshake from the Akta.
fn link_reset(st: &mut LinkState, tx: &Sender<OpcMessage>, rx: &Receiver<OpcMessage>) {
    st.sn = 0;
    st.rn = 0;
    st.pending_tx = false;
    st.idle_ms = 0;
    st.probe_ms = 0;
    st.silent = false;
    st.connected = true;
    st.driver_enabled = false;
    st.method = MethodTransfer::default();
    drain(rx);
    let _ = tx.send((OPC_IDENTITY_REQUEST, vec![0x00]));
}

/// Any valid frame from the Akta proves the link is alive.
fn link_alive(st: &mut LinkState, app: &SharedApp) {
    st.idle_ms = 0;
    st.probe_ms = 0;
    if st.silent {
        st.silent = false;
        log::info!("Akta responding again");
        event(app, EventKind::Log, "Link: Akta responding again");
    }
}

/// Decode an I-frame from the Akta and apply each message. The driver-enable
/// handshake (0x1001/0x01 after the version string) is queued here.
pub fn parse_rx_data_frame(
    frame: &[u8],
    tx: &Sender<OpcMessage>,
    st: &mut LinkState,
    app: &SharedApp,
) {
    let len = frame.len();
    if len < 6 {
        return;
    }
    let payload = &frame[3..len - 2];
    for (opc, data) in decode_opc_payload(payload) {
        if opc == OPC_VERSION_STRING && !st.driver_enabled {
            st.driver_enabled = true;
            let _ = tx.send((OPC_IDENTITY_REQUEST, vec![0x01]));
            log::info!("Queued driver-enable (OPC 0x1001/0x01)");
        }
        guarded(
            || "Error handling Akta data".to_string(),
            || dispatch_rx_opc(&mut app.lock(), opc, &data),
        );
        guarded(
            || format!("Error handling method OPC 0x{opc:04x}"),
            || {
                if rc_methods::is_method_opc(opc) {
                    method_opc(st, opc, &data, tx, app);
                }
            },
        );
    }
}

/// Method store requests are answered here on the link thread; the methods
/// folder is read without holding the app lock.
fn method_opc(st: &mut LinkState, opc: u16, data: &[u8], tx: &Sender<OpcMessage>, app: &SharedApp) {
    let (dir, last_loaded): (PathBuf, u16) = {
        let a = app.lock();
        (a.method_dir.clone(), a.last_loaded_method)
    };
    let Some(action) = st.method.handle(opc, data, &dir) else {
        return;
    };
    for m in &action.replies {
        let _ = tx.send(m.clone());
    }
    // which PC method a dump is: compared with the stored methods, unlocked
    let setup = action
        .dump
        .as_ref()
        .map(|dump| dump_run_setup(&dir, last_loaded, dump));
    let mut a = app.lock();
    for (o, d) in &action.replies {
        capture_opc(&mut a, *o, d, true);
    }
    if let Some(note) = &action.note {
        a.push_event(EventKind::Log, note.as_str());
    }
    if let (Some(dump), Some(setup)) = (&action.dump, setup) {
        method_dump_received_with(&mut a, dump, setup);
    }
    if matches!(
        opc,
        OPC_METHOD_SAVE_END | OPC_METHOD_DELETE | OPC_METHOD_LOAD
    ) {
        let load_ok = action.replies.last() == Some(&(OPC_METHOD_LOAD_END, vec![0x00]));
        let loaded = if opc == OPC_METHOD_LOAD && load_ok {
            u16_le(data, 0)
        } else {
            0
        };
        if loaded > 0 {
            a.last_loaded_method = loaded;
        }
        a.methods_changed += 1;
        a.touch();
    }
}

/// Write a frame and read the reply. All but the last two bytes go out first,
/// then the input buffer is cleared, so a stale reply cannot be taken for the
/// answer to this frame: the Akta cannot answer before it has the CR. LF ends a
/// frame like CR. Returns what arrived of a frame within the timeout.
pub fn write_frame_read_reply<T: Transport + ?Sized>(
    port: &mut T,
    frame: &[u8],
    reply_timeout_ms: u64,
) -> io::Result<Vec<u8>> {
    let n = frame.len();
    log::debug!("TX {}", fmt_frame(frame));
    port.write(&frame[..n - 2])?;
    port.clear_input()?;
    port.write(&frame[n - 2..])?;

    let mut reply = Vec::new();
    let mut in_frame = false;
    let mut buf = [0u8; 256];
    let deadline = Instant::now() + Duration::from_millis(reply_timeout_ms);
    'read: while Instant::now() < deadline {
        if port.bytes_to_read()? > 0 {
            let got = port.read_available(&mut buf)?;
            for &b in &buf[..got] {
                if b == FRAME_START {
                    reply.clear();
                    reply.push(b);
                    in_frame = true;
                } else if in_frame {
                    let b = if b == 0x0a { FRAME_END } else { b };
                    reply.push(b);
                    if b == FRAME_END {
                        break 'read;
                    }
                    if reply.len() > 999 {
                        reply.clear();
                        in_frame = false;
                    }
                }
            }
        } else {
            thread::sleep(Duration::from_millis(1));
        }
    }
    if !reply.is_empty() {
        log::debug!("RX {}", fmt_frame(&reply));
    }
    Ok(reply)
}

fn stopped(shutdown: &AtomicBool) -> bool {
    shutdown.load(Ordering::Relaxed)
}

/// Sleep, but wake early when shutting down.
fn sleep_unless_stopped(shutdown: &AtomicBool, ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    while !stopped(shutdown) {
        let now = Instant::now();
        if now >= end {
            break;
        }
        thread::sleep((end - now).min(Duration::from_millis(20)));
    }
}

/// The link on the real serial port, until `shutdown` is set.
pub fn run_linklayer(app: SharedApp, shutdown: &AtomicBool, serial_port: &str) {
    run_linklayer_with(
        &app,
        shutdown,
        serial_port,
        || SerialTransport::open(serial_port),
        &LinkConfig::default(),
    );
}

/// The link over any transport. `open` is called for every session: one
/// session per connection; open the port (retrying while it is missing), then
/// poll. If the port fails (cable pulled, adaptor reset: "Input/output
/// error"), the session ends and a new one starts — the link never just stops.
pub fn run_linklayer_with<T, O>(
    app: &SharedApp,
    shutdown: &AtomicBool,
    port_name: &str,
    mut open: O,
    cfg: &LinkConfig,
) where
    T: Transport,
    O: FnMut() -> io::Result<T>,
{
    log::info!(
        "Link layer on thread {:?}",
        thread::current().name().unwrap_or("unnamed")
    );
    // Unbounded: the link queues a method load (dozens of messages) itself.
    let (tx, rx) = unbounded::<OpcMessage>();
    while !stopped(shutdown) {
        // A missing or unplugged adaptor is retried until it appears, not fatal.
        let mut port = None;
        let mut warned = false;
        while port.is_none() && !stopped(shutdown) {
            match open() {
                Ok(p) => port = Some(p),
                Err(err) => {
                    if !warned {
                        log::warn!("Serial port {port_name} not available — retrying: {err}");
                        say(
                            app,
                            format!("Serial port {port_name} not available — retrying"),
                        );
                    }
                    warned = true;
                    sleep_unless_stopped(shutdown, cfg.reopen_delay_ms);
                }
            }
        }
        let Some(mut port) = port else { break };
        log::info!("Opened {port_name} at {BAUD_RATE} baud");
        say(app, format!("Waiting for Akta on {port_name}"));

        if let Err(err) = run_session(&mut port, app, shutdown, port_name, &tx, &rx, cfg) {
            log::warn!("Serial connection lost — reconnecting: {err}");
            event(
                app,
                EventKind::Log,
                "Link: serial connection lost — reconnecting",
            );
            say(
                app,
                format!("Connection to {port_name} lost — reconnecting"),
            );
        }
        drop(port);
        if !stopped(shutdown) {
            sleep_unless_stopped(shutdown, cfg.reopen_delay_ms);
        }
    }
    log::info!("Link layer shutting down");
}

/// Poll the Akta every `poll_interval_ms` until shutdown (Ok) or a port error.
fn run_session<T: Transport>(
    port: &mut T,
    app: &SharedApp,
    shutdown: &AtomicBool,
    port_name: &str,
    tx: &Sender<OpcMessage>,
    rx: &Receiver<OpcMessage>,
    cfg: &LinkConfig,
) -> io::Result<()> {
    let mut st = LinkState::default();
    let poll_frame = build_poll_frame();
    let mut cur_frame = poll_frame.clone();
    let mut t_last = Instant::now();
    log::info!("Link layer started; polling for Akta");

    while !stopped(shutdown) {
        let elapsed_ms = t_last.elapsed().as_millis() as u64;
        let delay_ms = if elapsed_ms < cfg.poll_interval_ms {
            cfg.poll_interval_ms - elapsed_ms
        } else {
            1
        };
        thread::sleep(Duration::from_millis(delay_ms));
        let t_now = Instant::now();
        let elapsed_ms = (t_now - t_last).as_millis() as u64;
        t_last = t_now;

        let mut probing = false;
        if !st.connected {
            drain(rx);
            cur_frame.clone_from(&poll_frame);
        } else {
            st.idle_ms += elapsed_ms;
            // While the Akta is silent, interleave poll frames: if it has
            // reset its link it answers with CTRL_CONN and we resync below.
            if st.idle_ms > cfg.idle_timeout_ms {
                st.probe_ms += elapsed_ms;
                if st.probe_ms >= cfg.probe_interval_ms {
                    st.probe_ms = 0;
                    probing = true;
                }
            }
            if !probing && !st.pending_tx {
                if let Ok((opc, data)) = rx.try_recv() {
                    let payload = encode_opc_payload(opc, &data);
                    st.sn = (st.sn + 1) & 0x03;
                    cur_frame = build_i_frame(&payload, st.sn, st.rn);
                    st.pending_tx = true;
                } else {
                    cur_frame = build_ack_frame(st.rn);
                }
            }
        }

        // cur_frame is kept intact while probing so a pending I-frame is retransmitted later
        let frame = if probing { &poll_frame } else { &cur_frame };
        let reply = write_frame_read_reply(port, frame, cfg.reply_timeout_ms)?;
        let rx_len = reply.len();

        if rx_len > 0 && validate_rx_frame(&reply) {
            let ctrl = reply[2];
            if !st.connected {
                if rx_len == 5 && ctrl == CTRL_CONN {
                    log::info!("Akta connected");
                    link_reset(&mut st, tx, rx);
                    event(app, EventKind::Connect, "Akta connected");
                    say(app, format!("● Connected on {port_name}"));
                } else {
                    log::warn!("Unexpected frame while disconnected: ctrl=0x{ctrl:x}");
                }
            } else {
                if st.silent {
                    say(app, format!("● Connected on {port_name}"));
                }
                link_alive(&mut st, app);

                if rx_len == 5 && ctrl == CTRL_CONN {
                    // The Akta restarted its side of the link: resync rather than drop out
                    log::warn!("Akta re-established the link — resynchronising");
                    link_reset(&mut st, tx, rx);
                    event(app, EventKind::Log, "Link: Akta re-established the link");
                } else if (ctrl & 0xf0) == CTRL_IFRAME {
                    let received_sn = (ctrl >> 2) & 0x03;
                    if received_sn == (st.rn + 1) & 0x03 {
                        st.rn = received_sn;
                        parse_rx_data_frame(&reply, tx, &mut st, app);
                    } else if received_sn == st.rn {
                        log::warn!("LinkLayer: duplicate I-frame SN={received_sn}");
                    } else {
                        // Frames were lost; accept this one and resync, otherwise the
                        // Akta retransmits it forever and nothing more is recorded.
                        log::warn!(
                            "LinkLayer: SN error — expected {}, got {received_sn}; resynchronising",
                            (st.rn + 1) & 0x03
                        );
                        st.rn = received_sn;
                        parse_rx_data_frame(&reply, tx, &mut st, app);
                    }
                    if st.sn == (ctrl & 0x03) {
                        st.pending_tx = false;
                    } else {
                        log::warn!("LinkLayer: RN mismatch in I-frame ctrl");
                    }
                } else if (ctrl & 0xfc) == CTRL_RR_BASE {
                    if st.sn == (ctrl & 0x03) {
                        st.pending_tx = false;
                    } else {
                        log::warn!("LinkLayer: RN mismatch in RR frame");
                    }
                } else {
                    log::warn!("LinkLayer: unexpected ctrl 0x{ctrl:x}");
                }
            }
        }

        // Never disconnect on silence: keep the link and the log open and just report it
        if st.connected && !st.silent && st.idle_ms > cfg.idle_timeout_ms {
            st.silent = true;
            log::warn!(
                "No response from Akta for {} s — staying connected, probing",
                st.idle_ms / 1000
            );
            event(app, EventKind::Log, "Link: no response from Akta — waiting");
            say(
                app,
                format!("No response from Akta on {port_name} — waiting"),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use rc_core::AppState;

    use super::*;

    /// A port that answers each complete frame with a scripted reply.
    #[derive(Default)]
    struct Scripted {
        written: Vec<u8>,
        input: VecDeque<u8>,
        replies: VecDeque<Vec<u8>>,
        /// put stale bytes in the input while a frame is half written
        noise: bool,
    }

    impl Transport for Scripted {
        fn write(&mut self, data: &[u8]) -> io::Result<()> {
            self.written.extend_from_slice(data);
            if data.last() == Some(&FRAME_END) {
                if let Some(r) = self.replies.pop_front() {
                    self.input.extend(r);
                }
            } else if self.noise {
                self.input.extend(b"#1uX\r");
            }
            Ok(())
        }
        fn clear_input(&mut self) -> io::Result<()> {
            self.input.clear();
            Ok(())
        }
        fn bytes_to_read(&mut self) -> io::Result<usize> {
            Ok(self.input.len())
        }
        fn read_available(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = self.input.len().min(buf.len()).min(3); // a few bytes at a time
            for b in buf.iter_mut().take(n) {
                *b = self.input.pop_front().unwrap_or(0);
            }
            Ok(n)
        }
    }

    #[test]
    fn reply_is_read_after_clearing_stale_input() {
        let mut port = Scripted {
            noise: true,
            ..Default::default()
        };
        port.replies.push_back(b"xx#1\x30a\r#junk".to_vec());
        let reply = write_frame_read_reply(&mut port, &build_poll_frame(), 50).unwrap();
        assert_eq!(port.written, b"#0S5\r");
        // the stale UA was cleared; bytes before '#' are skipped; reading stops at CR
        assert_eq!(reply, b"#1\x30a\r");
    }

    #[test]
    fn line_feed_ends_a_frame_and_restart_on_hash() {
        let mut port = Scripted::default();
        port.replies.push_back(b"#1u#1uX\n".to_vec());
        let reply = write_frame_read_reply(&mut port, &build_ack_frame(0), 50).unwrap();
        assert_eq!(reply, b"#1uX\r");
        assert!(validate_rx_frame(&reply));
    }

    #[test]
    fn timeout_returns_partial_or_nothing() {
        let mut port = Scripted::default();
        let t = Instant::now();
        assert!(
            write_frame_read_reply(&mut port, &build_poll_frame(), 30)
                .unwrap()
                .is_empty()
        );
        assert!(t.elapsed() >= Duration::from_millis(30));
        port.replies.push_back(b"#1uX".to_vec());
        let reply = write_frame_read_reply(&mut port, &build_poll_frame(), 30).unwrap();
        assert_eq!(reply, b"#1uX");
        assert!(!validate_rx_frame(&reply));
        // over 999 bytes without CR: dropped
        let mut long = vec![FRAME_START];
        long.extend(std::iter::repeat_n(b'A', 1200));
        port.replies.push_back(long);
        assert!(
            write_frame_read_reply(&mut port, &build_poll_frame(), 30)
                .unwrap()
                .is_empty()
        );
    }

    /// The method store end to end: a list request in a frame from the
    /// Akta is answered from the methods folder.
    #[test]
    fn list_request_in_a_frame_is_answered() {
        let dir = tempfile::tempdir().unwrap();
        for n in [3u16, 5, 12] {
            std::fs::write(rc_methods::method_path(dir.path(), n), "x\r\n").unwrap();
        }
        let mut frame = build_i_frame(
            &encode_opc_payload(OPC_METHOD_LIST, &[0x01, 0x00, 0x01]),
            1,
            0,
        );
        frame[1] = AKTA_ADDR;
        let n = frame.len();
        frame[n - 2] = compute_checksum(&frame, 1, n - 3);
        assert!(validate_rx_frame(&frame));
        let app = AppState {
            method_dir: dir.path().to_path_buf(),
            ..AppState::default()
        }
        .shared();
        let (tx, rx) = unbounded();
        parse_rx_data_frame(&frame, &tx, &mut LinkState::default(), &app);
        let (opc, data) = rx.try_recv().unwrap();
        assert_eq!(opc, OPC_METHOD_LIST_REPLY);
        assert_eq!(u16_le(&data, 0), 3);
        assert_eq!(data.len(), 20);
        assert!(rx.try_recv().is_err());
    }

    /// The version string queues the driver enable once per connection.
    #[test]
    fn version_queues_driver_enable_once() {
        let app = AppState::default().shared();
        let (tx, rx) = unbounded();
        let mut st = LinkState::default();
        let mut payload = encode_opc_payload(OPC_PROG_NAME, b"Manual run");
        payload.extend(encode_opc_payload(OPC_VERSION_STRING, b"V2.01"));
        let frame = [b"#1D".as_slice(), &payload, b"?\r"].concat();
        parse_rx_data_frame(&frame, &tx, &mut st, &app);
        parse_rx_data_frame(&frame, &tx, &mut st, &app);
        assert!(st.driver_enabled);
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            vec![(OPC_IDENTITY_REQUEST, vec![0x01])]
        );
        assert_eq!(app.lock().inst.version, "V2.01");
        assert_eq!(app.lock().inst.prog_name, "Manual run");
        link_reset(&mut st, &tx, &rx);
        assert!(!st.driver_enabled && st.connected);
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            vec![(OPC_IDENTITY_REQUEST, vec![0x00])]
        );
    }

    #[test]
    fn serial_port_that_does_not_exist_fails_to_open() {
        // never a real device: a path that cannot exist
        assert!(SerialTransport::open("/nonexistent/rustycorn-test-port").is_err());
    }
}
