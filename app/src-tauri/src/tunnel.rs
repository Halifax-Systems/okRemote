//! Shared SSH transports, forwards and health monitoring (spec section 11).
use crate::ssh::{connect_handle, Handler, RemoteMap, Resolved};
use crate::state::*;
use ok_model::Forward;
use russh::client::Handle;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::Emitter;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{watch, RwLock};

#[derive(Clone, Debug)]
pub enum Health { Idle, Connecting, Up, Reconnecting(u32, String), Failed(String) }

impl Health {
    fn name(&self) -> &'static str {
        match self { Health::Idle => "idle", Health::Connecting => "connecting", Health::Up => "up", Health::Reconnecting(..) => "reconnecting", Health::Failed(_) => "failed" }
    }
}

pub struct Transport {
    pub key: String,
    pub label: String,
    chain: Vec<Resolved>,
    hops: RwLock<Vec<Arc<Handle<Handler>>>>,
    health: Mutex<Health>,
    users: AtomicUsize,
    idle_since: Mutex<Option<Instant>>,
    remote: RemoteMap,
    remote_fwds: Mutex<Vec<(String, u32, (String, u16))>>,
    connect_lock: tokio::sync::Mutex<()>,
    reconnecting: AtomicBool,
}

impl Transport {
    pub fn health(&self) -> Health { self.health.lock().unwrap().clone() }
    pub async fn last(&self) -> Option<Arc<Handle<Handler>>> {
        let h = self.hops.read().await.last().cloned()?;
        (!h.is_closed()).then_some(h)
    }
}

/// Keeps a transport alive while held.
pub struct Lease { t: Arc<Transport>, state: AppState }

impl Lease {
    fn new(t: &Arc<Transport>, state: &AppState) -> Lease {
        t.users.fetch_add(1, Ordering::SeqCst);
        *t.idle_since.lock().unwrap() = None;
        changed(state);
        Lease { t: t.clone(), state: state.clone() }
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if self.t.users.fetch_sub(1, Ordering::SeqCst) == 1 { *self.t.idle_since.lock().unwrap() = Some(Instant::now()); }
        changed(&self.state);
    }
}

pub struct Fwd {
    id: String, label: String, kind: &'static str, listen: String, target: String, transport: Arc<Transport>, owner: String, auto: bool,
    active: Arc<AtomicUsize>, cancel: watch::Sender<bool>, remote: Option<(String, u32)>, _lease: Lease,
}

#[derive(Default)]
pub struct Manager {
    transports: Mutex<HashMap<String, Arc<Transport>>>,
    forwards: Mutex<HashMap<String, Fwd>>,
}

pub fn changed(state: &AppState) { let _ = state.app.emit("tunnels-changed", ()); }

fn set_health(state: &AppState, t: &Transport, h: Health) { *t.health.lock().unwrap() = h; changed(state); }

async fn connect_chain(state: &AppState, t: &Arc<Transport>) -> CmdResult<()> {
    let _g = t.connect_lock.lock().await;
    if t.last().await.is_some() { return Ok(()); }
    set_health(state, t, Health::Connecting);
    let mut hops: Vec<Arc<Handle<Handler>>> = vec![];
    for (i, hop) in t.chain.iter().enumerate() {
        let remote = (i == t.chain.len() - 1).then(|| t.remote.clone());
        match connect_handle(state, hop, hops.last().map(|h| h.as_ref()), remote).await {
            Ok(h) => hops.push(Arc::new(h)),
            Err(e) => { set_health(state, t, Health::Failed(e.message.clone())); return Err(e); }
        }
    }
    // re-request remote forwards after a (re)connect
    if let Some(last) = hops.last() {
        let fw = t.remote_fwds.lock().unwrap().clone();
        for (addr, port, _) in fw { let _ = last.tcpip_forward(addr, port).await; }
    }
    *t.hops.write().await = hops;
    set_health(state, t, Health::Up);
    Ok(())
}

pub async fn transport_for(state: &AppState, ws: &str, chain: Vec<(String, Resolved)>) -> CmdResult<Arc<Transport>> {
    if chain.is_empty() { return Err(CmdErr::new("tunnel", "the tunnel profile has no SSH hops")); }
    let key = format!("{ws}:{}", chain.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>().join(","));
    let t = {
        let mut m = state.tunnels.transports.lock().unwrap();
        m.entry(key.clone()).or_insert_with(|| Arc::new(Transport {
            key: key.clone(), label: chain.iter().map(|(_, r)| r.title.clone()).collect::<Vec<_>>().join(" → "),
            chain: chain.iter().map(|(_, r)| r.clone()).collect(), hops: RwLock::new(vec![]), health: Mutex::new(Health::Idle),
            users: AtomicUsize::new(0), idle_since: Mutex::new(None), remote: Default::default(), remote_fwds: Default::default(),
            connect_lock: Default::default(), reconnecting: AtomicBool::new(false),
        })).clone()
    };
    connect_chain(state, &t).await?;
    Ok(t)
}

