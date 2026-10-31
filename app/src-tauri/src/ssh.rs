//! Direct SSH sessions: host-key trust, authentication, shell channel (spec section 8).
use crate::state::*;
use ok_model::*;
use russh::client::{self, KeyboardInteractiveAuthResponse};
use russh::keys::{self, HashAlg, PrivateKeyWithHashAlg};
use russh::{ChannelMsg, Disconnect};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tauri::ipc::{Channel, InvokeResponseBody};
use tokio::sync::mpsc;

pub type RemoteMap = Arc<Mutex<std::collections::HashMap<u32, (String, u16)>>>;

pub struct Handler {
    remote: Option<RemoteMap>,
    state: AppState,
    ws: Option<String>,
    host: String,
    port: u16,
    label: String,
    failure: Arc<Mutex<Option<CmdErr>>>,
}

impl client::Handler for Handler {
    type Error = russh::Error;

    async fn server_channel_open_forwarded_tcpip(&mut self, channel: russh::Channel<client::Msg>, _addr: &str, port: u32, _oa: &str, _op: u32,
        reply: russh::client::ChannelOpenHandle, _session: &mut client::Session) -> Result<(), Self::Error> {
        let target = self.remote.as_ref().and_then(|m| m.lock().unwrap().get(&port).cloned());
        match target {
            Some((host, p)) => {
                reply.accept().await;
                tokio::spawn(async move {
                    if let Ok(mut local) = tokio::net::TcpStream::connect((host.as_str(), p)).await {
                        let mut remote = channel.into_stream();
                        let _ = tokio::io::copy_bidirectional(&mut local, &mut remote).await;
                    }
                });
            }
            None => { reply.accept().await; }
        }
        Ok(())
    }

    async fn check_server_key(&mut self, key: &russh::keys::PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let pk = match key {
            russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            russh::keys::PublicKeyOrCertificate::Certificate(c) => c.signature_key().clone().into(),
        };
        let key_type = pk.algorithm().to_string();
        let fp = pk.fingerprint(HashAlg::Sha256).to_string();
        let host = self.host.to_lowercase();
        let known: Vec<HostKey> = match &self.ws {
            Some(ws) => self.state.with_vault(ws, |v| v.hostkeys()).unwrap_or_default(),
            None => self.state.device_hostkeys(),
        };
        let same_host: Vec<&HostKey> = known.iter().filter(|k| k.host == host && k.port == self.port).collect();
        if let Some(k) = same_host.iter().find(|k| k.key_type == key_type) {
            if k.fingerprint == fp { return Ok(true); }
            let _ = self.state.ask("hostkey_changed", json!({
                "host": self.host, "port": self.port, "key_type": key_type, "old": k.fingerprint, "new": fp, "label": self.label,
            })).await;
            *self.failure.lock().unwrap() = Some(CmdErr::new("host_key", format!("host key for {}:{} was not trusted", self.host, self.port)));
            return Ok(false);
        }
        let others: Vec<&str> = same_host.iter().map(|k| k.key_type.as_str()).collect();
        let answer = self.state.ask("hostkey_unknown", json!({
            "host": self.host, "port": self.port, "key_type": key_type, "fingerprint": fp,
            "other_keys": others, "label": self.label, "shared": self.ws.is_some(),
        })).await;
        match answer.as_ref().and_then(Value::as_str) {
            Some("trust") => {
                let entry = HostKey { host: host.clone(), port: self.port, key_type: key_type.clone(), fingerprint: fp.clone(),
                    first_seen: chrono::Utc::now().to_rfc3339(), trusted_by: ok_store::os_actor() };
                match &self.ws {
                    Some(ws) => { let _ = self.state.with_vault(ws, |v| v.hostkey_trust(&host, self.port, &key_type, &fp)); }
                    None => {
                        let mut all = self.state.device_hostkeys();
                        all.push(entry);
                        let _ = self.state.save_device_hostkeys(&all);
                    }
                }
                Ok(true)
            }
            Some("once") => Ok(true),
            _ => {
                *self.failure.lock().unwrap() = Some(CmdErr::new("cancelled", "Sign-in was cancelled."));
                Ok(false)
            }
        }
    }
}

#[derive(Clone)]
pub struct Resolved {
    pub title: String,
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub secret: Secret,
    pub options: Options,
    pub ws: Option<String>,
    pub conn_id: Option<String>,
    pub color: Option<String>,
}

