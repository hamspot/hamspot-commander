//! TCP client for a running `rigctld`.
//!
//! Commands use Hamlib's extended response prefix `+\`. The read timeout is
//! the completion signal: `rigctld` answers with `RPRT`, and a peer that
//! accepts the socket and then stays silent never closes it.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use crate::caps::{parse_caps, Caps};
use crate::{Command, LinkKind, Rig, RigError, Snapshot};

const READ_LEVELS: &[&str] = &[
    "STRENGTH",
    "AF",
    "RF",
    "RFPOWER",
    "IF",
    "NR",
    "MICGAIN",
    "VOXDELAY",
    "VOXGAIN",
    "AGC",
    "PREAMP",
    "ALC",
    "SWR",
    "RFPOWER_METER",
];

const READ_FUNCS: &[&str] = &[
    "NR", "LOCK", "TUNER", "VOX", "SBKIN", "MON", "COMP", "NB", "FAGC", "ANF",
];

pub struct LiveRig {
    stream: TcpStream,
    endpoint: String,
    caps: Caps,
    /// Levels and funcs whose first read failed. Asking again would spam `RPRT`.
    skip: BTreeSet<String>,
    ant: i32,
    message: String,
}

struct Reply {
    rprt: i32,
    lines: Vec<String>,
}

impl LiveRig {
    pub fn connect(host: &str, port: u16) -> Result<Self, RigError> {
        let endpoint = format!("{host}:{port}");
        let addr = endpoint
            .to_socket_addrs()
            .map_err(|e| RigError::Io(format!("{endpoint}: {e}")))?
            .next()
            .ok_or_else(|| RigError::Io(format!("{endpoint}: no address")))?;
        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
            .map_err(|e| RigError::Io(format!("{endpoint}: {e}")))?;
        // `rigctld` finishes a command with `RPRT`. A dead session that keeps
        // the TCP connection open has no other completion signal.
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|e| RigError::Io(e.to_string()))?;
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(|e| RigError::Io(e.to_string()))?;
        stream
            .set_nodelay(true)
            .map_err(|e| RigError::Io(e.to_string()))?;

        let dump = transact(&mut stream, "dump_caps")?;
        if dump.rprt != 0 {
            return Err(RigError::Remote(format!("dump_caps RPRT {}", dump.rprt)));
        }
        let caps = parse_caps(&dump.lines);
        Ok(Self {
            stream,
            endpoint,
            caps,
            skip: BTreeSet::new(),
            ant: 1,
            message: String::new(),
        })
    }

    fn exchange(&mut self, body: &str) -> Result<Reply, RigError> {
        transact(&mut self.stream, body)
    }

    /// Read the paired VFO without leaving the rig on it.
    ///
    /// `rigctld` was not started with `-o`, so `get_freq VFOB` stays on the
    /// current VFO. Selecting the pair and selecting the original back is the
    /// hamlib command that actually returns the other frequency. This runs
    /// inside one poll, before the rig thread applies another window command.
    fn read_other_vfo(&mut self, current: &str) -> Result<(String, f64, String), RigError> {
        let Some(other) = crate::other_vfo_name(current, &self.caps.vfos) else {
            return Ok((String::new(), 0.0, String::new()));
        };
        let switched = self.exchange(&format!("set_vfo {other}"))?;
        if switched.rprt != 0 {
            return Ok((String::new(), 0.0, String::new()));
        }
        let read = (|| -> Result<(f64, String), RigError> {
            let freq_reply = self.exchange("get_freq")?;
            let freq = if freq_reply.rprt == 0 {
                labeled(&freq_reply.lines, "Frequency")
                    .and_then(|text| text.parse().ok())
                    .unwrap_or(0.0)
            } else {
                0.0
            };
            let mode_reply = self.exchange("get_mode")?;
            let mode = if mode_reply.rprt == 0 {
                labeled(&mode_reply.lines, "Mode").unwrap_or("").to_string()
            } else {
                String::new()
            };
            Ok((freq, mode))
        })();
        let restored = self.exchange(&format!("set_vfo {current}"));
        if let Err(err) = &restored {
            if matches!(err, RigError::Io(_)) {
                return Err(RigError::Io(err.to_string()));
            }
        }
        if !matches!(restored, Ok(reply) if reply.rprt == 0) {
            let again = self.exchange(&format!("set_vfo {current}"))?;
            if again.rprt != 0 {
                return Err(RigError::Remote(format!(
                    "set_vfo {current} RPRT {}",
                    again.rprt
                )));
            }
        }
        let (freq, mode) = read?;
        Ok((other, freq, mode))
    }
}

