//! Monogram's Lightroom protocol on 127.0.0.1:50111 (`LIGHTCRAFT_MONOGRAM_PORT`; `0` = off).
//!
//! Monogram Creator's service talks to Lightroom's Monogram plugin over a TCP line protocol
//! (reverse-engineered from its service, see `~/.hammerspoon/devices/re/MONOGRAM.md`): one UTF-8
//! line per action — `Exposure = 128,-10,10` (slider), `Exposure = ^0.05,-10,10` (dial detents,
//! Monogram's own step), `Exposure = reset`, `set_preset = Name`, `flag = pick`… With LightCraft
//! registered as a Monogram integration that uses this protocol, the console drives LightCraft the
//! way it drives Lightroom: whatever app is in front, with Monogram's increments. Each line becomes
//! the UI command `keys.lrinput` on the UI thread; nothing is written back (the plugin's replies
//! aren't used by Monogram).

use std::io::{BufRead, BufReader, Read};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::Sender;

use lightcraft_ui_egui::ControlRequest;
use serde_json::json;

/// Longest accepted line; Monogram's are tens of bytes.
const MAX_LINE: usize = 4096;

pub fn start(port: u16, tx: Sender<ControlRequest>, ctx: egui::Context) {
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            log::warn!("Monogram listener failed to bind 127.0.0.1:{port}: {e}");
            return;
        }
    };
    log::info!("Monogram listener on 127.0.0.1:{port}");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (tx, ctx) = (tx.clone(), ctx.clone());
            std::thread::spawn(move || serve(stream, &tx, &ctx));
        }
    });
}

fn serve(stream: TcpStream, tx: &Sender<ControlRequest>, ctx: &egui::Context) {
    for line in lines(BufReader::new(stream)) {
        let (req, _reply) = ControlRequest::new(String::from("engine.execute"), json!({"command": "keys.lrinput", "params": {"line": line}}));
        if tx.send(req).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

/// The non-empty lines of `r` (each at most `MAX_LINE` bytes; a longer one ends the connection).
fn lines<R: BufRead>(mut r: R) -> impl Iterator<Item = String> {
    std::iter::from_fn(move || {
        loop {
            let mut buf = Vec::new();
            let n = std::io::Read::by_ref(&mut r).take(MAX_LINE as u64 + 1).read_until(b'\n', &mut buf).ok()?;
            if n == 0 || (buf.len() > MAX_LINE && !buf.ends_with(b"\n")) {
                return None;
            }
            let s = String::from_utf8_lossy(&buf).trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_monogram_lines_and_refuses_overlong_ones() {
        let got: Vec<String> = lines(&b"Exposure = ^0.05,-10,10\n\nset_preset = BH - NMD\r\n"[..]).collect();
        assert_eq!(got, vec!["Exposure = ^0.05,-10,10", "set_preset = BH - NMD"]);
        let long = vec![b'a'; MAX_LINE + 10];
        assert_eq!(lines(&long[..]).count(), 0);
    }
}