fn opt_str(o: &Options, k: &str) -> Option<String> { o.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from) }

fn expand_home(p: &str) -> std::path::PathBuf {
    match p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")) {
        Some(rest) => dirs::home_dir().unwrap_or_default().join(rest),
        None => p.into(),
    }
}

pub struct Opened {
    pub id: String,
    pub title: String,
    pub osc52: bool,
    pub fixed_cols: Option<u32>,
    pub fixed_rows: Option<u32>,
    pub terminal_profile: Option<String>,
    pub log_file: Option<String>,
}

pub async fn connect_handle(state: &AppState, r: &Resolved, via: Option<&client::Handle<Handler>>, remote: Option<RemoteMap>) -> CmdResult<client::Handle<Handler>> {
    let o = &r.options;
    let mut user = r.user.clone().or_else(|| r.secret.username.clone());
    let mut secret = Secret { username: r.secret.username.clone(), password: r.secret.password.clone(), domain: None,
        private_key: r.secret.private_key.clone(), key_passphrase: r.secret.key_passphrase.clone() };
    if user.is_none() {
        let default_user = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default();
        let Some(ans) = state.ask("credentials", json!({
            "host": r.host, "name": r.title, "protocol": "ssh", "need_user": true, "need_password": false,
            "user": "", "default_user": default_user, "domain": "",
        })).await else {
            return Err(CmdErr::new("cancelled", format!("{} was not opened: no credentials were entered", r.title)));
        };
        user = ans.get("user").and_then(Value::as_str).map(String::from).filter(|s| !s.is_empty());
        if let Some(p) = ans.get("password").and_then(Value::as_str).filter(|s| !s.is_empty()) { secret.password = Some(p.into()); }
    }
    let user = user.ok_or_else(|| CmdErr::new("cancelled", format!("{} was not opened: no credentials were entered", r.title)))?;
    let label = r.title.clone();

    let keepalive = o.get("keepalive_seconds").and_then(Value::as_u64).unwrap_or(30);

    let mut config = client::Config::default();
    config.keepalive_interval = (keepalive > 0).then(|| std::time::Duration::from_secs(keepalive));
    config.keepalive_max = 3;
    config.nodelay = true;
    if !o.get("allow_sha1_rsa").and_then(Value::as_bool).unwrap_or(false) {
        let mut pref = config.preferred.clone();
        pref.key = std::borrow::Cow::Owned(pref.key.iter().filter(|a| a.as_str() != "ssh-rsa").cloned().collect());
        config.preferred = pref;
    }
    let failure = Arc::new(Mutex::new(None));
    let handler = Handler { remote, state: state.clone(), ws: r.ws.clone(), host: r.host.clone(), port: r.port, label: label.clone(), failure: failure.clone() };
    let cfg = Arc::new(config);
    let attempt = async {
        match via {
            None => client::connect(cfg, (r.host.as_str(), r.port), handler).await,
            Some(v) => {
                let ch = v.channel_open_direct_tcpip(r.host.clone(), r.port as u32, "127.0.0.1", 0).await?;
                client::connect_stream(cfg, ch.into_stream(), handler).await
            }
        }
    };
    let mut handle = match tokio::time::timeout(std::time::Duration::from_secs(20), attempt).await {
        Err(_) => return Err(CmdErr::new("ssh", format!("timed out connecting to {}:{}", r.host, r.port))),
        Ok(Err(e)) => {
            if let Some(f) = failure.lock().unwrap().take() { return Err(f); }
            return Err(CmdErr::new("ssh", format!("could not reach {}:{}: {e}", r.host, r.port)));
        }
        Ok(Ok(h)) => h,
    };

    authenticate(state, &mut handle, &user, &secret, o, r, &label).await?;
    Ok(handle)
}