pub fn lease(state: &AppState, t: &Arc<Transport>) -> Lease { Lease::new(t, state) }

pub async fn open_via(t: &Transport, host: &str, port: u16) -> Result<russh::Channel<russh::client::Msg>, u8> {
    let h = t.last().await.ok_or(1u8)?;
    h.channel_open_direct_tcpip(host.to_string(), port as u32, "127.0.0.1", 0).await.map_err(|_| 5u8)
}

pub async fn start_forward(state: &AppState, t: &Arc<Transport>, spec: &Forward, owner: &str, auto: bool, label: &str, allow_exposed: bool) -> CmdResult<(String, u16)> {
    if spec.exposes() && !allow_exposed { return Err(CmdErr::new("tunnel", "this forward is reachable from the network and needs confirmation")); }
    let id = ok_crypto::random_token();
    let (cancel, rx) = watch::channel(false);
    let active = Arc::new(AtomicUsize::new(0));
    let (kind, listen, target, bound, remote) = match spec {
        Forward::Local { bind_addr, bind_port, remote_host, remote_port } => {
            let (l, addr, port) = listen(bind_addr, *bind_port).await?;
            let target = Some((remote_host.clone(), *remote_port));
            tokio::spawn(accept_loop(l, t.clone(), target, active.clone(), rx));
            ("local", format!("{addr}:{port}"), format!("{remote_host}:{remote_port}"), port, None)
        }
        Forward::Dynamic { bind_addr, bind_port } => {
            let (l, addr, port) = listen(bind_addr, *bind_port).await?;
            tokio::spawn(accept_loop(l, t.clone(), None, active.clone(), rx));
            ("dynamic", format!("{addr}:{port}"), "SOCKS5".to_string(), port, None)
        }
        Forward::Remote { remote_bind_addr, remote_bind_port, local_host, local_port } => {
            let addr = if remote_bind_addr.is_empty() { "127.0.0.1" } else { remote_bind_addr };
            let h = t.last().await.ok_or_else(|| CmdErr::new("tunnel", "the SSH transport is down"))?;
            let port = h.tcpip_forward(addr.to_string(), *remote_bind_port as u32).await.map_err(|_| CmdErr::new("tunnel", "server refused remote forward"))?;
            let port = if *remote_bind_port == 0 { port } else { *remote_bind_port as u32 };
            t.remote.lock().unwrap().insert(port, (local_host.clone(), *local_port));
            t.remote_fwds.lock().unwrap().push((addr.to_string(), port, (local_host.clone(), *local_port)));
            ("remote", format!("{addr}:{port} (server)"), format!("{local_host}:{local_port}"), port as u16, Some((addr.to_string(), port)))
        }
    };
    let fwd = Fwd { id: id.clone(), label: label.into(), kind, listen, target, transport: t.clone(), owner: owner.into(), auto, active, cancel, remote, _lease: Lease::new(t, state) };
    state.tunnels.forwards.lock().unwrap().insert(id.clone(), fwd);
    changed(state);
    Ok((id, bound))
}

async fn listen(bind_addr: &str, port: u16) -> CmdResult<(TcpListener, String, u16)> {
    let addr = if bind_addr.is_empty() { "127.0.0.1" } else { bind_addr };
    let l = TcpListener::bind((addr, port)).await.map_err(|e| CmdErr::new("tunnel", format!("could not listen on {addr}:{port}: {e}")))?;
    let bound = l.local_addr().map(|a| a.port()).unwrap_or(port);
    Ok((l, addr.to_string(), bound))
}

async fn accept_loop(l: TcpListener, t: Arc<Transport>, target: Option<(String, u16)>, active: Arc<AtomicUsize>, mut cancel: watch::Receiver<bool>) {
    loop {
        tokio::select! {
            _ = cancel.changed() => break,
            r = l.accept() => {
                let Ok((sock, _)) = r else { continue };
                let _ = sock.set_nodelay(true);
                let (t, target, active, c) = (t.clone(), target.clone(), active.clone(), cancel.clone());
                tokio::spawn(async move {
                    active.fetch_add(1, Ordering::SeqCst);
                    handle_conn(sock, &t, target, c).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                });
            }
        }
    }
}

