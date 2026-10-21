//! A simulated ÄKTAprime for the link-layer tests: the simulator answers each
//! frame the PC writes, like the instrument does (connect handshake, I-frames
//! with SN/RN, receive-ready), and can be told to go silent, restart its link,
//! duplicate or skip frames, or fail the port. No serial port is ever opened.
#![allow(dead_code)] // each test file uses part of it

use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use rc_core::SharedApp;
use rc_core::opc::*;
use rc_protocol::frame::*;
use rc_protocol::link::{LinkConfig, Transport};

pub const METHOD_DUMP: &str = concat!(
    "{l{5}{2 Method Dump}{2}{2 AKTAprime|3Ver|2, V2.01|?}{2}{2 Dump Format Ver|2, V1.00|?}",
    "{2}{l{5}}}{B}{2 Method Settings}{B}Method Type|8,Manaul Method}Method|=,1}Method base|8,ml}",
    "Frac base|:,ml}Max pressure (kPa) ,1000}Alarm (min/ml)|5,0.0}}{E}{2 Method Breakpoints}{E}|E}",
    "Brkp Volume|3Conc Flow|2Frac|3Buff Inj|4Peak|3Autozzero Eventmark}|Z(mAu/min)}}",
    ",1|3,0.0|5,0|3,0.5|2,0.0|3,8|3,Load|2,0|5,N|7,N},2|3,99999.9 ,0|3,0.5|2,0.0|3,8|3,Load|2,0|5,N|7,N}"
);

pub const TREND: [u8; 14] = [
    10, 7, 0x40, 0x42, 0x0f, 0x10, 0x27, 0x00, 0xbc, 0x02, 0x50, 0xa0, 0x5e, 0x01,
];

/// Encoded payload of one I-frame holding these messages.
pub fn payload(msgs: &[(u16, &[u8])]) -> Vec<u8> {
    msgs.iter()
        .flat_map(|(o, d)| encode_opc_payload(*o, d))
        .collect()
}

#[derive(Default)]
pub struct Sim {
    pub app: Option<SharedApp>,
    /// bytes of the frame the PC is writing
    pub partial: Vec<u8>,
    /// bytes for the PC to read
    pub input: VecDeque<u8>,
    /// every complete frame from the PC
    pub pc_frames: Vec<Vec<u8>>,
    pub bad_pc_frames: usize,
    /// status texts of the app, as seen whenever the PC wrote (consecutive duplicates dropped)
    pub statuses: Vec<String>,

    // the Akta's side of the link
    pub connected: bool,
    pub sn: u8,
    pub rn: u8,
    pub unacked: Option<(u8, Vec<u8>)>,
    pub outbox: VecDeque<Vec<u8>>,
    /// messages received in new I-frames from the PC
    pub received: Vec<OpcMessage>,
    /// sent once the PC enables the driver (0x1001/0x01)
    pub on_enable: Vec<Vec<u8>>,

    // behaviour
    /// answer nothing to the next n frames
    pub ignore_frames: usize,
    /// once the next message is sent, ignore this many frames (its answer is lost)
    pub ignore_after_send: usize,
    /// stay silent until this many polls have arrived (0 = answer)
    pub silent_until_polls: usize,
    pub polls_while_silent: usize,
    /// answer the next frame with UA, as after a restart of the Akta's link
    pub send_ua_next: bool,
    /// send the next I-frame again with the same SN once it is acknowledged
    pub duplicate_next: bool,
    pub dup_pending: Option<(u8, Vec<u8>)>,
    pub dups_sent: usize,
    /// skip one SN (as if a frame was lost)
    pub skip_sn_next: bool,
    /// end frames with LF instead of CR
    pub lf_endings: bool,
    /// put a stale UA in the input while a frame is half written (cleared by the PC)
    pub noise: bool,
    /// the next write fails, like an unplugged adaptor
    pub fail_io: bool,
    pub opens: usize,
    pub fail_opens: usize,
}

impl Sim {
    pub fn send(&mut self, mut frame: Vec<u8>) {
        if self.lf_endings
            && let Some(last) = frame.last_mut()
        {
            *last = 0x0a;
        }
        self.input.extend(frame);
    }

    pub fn i_frame(&self, sn: u8, payload: &[u8]) -> Vec<u8> {
        let mut f = build_i_frame(payload, sn, self.rn);
        f[1] = AKTA_ADDR;
        let n = f.len();
        f[n - 2] = compute_checksum(&f, 1, n - 3);
        f
    }

    pub fn rr_frame(&self) -> Vec<u8> {
        let mut f = vec![FRAME_START, AKTA_ADDR, CTRL_RR_BASE | self.rn, 0, FRAME_END];
        f[3] = compute_checksum(&f, 1, 2);
        f
    }

    pub fn ua_frame() -> Vec<u8> {
        vec![FRAME_START, AKTA_ADDR, CTRL_CONN, b'X', FRAME_END]
    }

    pub fn react(&mut self, msg: &OpcMessage) {
        match (msg.0, msg.1.as_slice()) {
            (OPC_IDENTITY_REQUEST, [0x00]) => {
                // NULs at the end of the name, as the instrument sends them
                let mut p = encode_opc_payload(OPC_PROG_NAME, b"Manual run  ");
                let n = p.len();
                p[n - 2..].fill(0);
                p.extend(encode_opc_payload(OPC_VERSION_STRING, b"V2.01"));
                self.outbox.push_back(p);
            }
            (OPC_IDENTITY_REQUEST, [0x01]) => {
                let script = std::mem::take(&mut self.on_enable);
                self.outbox.extend(script);
            }
            _ => {}
        }
    }