pub async fn open_session(state: &AppState, r: Resolved, via: Option<Arc<client::Handle<Handler>>>, lease: Option<crate::tunnel::Lease>, cols: u32, rows: u32, out: Channel<InvokeResponseBody>) -> CmdResult<Opened> {
    let o = &r.options;
    let handle = connect_handle(state, &r, via.as_deref(), None).await?;
    let fixed_cols = o.get("fixed_cols").and_then(Value::as_u64).map(|n| n as u32);
    let fixed_rows = o.get("fixed_rows").and_then(Value::as_u64).map(|n| n as u32);
    let (c, rw) = match (fixed_cols, fixed_rows) { (Some(c), Some(r)) => (c, r), _ => (cols, rows) };
    let mut channel = handle.channel_open_session().await.map_err(|e| CmdErr::new("ssh", e.to_string()))?;
    let term = opt_str(o, "term_type").unwrap_or_else(|| "xterm-256color".into());
    channel.request_pty(true, &term, c.max(1), rw.max(1), 0, 0, &[]).await.map_err(|e| CmdErr::new("ssh", e.to_string()))?;
    channel.request_shell(true).await.map_err(|e| CmdErr::new("ssh", e.to_string()))?;

    let id = ok_crypto::random_token();
    let log_file = if o.get("session_log").and_then(Value::as_bool).unwrap_or(false) {
        let dir = opt_str(o, "session_log_dir").map(std::path::PathBuf::from).unwrap_or_else(|| state.config_dir.join("logs"));
        let safe: String = r.title.chars().map(|ch| if ch.is_ascii_alphanumeric() || "-_.".contains(ch) { ch } else { '_' }).collect();
        std::fs::create_dir_all(&dir).ok();
        let p = dir.join(format!("{safe}-{}.log", chrono::Local::now().format("%Y%m%d-%H%M%S")));
        std::fs::File::create(&p).ok().map(|f| (p, f))
    } else { None };
    let log_timestamps = o.get("session_log_timestamps").and_then(Value::as_bool).unwrap_or(false);
    let log_path = log_file.as_ref().map(|(p, _)| p.display().to_string());

    let (tx, mut rx) = mpsc::unbounded_channel::<SessionInput>();
    state.sessions.lock().unwrap().insert(id.clone(), SessionHandle { tx, title: r.title.clone(), ws: r.ws.clone(), conn_id: r.conn_id.clone() });
    let st = state.clone();
    let sid = id.clone();
    tokio::spawn(async move {
        let _keep = (via, lease);
        use std::io::Write;
        let mut log = log_file.map(|(_, f)| f);
        let mut at_line_start = true;
        let mut exit_status: Option<u32> = None;
        loop {
            tokio::select! {
                msg = channel.wait() => match msg {
                    Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                        if let Some(f) = log.as_mut() {
                            if log_timestamps {
                                for b in data.iter() {
                                    if at_line_start { let _ = write!(f, "[{}] ", chrono::Local::now().format("%Y-%m-%d %H:%M:%S")); at_line_start = false; }
                                    let _ = f.write_all(&[*b]);
                                    if *b == b'\n' { at_line_start = true; }
                                }
                            } else { let _ = f.write_all(&data); }
                        }
                        let _ = out.send(InvokeResponseBody::Raw(data.to_vec()));
                    }
                    Some(ChannelMsg::ExitStatus { exit_status: s }) => exit_status = Some(s),
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => break,
                    _ => {}
                },
                inp = rx.recv() => match inp {
                    Some(SessionInput::Data(d)) => { if channel.data_bytes(d).await.is_err() { break; } }
                    Some(SessionInput::Resize(c, r)) => { let _ = channel.window_change(c.max(1), r.max(1), 0, 0).await; }
                    Some(SessionInput::Close) | None => break,
                }
            }
        }
        let _ = channel.close().await;
        let _ = handle.disconnect(Disconnect::ByApplication, "", "en").await;
        st.sessions.lock().unwrap().remove(&sid);
        use tauri::Emitter;
        let _ = st.app.emit("session-state", json!({"id": sid, "state": "closed", "exit_status": exit_status}));
    });

    Ok(Opened {
        id, title: r.title,
        osc52: o.get("osc52").and_then(Value::as_bool).unwrap_or(false),
        fixed_cols, fixed_rows,
        terminal_profile: opt_str(o, "terminal_profile"),
        log_file: log_path,
    })
}

type DynAgent = russh::keys::agent::client::AgentClient<Box<dyn russh::keys::agent::client::AgentStream + Send + Unpin>>;

async fn connect_agent() -> Option<DynAgent> {
    use russh::keys::agent::client::AgentClient;
    #[cfg(unix)]
    { AgentClient::connect_env().await.ok().map(|a| a.dynamic()) }
    #[cfg(windows)]
    {
        if let Ok(a) = AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await { return Some(a.dynamic()); }
        AgentClient::connect_pageant().await.ok().map(|a| a.dynamic())
    }
}