impl Rig for LiveRig {
    fn apply(&mut self, cmd: &Command) -> Result<(), RigError> {
        let body = cmd.cat_line();
        let reply = self.exchange(&body)?;
        if reply.rprt != 0 {
            let msg = format!("{body} RPRT {}", reply.rprt);
            self.message = msg.clone();
            return Err(RigError::Remote(msg));
        }
        if let Command::SetAnt(n) = cmd {
            self.ant = *n;
        }
        self.message.clear();
        Ok(())
    }

    fn poll(&mut self) -> Result<Snapshot, RigError> {
        let freq = match labeled(&self.exchange("get_freq")?.lines, "Frequency") {
            Some(text) => text
                .parse::<f64>()
                .map_err(|_| RigError::Remote(format!("bad frequency {text}")))?,
            None => return Err(RigError::Remote("get_freq had no Frequency line".into())),
        };
        let mode_reply = self.exchange("get_mode")?;
        let mode = labeled(&mode_reply.lines, "Mode")
            .unwrap_or("USB")
            .to_string();
        let passband_hz = labeled(&mode_reply.lines, "Passband")
            .and_then(|s| s.parse().ok())
            .unwrap_or(2400);
        let vfo = self
            .exchange("get_vfo")
            .ok()
            .and_then(|r| labeled(&r.lines, "VFO").map(str::to_string))
            .unwrap_or_else(|| "VFOA".into());
        let rit_hz = self
            .exchange("get_rit")
            .ok()
            .and_then(|r| labeled(&r.lines, "RIT").and_then(|s| s.parse().ok()))
            .unwrap_or(0);

        let mut levels = BTreeMap::new();
        let mut strength_db = -54.0;
        for name in READ_LEVELS {
            if self.skip.contains(*name) || !self.caps.get_levels.contains_key(*name) {
                continue;
            }
            match self.exchange(&format!("get_level {name}")) {
                Ok(reply) if reply.rprt == 0 => {
                    if let Some(value) = bare_number(&reply.lines) {
                        if *name == "STRENGTH" {
                            strength_db = value;
                        }
                        levels.insert((*name).to_string(), value);
                    }
                }
                Ok(_) => {
                    self.skip.insert((*name).to_string());
                }
                Err(RigError::Io(err)) => return Err(RigError::Io(err)),
                Err(RigError::Remote(_)) => {
                    self.skip.insert((*name).to_string());
                }
            }
        }

        let mut funcs = BTreeMap::new();
        for name in READ_FUNCS {
            if self.skip.contains(*name) || !self.caps.set_funcs.contains(*name) {
                continue;
            }
            match self.exchange(&format!("get_func {name}")) {
                Ok(reply) if reply.rprt == 0 => {
                    funcs.insert(
                        (*name).to_string(),
                        bare_number(&reply.lines).unwrap_or(0.0) != 0.0,
                    );
                }
                Ok(_) => {
                    self.skip.insert((*name).to_string());
                }
                Err(RigError::Io(err)) => return Err(RigError::Io(err)),
                Err(RigError::Remote(_)) => {
                    self.skip.insert((*name).to_string());
                }
            }
        }

        let ptt = self
            .exchange("get_ptt")
            .ok()
            .and_then(|r| labeled(&r.lines, "PTT").map(|s| s.trim() != "0"))
            .unwrap_or(false);
        let power_on = self
            .exchange("get_powerstat")
            .ok()
            .and_then(|r| labeled(&r.lines, "Power Status").map(|s| s.trim() != "0"))
            .unwrap_or(true);

        let (sub_vfo, sub_freq_hz, sub_mode) = self.read_other_vfo(&vfo)?;
        let (sub_mode, sub_freq_hz, sub_vfo, tx_vfo) = if self.caps.can_set_split {
            let split_freq = self
                .exchange("get_split_freq")
                .ok()
                .and_then(|r| {
                    if r.rprt == 0 {
                        labeled(&r.lines, "TX Frequency").and_then(|s| s.parse().ok())
                    } else {
                        None
                    }
                })
                .unwrap_or(0.0);
            let reply = self.exchange("get_split_vfo").ok();
            let split_on = reply
                .as_ref()
                .and_then(|r| labeled(&r.lines, "Split"))
                .is_some_and(|s| s.trim() == "1");
            let tx = if split_on {
                reply
                    .as_ref()
                    .and_then(|r| labeled(&r.lines, "TX VFO"))
                    .unwrap_or("VFOB")
                    .to_string()
            } else {
                vfo.clone()
            };
            let split_mode = self
                .exchange("get_split_mode")
                .ok()
                .and_then(|r| {
                    if r.rprt == 0 {
                        labeled(&r.lines, "TX Mode")
                            .or_else(|| labeled(&r.lines, "Split Mode"))
                            .or_else(|| labeled(&r.lines, "Mode"))
                            .map(str::to_string)
                    } else {
                        None
                    }
                })
                .unwrap_or_default();
            // The paired VFO is the readout. Split is only a fallback when
            // selecting that VFO failed and hamlib still has a distinct TX VFO.
            if sub_vfo.is_empty() && !tx.eq_ignore_ascii_case(&vfo) && split_freq > 0.0 {
                (split_mode, split_freq, tx.clone(), tx)
            } else {
                (sub_mode, sub_freq_hz, sub_vfo, tx)
            }
        } else {
            (sub_mode, sub_freq_hz, sub_vfo, vfo.clone())
        };

        Ok(Snapshot {
            kind: LinkKind::Live,
            model: self.caps.model.clone(),
            endpoint: self.endpoint.clone(),
            message: self.message.clone(),
            last_cat: String::new(),
            freq_hz: freq,
            mode,
            passband_hz,
            sub_mode,
            sub_freq_hz,
            sub_vfo,
            vfo,
            tx_vfo,
            rit_hz,
            ptt,
            power_on,
            ant: self.ant,
            strength_db,
            levels,
            funcs,
            caps: self.caps.clone(),
            roofing: None,
        })
    }
}