async fn handle_conn(mut sock: TcpStream, t: &Transport, target: Option<(String, u16)>, mut cancel: watch::Receiver<bool>) {
    let socks = target.is_none();
    let (host, port) = match target {
        Some(x) => x,
        None => match socks5(&mut sock).await { Some(x) => x, None => return },
    };
    let reply = |code: u8| [5, code, 0, 1, 0, 0, 0, 0, 0, 0];
    match open_via(t, &host, port).await {
        Ok(ch) => {
            if socks && sock.write_all(&reply(0)).await.is_err() { return; }
            let mut remote = ch.into_stream();
            tokio::select! { _ = tokio::io::copy_bidirectional(&mut sock, &mut remote) => {}, _ = cancel.changed() => {} }
        }
        Err(code) => { if socks { let _ = sock.write_all(&reply(code)).await; } }
    }
}

/// SOCKS5 handshake without authentication, CONNECT only. Returns the requested target; the caller sends the final reply.
async fn socks5(s: &mut TcpStream) -> Option<(String, u16)> {
    let mut hdr = [0u8; 2];
    s.read_exact(&mut hdr).await.ok()?;
    if hdr[0] != 5 { return None; }
    let mut methods = vec![0u8; hdr[1] as usize];
    s.read_exact(&mut methods).await.ok()?;
    if !methods.contains(&0) { let _ = s.write_all(&[5, 0xFF]).await; return None; }
    s.write_all(&[5, 0]).await.ok()?;
    let mut req = [0u8; 4];
    s.read_exact(&mut req).await.ok()?;
    let reply = |code: u8| [5, code, 0, 1, 0, 0, 0, 0, 0, 0];
    if req[1] != 1 { let _ = s.write_all(&reply(7)).await; return None; }
    let host = match req[3] {
        1 => { let mut a = [0u8; 4]; s.read_exact(&mut a).await.ok()?; std::net::Ipv4Addr::from(a).to_string() }
        3 => { let mut l = [0u8; 1]; s.read_exact(&mut l).await.ok()?; let mut n = vec![0u8; l[0] as usize]; s.read_exact(&mut n).await.ok()?; String::from_utf8(n).ok()? }
        4 => { let mut a = [0u8; 16]; s.read_exact(&mut a).await.ok()?; std::net::Ipv6Addr::from(a).to_string() }
        _ => { let _ = s.write_all(&reply(8)).await; return None; }
    };
    let mut p = [0u8; 2];
    s.read_exact(&mut p).await.ok()?;
    Some((host, u16::from_be_bytes(p)))
}

pub async fn stop_forward(state: &AppState, id: &str) {
    let f = state.tunnels.forwards.lock().unwrap().remove(id);
    if let Some(f) = f {
        let _ = f.cancel.send(true);
        if let Some((addr, port)) = &f.remote {
            f.transport.remote.lock().unwrap().remove(port);
            f.transport.remote_fwds.lock().unwrap().retain(|(a, p, _)| !(a == addr && p == port));
            if let Some(h) = f.transport.last().await { let _ = h.cancel_tcpip_forward(addr.clone(), *port).await; }
        }
    }
    changed(state);
}

pub async fn stop_owner(state: &AppState, owner: &str) {
    let ids: Vec<String> = state.tunnels.forwards.lock().unwrap().values().filter(|f| f.owner == owner).map(|f| f.id.clone()).collect();
    for id in ids { stop_forward(state, &id).await; }
}

pub fn owner_running(state: &AppState, owner: &str) -> bool { state.tunnels.forwards.lock().unwrap().values().any(|f| f.owner == owner) }

pub fn status(state: &AppState) -> Value {
    let forwards: Vec<Value> = state.tunnels.forwards.lock().unwrap().values().map(|f| {
        let h = f.transport.health();
        json!({"id": f.id, "label": f.label, "kind": f.kind, "listen": f.listen, "target": f.target, "transport_key": f.transport.key,
            "transport_label": f.transport.label, "health": h.name(), "error": match &h { Health::Failed(e) | Health::Reconnecting(_, e) => Some(e.clone()), _ => None },
            "retry": if let Health::Reconnecting(n, _) = h { Some(n) } else { None },
            "active": f.active.load(Ordering::SeqCst), "auto": f.auto, "owner": f.owner })
    }).collect();
    let transports: Vec<Value> = state.tunnels.transports.lock().unwrap().values().map(|t| {
        let h = t.health();
        json!({"key": t.key, "label": t.label, "health": h.name(), "users": t.users.load(Ordering::SeqCst),
            "error": match &h { Health::Failed(e) | Health::Reconnecting(_, e) => Some(e.clone()), _ => None } })
    }).collect();
    json!({"forwards": forwards, "transports": transports})
}