    /// A complete frame from the PC.
    pub fn handle(&mut self, frame: Vec<u8>) {
        let n = frame.len();
        let ok = n >= 5
            && frame[0] == FRAME_START
            && frame[1] == PC_ADDR
            && frame[n - 2] == compute_checksum(&frame, 1, n - 3);
        self.pc_frames.push(frame.clone());
        if !ok {
            self.bad_pc_frames += 1;
            return;
        }
        let ctrl = frame[2];
        if self.ignore_frames > 0 {
            self.ignore_frames -= 1;
            return;
        }
        if self.silent_until_polls > 0 {
            if ctrl == CTRL_POLL {
                self.polls_while_silent += 1;
                if self.polls_while_silent >= self.silent_until_polls {
                    self.silent_until_polls = 0;
                    self.polls_while_silent = 0;
                }
            }
            return;
        }
        if ctrl == CTRL_POLL {
            if !self.connected || self.send_ua_next {
                // (re)connect: both sides start from SN = RN = 0
                self.connected = true;
                self.send_ua_next = false;
                self.sn = 0;
                self.rn = 0;
                self.unacked = None;
                self.send(Sim::ua_frame());
            } else {
                let rr = self.rr_frame();
                self.send(rr);
            }
            return;
        }
        if !self.connected {
            return; // a reset Akta waits for SNRM
        }
        if self.send_ua_next {
            self.send_ua_next = false;
            self.sn = 0;
            self.rn = 0;
            self.unacked = None;
            self.send(Sim::ua_frame());
            return;
        }
        let pc_rn = ctrl & 0x03;
        if (ctrl & 0xf0) == CTRL_IFRAME {
            let pc_sn = (ctrl >> 2) & 0x03;
            if pc_sn == (self.rn + 1) & 0x03 {
                self.rn = pc_sn;
                for msg in decode_opc_payload(&frame[3..n - 2]) {
                    self.react(&msg);
                    self.received.push(msg);
                }
            }
        }
        if let Some((sn, p)) = self.unacked.take() {
            if pc_rn != sn {
                self.unacked = Some((sn, p));
            } else if self.duplicate_next {
                self.duplicate_next = false;
                self.dup_pending = Some((sn, p));
            }
        }
        // reply: retransmit, a duplicate, the next message, or receive-ready
        if let Some((sn, p)) = self.unacked.clone() {
            let f = self.i_frame(sn, &p);
            self.send(f);
        } else if let Some((sn, p)) = self.dup_pending.take() {
            self.dups_sent += 1;
            let f = self.i_frame(sn, &p);
            self.send(f);
        } else if let Some(p) = self.outbox.pop_front() {
            let step = if std::mem::take(&mut self.skip_sn_next) {
                2
            } else {
                1
            };
            self.sn = (self.sn + step) & 0x03;
            let f = self.i_frame(self.sn, &p);
            self.unacked = Some((self.sn, p));
            self.send(f);
            self.ignore_frames = std::mem::take(&mut self.ignore_after_send);
        } else {
            let rr = self.rr_frame();
            self.send(rr);
        }
    }

    pub fn record_status(&mut self) {
        if let Some(app) = self.app.clone() {
            let status = app.lock().status.clone();
            if self.statuses.last() != Some(&status) {
                self.statuses.push(status);
            }
        }
    }

    pub fn count(&self, msg: (u16, &[u8])) -> usize {
        self.received
            .iter()
            .filter(|(o, d)| *o == msg.0 && d.as_slice() == msg.1)
            .count()
    }
}

pub struct SimPort(pub Arc<Mutex<Sim>>);

impl Transport for SimPort {
    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        let mut sim = self.0.lock();
        if sim.fail_io {
            return Err(io::Error::other("Input/output error"));
        }
        sim.record_status();
        sim.partial.extend_from_slice(data);
        if sim.partial.last() == Some(&FRAME_END) {
            let frame = std::mem::take(&mut sim.partial);
            sim.handle(frame);
        } else if sim.noise {
            sim.input.extend(b"#1uX\r");
        }
        Ok(())
    }
    fn clear_input(&mut self) -> io::Result<()> {
        self.0.lock().input.clear();
        Ok(())
    }
    fn bytes_to_read(&mut self) -> io::Result<usize> {
        Ok(self.0.lock().input.len())
    }
    fn read_available(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut sim = self.0.lock();
        let n = sim.input.len().min(buf.len());
        for b in buf.iter_mut().take(n) {
            *b = sim.input.pop_front().unwrap_or(0);
        }
        Ok(n)
    }
}

/// Opens the simulated port; the first `fail_opens` attempts fail as if the
/// adaptor were missing. A new connection finds the Akta waiting for SNRM.
pub fn opener(sim: &Arc<Mutex<Sim>>) -> impl FnMut() -> io::Result<SimPort> + use<> {
    let sim = sim.clone();
    move || {
        let mut s = sim.lock();
        s.record_status();
        s.opens += 1;
        if s.opens <= s.fail_opens {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "No such file or directory",
            ));
        }
        s.fail_io = false;
        s.connected = false;
        s.partial.clear();
        s.input.clear();
        s.unacked = None;
        Ok(SimPort(sim.clone()))
    }
}

pub fn wait_until(what: &str, timeout_s: u64, mut cond: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(timeout_s);
    while !cond() {
        assert!(Instant::now() < end, "timed out waiting for: {what}");
        thread::sleep(Duration::from_millis(5));
    }
}

pub fn fast() -> LinkConfig {
    LinkConfig {
        poll_interval_ms: 5,
        idle_timeout_ms: 200,
        probe_interval_ms: 50,
        reply_timeout_ms: 20,
        reopen_delay_ms: 30,
    }
}
