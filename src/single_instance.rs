//! Single instance: the first walle listens on a per-user localhost port; later launches
//! ask it to show its window and exit.
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    sync::{Arc, Mutex},
    time::Duration,
};

const MAGIC: &[u8] = b"walle:show\n";
const ACK: &[u8] = b"walle:ok\n";

fn port() -> u16 {
    let user = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default();
    // An isolated run is its own instance.
    let name = match crate::config::Paths::isolated_home() {
        Some(home) => format!("walle-{user}-{}", home.display()),
        None => format!("walle-{user}"),
    };
    40000 + (crate::net::fnv64(&name) % 20000) as u16
}

/// Callback invoked when another instance asks us to show the window.
pub type OnShow = Arc<Mutex<Option<Box<dyn Fn() + Send>>>>;

pub enum Claim {
    /// We are the first instance; set the callback once the UI exists.
    Primary(OnShow),
    /// Another instance was told to show itself; this process should exit.
    Secondary,
}

pub fn claim() -> Claim {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port()));
    match TcpListener::bind(addr) {
        Ok(listener) => {
            let on_show: OnShow = Arc::new(Mutex::new(None));
            let cb = on_show.clone();
            std::thread::Builder::new()
                .name("walle-ipc".into())
                .spawn(move || {
                    for mut stream in listener.incoming().flatten() {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let mut buf = [0u8; 32];
                        let n = stream.read(&mut buf).unwrap_or(0);
                        if &buf[..n] == MAGIC {
                            let _ = stream.write_all(ACK);
                            if let Some(f) = cb.lock().unwrap().as_ref() {
                                f();
                            }
                        }
                    }
                })
                .ok();
            Claim::Primary(on_show)
        }
        Err(_) => {
            // Port taken: probably walle. If it answers, it's the other instance.
            match TcpStream::connect_timeout(&addr, Duration::from_millis(500)) {
                Ok(mut s) => {
                    if s.write_all(MAGIC).is_err() {
                        return Claim::Primary(Arc::new(Mutex::new(None)));
                    }
                    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut buf = [0u8; 16];
                    let n = s.read(&mut buf).unwrap_or(0);
                    if &buf[..n] == ACK { Claim::Secondary } else { Claim::Primary(Arc::new(Mutex::new(None))) }
                }
                // Something else owns the port; run without single-instance support.
                _ => Claim::Primary(Arc::new(Mutex::new(None))),
            }
        }
    }
}