fn transact(stream: &mut TcpStream, body: &str) -> Result<Reply, RigError> {
    let line = format!("+\\{body}\n");
    stream
        .write_all(line.as_bytes())
        .map_err(|e| RigError::Io(e.to_string()))?;
    read_reply(stream)
}

fn read_reply(stream: &mut TcpStream) -> Result<Reply, RigError> {
    let mut acc = Vec::new();
    let mut lines = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        while let Some(pos) = acc.iter().position(|b| *b == b'\n') {
            let line = String::from_utf8_lossy(&acc[..pos]).trim().to_string();
            acc.drain(..=pos);
            if let Some(code) = line.strip_prefix("RPRT") {
                let rprt = code.trim().parse::<i32>().unwrap_or(-1);
                return Ok(Reply { rprt, lines });
            }
            if !line.is_empty() {
                lines.push(line);
            }
        }
        if Instant::now() >= deadline {
            return Err(RigError::Io("rigctld did not answer".into()));
        }
        let mut buf = [0u8; 4096];
        match stream.read(&mut buf) {
            Ok(0) => return Err(RigError::Io("rigctld closed the connection".into())),
            Ok(n) => {
                acc.extend_from_slice(&buf[..n]);
                if acc.len() > 2_000_000 {
                    return Err(RigError::Io("response too long".into()));
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                return Err(RigError::Io("rigctld did not answer".into()));
            }
            Err(e) => return Err(RigError::Io(e.to_string())),
        }
    }
}

fn labeled<'a>(lines: &'a [String], key: &str) -> Option<&'a str> {
    let prefix = format!("{key}:");
    lines.iter().find_map(|line| {
        line.trim()
            .strip_prefix(&prefix)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    })
}