/// Tries every identity the agent offers until one is accepted.
async fn agent_auth(h: &mut client::Handle<Handler>, user: &str) -> Result<bool, russh::Error> {
    let Some(mut agent) = connect_agent().await else { return Ok(false) };
    let Ok(ids) = agent.request_identities().await else { return Ok(false) };
    let hash = h.best_supported_rsa_hash().await?.flatten();
    for id in ids {
        let russh::keys::agent::AgentIdentity::PublicKey { key, .. } = id else { continue };
        if let Ok(res) = h.authenticate_publickey_with(user, key, hash, &mut agent).await {
            if res.success() { return Ok(true); }
        }
    }
    Ok(false)
}

async fn authenticate(state: &AppState, h: &mut client::Handle<Handler>, user: &str, secret: &Secret, o: &Options, r: &Resolved, label: &str) -> CmdResult<()> {
    let method = opt_str(o, "auth_method").unwrap_or_else(|| "auto".into());
    let auto = method == "auto";
    let fail = || CmdErr::new("auth", format!("authentication failed for {user}@{}:{}", r.host, r.port));
    let io = |e: russh::Error| CmdErr::new("ssh", e.to_string());

    // key
    if auto && (secret.private_key.is_some() || opt_str(o, "key_file").is_some()) || method == "key" {
        let text = match (&secret.private_key, opt_str(o, "key_file")) {
            (Some(k), _) => Some((k.clone(), "stored key".to_string())),
            (None, Some(f)) => std::fs::read_to_string(expand_home(&f)).ok().map(|t| (t, f)),
            _ => None,
        };
        if let Some((text, name)) = text {
            let mut pass = secret.key_passphrase.clone();
            let key = loop {
                match keys::decode_secret_key(&text, pass.as_deref()) {
                    Ok(k) => break Some(k),
                    Err(keys::Error::KeyIsEncrypted) if pass.is_none() => {
                        match state.ask("key_passphrase", json!({"host": r.host, "key": name, "label": label})).await.and_then(|v| v.as_str().map(String::from)) {
                            Some(p) => pass = Some(p),
                            None => return Err(CmdErr::new("cancelled", "cancelled")),
                        }
                    }
                    Err(_) => break None,
                }
            };
            if let Some(key) = key {
                let hash = h.best_supported_rsa_hash().await.map_err(io)?.flatten();
                let res = h.authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash)).await.map_err(io)?;
                if res.success() { return Ok(()); }
            }
        }
        if !auto { return Err(fail()); }
    }

    // agent
    if method == "agent" || auto {
        if agent_auth(h, user).await.map_err(io)? { return Ok(()); }
        if !auto { return Err(fail()); }
    }

    // password
    if method == "password" || auto {
        let mut pw = secret.password.clone();
        if pw.is_none() && method == "password" {
            pw = state.ask("password", json!({"user": user, "host": r.host, "label": label})).await.and_then(|v| v.as_str().map(String::from));
            if pw.is_none() { return Err(CmdErr::new("cancelled", "cancelled")); }
        }
        if let Some(p) = &pw {
            if h.authenticate_password(user, p.clone()).await.map_err(io)?.success() { return Ok(()); }
        }
        if !auto { return Err(fail()); }
    }

    // keyboard-interactive
    if method == "keyboard_interactive" || auto {
        let mut resp = h.authenticate_keyboard_interactive_start(user, None).await.map_err(io)?;
        let mut used_stored = false;
        for _ in 0..10 {
            match resp {
                KeyboardInteractiveAuthResponse::Success => return Ok(()),
                KeyboardInteractiveAuthResponse::Failure { .. } => break,
                KeyboardInteractiveAuthResponse::InfoRequest { name, instructions, prompts } => {
                    let answers: Vec<String> = if prompts.is_empty() { vec![] }
                    else if prompts.len() == 1 && !prompts[0].echo && secret.password.is_some() && !used_stored {
                        used_stored = true;
                        vec![secret.password.clone().unwrap()]
                    } else {
                        let ps: Vec<Value> = prompts.iter().map(|p| json!({"prompt": p.prompt, "echo": p.echo})).collect();
                        match state.ask("keyboard_interactive", json!({"host": r.host, "name": name, "instructions": instructions, "prompts": ps, "label": label})).await {
                            Some(Value::Array(a)) => a.into_iter().map(|v| v.as_str().unwrap_or("").to_string()).collect(),
                            _ => return Err(CmdErr::new("cancelled", "cancelled")),
                        }
                    };
                    resp = h.authenticate_keyboard_interactive_respond(answers).await.map_err(io)?;
                }
            }
        }
    }
    Err(fail())
}