/// Wakes every 3 s: reconnect dropped transports that have users, close idle ones.
pub fn start_monitor(state: AppState) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;
            let list: Vec<Arc<Transport>> = state.tunnels.transports.lock().unwrap().values().cloned().collect();
            for t in list {
                let users = t.users.load(Ordering::SeqCst);
                let up = matches!(t.health(), Health::Up);
                if users == 0 {
                    let idle = t.idle_since.lock().unwrap().map_or(false, |i| i.elapsed() > Duration::from_secs(60));
                    if up && idle { t.hops.write().await.clear(); set_health(&state, &t, Health::Idle); }
                } else if up && t.last().await.is_none() && !t.reconnecting.swap(true, Ordering::SeqCst) {
                    let (st, t2) = (state.clone(), t.clone());
                    tokio::spawn(async move {
                        for n in 1..=10u32 {
                            set_health(&st, &t2, Health::Reconnecting(n, "connection lost".into()));
                            tokio::time::sleep(Duration::from_secs(2u64.pow(n.min(6)).min(60))).await;
                            if t2.users.load(Ordering::SeqCst) == 0 { set_health(&st, &t2, Health::Idle); t2.reconnecting.store(false, Ordering::SeqCst); return; }
                            match connect_chain(&st, &t2).await {
                                Ok(()) => { t2.reconnecting.store(false, Ordering::SeqCst); return; }
                                Err(e) => set_health(&st, &t2, Health::Reconnecting(n, e.message)),
                            }
                        }
                        set_health(&st, &t2, Health::Failed("gave up after 10 attempts".into()));
                        t2.reconnecting.store(false, Ordering::SeqCst);
                    });
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pair() -> (TcpStream, TcpStream) {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let c = TcpStream::connect(l.local_addr().unwrap()).await.unwrap();
        (c, l.accept().await.unwrap().0)
    }

    #[tokio::test]
    async fn socks5_connect_domain_and_ipv4() {
        let (mut client, mut server) = pair().await;
        let task = tokio::spawn(async move { socks5(&mut server).await });
        client.write_all(&[5, 1, 0]).await.unwrap();
        let mut m = [0u8; 2];
        client.read_exact(&mut m).await.unwrap();
        assert_eq!(m, [5, 0]);
        let mut req = vec![5, 1, 0, 3, 11];
        req.extend_from_slice(b"example.com");
        req.extend_from_slice(&443u16.to_be_bytes());
        client.write_all(&req).await.unwrap();
        assert_eq!(task.await.unwrap(), Some(("example.com".to_string(), 443)));

        let (mut client, mut server) = pair().await;
        let task = tokio::spawn(async move { socks5(&mut server).await });
        client.write_all(&[5, 1, 0, 5, 1, 0, 1, 10, 0, 0, 7, 0, 80]).await.unwrap();
        let mut m = [0u8; 2];
        client.read_exact(&mut m).await.unwrap();
        assert_eq!(task.await.unwrap(), Some(("10.0.0.7".to_string(), 80)));
    }

    #[tokio::test]
    async fn socks5_rejects_bad_method_and_command() {
        let (mut client, mut server) = pair().await;
        let task = tokio::spawn(async move { socks5(&mut server).await });
        client.write_all(&[5, 1, 2]).await.unwrap();
        let mut m = [0u8; 2];
        client.read_exact(&mut m).await.unwrap();
        assert_eq!(m, [5, 0xFF]);
        assert_eq!(task.await.unwrap(), None);

        let (mut client, mut server) = pair().await;
        let task = tokio::spawn(async move { socks5(&mut server).await });
        client.write_all(&[5, 1, 0, 5, 2, 0, 1]).await.unwrap();
        let mut buf = [0u8; 12];
        client.read_exact(&mut buf[..2]).await.unwrap();
        client.read_exact(&mut buf[..10]).await.unwrap();
        assert_eq!(buf[1], 7);
        assert_eq!(task.await.unwrap(), None);
    }
}