fn bare_number(lines: &[String]) -> Option<f64> {
    lines.iter().rev().find_map(|line| {
        let line = line.trim();
        if line.contains(':') {
            None
        } else {
            line.parse().ok()
        }
    })
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::*;
    use crate::Command;

    fn serve() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            let mut freq = 145_000_000_i64;
            let (mut sock, _) = listener.accept().unwrap();
            sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut acc = Vec::new();
            loop {
                let mut buf = [0u8; 1024];
                let n = match sock.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => n,
                };
                acc.extend_from_slice(&buf[..n]);
                while let Some(pos) = acc.iter().position(|b| *b == b'\n') {
                    let line = String::from_utf8_lossy(&acc[..pos]).trim().to_string();
                    acc.drain(..=pos);
                    let reply = script(&line, &mut freq);
                    let _ = sock.write_all(reply.as_bytes());
                }
            }
        });
        addr.to_string()
    }

    fn script(line: &str, freq: &mut i64) -> String {
        let body = line.trim_start_matches("+\\").trim_start_matches('+');
        if body == "dump_caps" {
            return "\
dump_caps:
Model name:\tDummy
Set level: RF(0.000000..1.000000/0.010000) RFPOWER(0.050000..1.000000/0.010000) AF(0.000000..0.000000/0.000000)
Get level: RF(0.000000..1.000000/0.010000) STRENGTH(0..0/0) RFPOWER(0.050000..1.000000/0.010000)
Set functions: NR LOCK
Mode list: USB LSB FM
VFO list: VFOA VFOB
VFO Ops: XCHG CPY
AGC levels: 0=OFF 5=MEDIUM
Can set Frequency:\tY
Can set Mode:\tY
Can set Ant:\tN
Can set PTT:\tY
Can set RIT:\tY
Can set Power Stat:\tY
Can set Split VFO:\tN
RPRT 0
"
            .into();
        }
        if body == "get_freq" {
            return format!("get_freq:\nFrequency: {freq}\nRPRT 0\n");
        }
        if let Some(rest) = body.strip_prefix("set_freq ") {
            *freq = rest.trim().parse().unwrap_or(*freq);
            return format!("set_freq: {rest}\nRPRT 0\n");
        }
        if body == "get_mode" {
            return "get_mode:\nMode: USB\nPassband: 2400\nRPRT 0\n".into();
        }
        if body.starts_with("set_mode ") {
            return format!("{body}\nRPRT 0\n");
        }
        if body == "get_vfo" {
            return "get_vfo:\nVFO: VFOA\nRPRT 0\n".into();
        }
        if body == "get_rit" {
            return "get_rit:\nRIT: 0\nRPRT 0\n".into();
        }
        if body == "get_level STRENGTH" {
            return "get_level: STRENGTH\n-12\nRPRT 0\n".into();
        }
        if body == "get_level RF" {
            return "get_level: RF\n0.5\nRPRT 0\n".into();
        }
        if body == "get_level RFPOWER" {
            return "get_level: RFPOWER\n0.2\nRPRT 0\n".into();
        }
        if body.starts_with("get_func ") {
            return format!("{body}\n0\nRPRT 0\n");
        }
        if body == "get_ptt" {
            return "get_ptt:\nPTT: 0\nRPRT 0\n".into();
        }
        if body == "get_powerstat" {
            return "get_powerstat:\nPower Status: 1\nRPRT 0\n".into();
        }
        if body.starts_with("set_") || body.starts_with("vfo_op") {
            return format!("{body}\nRPRT 0\n");
        }
        "RPRT -1\n".into()
    }

    #[test]
    fn fake_server_sets_frequency_and_hides_degenerate_levels() {
        let addr = serve();
        let (host, port) = addr.rsplit_once(':').unwrap();
        let port: u16 = port.parse().unwrap();
        let mut rig = LiveRig::connect(host, port).unwrap();
        let snap = rig.poll().unwrap();
        assert_eq!(snap.model, "Dummy");
        assert!((snap.freq_hz - 145_000_000.0).abs() < 0.5);
        assert_eq!(snap.mode, "USB");
        assert!((snap.strength_db - -12.0).abs() < 0.1);
        assert!(snap.caps.level_range("RF").is_some());
        assert!(snap.caps.level_range("AF").is_none());
        rig.apply(&Command::SetFreq(14_153_608.0)).unwrap();
        let snap = rig.poll().unwrap();
        assert!((snap.freq_hz - 14_153_608.0).abs() < 0.5);
    }

    #[test]
    fn hamlib_dummy_when_rigctld_is_installed() {
        let port = 4577_u16;
        let child = std::process::Command::new("rigctld")
            .args(["-m", "1", "-T", "127.0.0.1", "-t", &port.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(_) => return,
        };
        let mut connected = None;
        // A refused connect returns at once. rigctld publishes no earlier
        // ready signal than the listen socket, so the pause is the wait
        // between those refusals.
        for _ in 0..40 {
            if let Ok(rig) = LiveRig::connect("127.0.0.1", port) {
                connected = Some(rig);
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let mut rig = match connected {
            Some(rig) => rig,
            None => {
                let _ = child.kill();
                panic!("rigctld did not accept a connection");
            }
        };
        rig.apply(&Command::SetVfo("VFOA".into())).unwrap();
        rig.apply(&Command::SetFreq(14_153_608.0)).unwrap();
        rig.apply(&Command::SetMode {
            mode: "USB".into(),
            passband_hz: 2400,
        })
        .unwrap();
        rig.apply(&Command::SetVfo("VFOB".into())).unwrap();
        rig.apply(&Command::SetFreq(146_520_000.0)).unwrap();
        rig.apply(&Command::SetMode {
            mode: "FM".into(),
            passband_hz: 15000,
        })
        .unwrap();
        rig.apply(&Command::SetVfo("VFOA".into())).unwrap();
        let snap = rig.poll().unwrap();
        let _ = child.kill();
        assert!(
            (snap.freq_hz - 14_153_608.0).abs() < 1.0,
            "{}",
            snap.freq_hz
        );
        assert_eq!(snap.mode, "USB");
        assert_eq!(snap.vfo, "VFOA");
        assert_eq!(snap.sub_vfo, "VFOB");
        assert!((snap.sub_freq_hz - 146_520_000.0).abs() < 1.0);
        assert_eq!(snap.sub_mode, "FM");
        assert!(
            snap.caps.level_range("AF").is_none(),
            "dummy AF is degenerate"
        );
        assert!(snap.caps.level_range("RFPOWER").is_some());
        assert_eq!(snap.caps.rx_ranges.len(), 1);
        assert!((snap.caps.rx_ranges[0].min - 150_000.0).abs() < 0.5);
        assert!((snap.caps.rx_ranges[0].max - 1_500_000_000.0).abs() < 0.5);
    }
}
