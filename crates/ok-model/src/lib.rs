//! Shared records, option inheritance and the quick-connect parser (spec section 3).
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use zeroize::Zeroize;

pub type Options = Map<String, Value>;

pub fn new_id() -> String { uuid::Uuid::now_v7().to_string() }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol { Ssh, Rdp, Vnc, Http, Https, External }

impl Protocol {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "ssh" | "ssh2" => Self::Ssh,
            "rdp" => Self::Rdp,
            "vnc" => Self::Vnc,
            "http" => Self::Http,
            "https" => Self::Https,
            "external" => Self::External,
            _ => return None,
        })
    }
    pub fn as_str(self) -> &'static str {
        match self { Self::Ssh => "ssh", Self::Rdp => "rdp", Self::Vnc => "vnc", Self::Http => "http", Self::Https => "https", Self::External => "external" }
    }
    pub fn default_port(self) -> u16 {
        match self { Self::Ssh => 22, Self::Rdp => 3389, Self::Vnc => 5900, Self::Http => 80, Self::Https => 443, Self::External => 0 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Folder {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    #[serde(default)] pub color: Option<String>,
    #[serde(default)] pub icon: Option<String>,
    #[serde(default)] pub defaults: Options,
    #[serde(default)] pub has_secret: bool,
    #[serde(default)] pub sort: i64,
    #[serde(default)] pub version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Connection {
    pub id: String,
    pub folder_id: Option<String>,
    pub name: String,
    pub protocol: Protocol,
    #[serde(default)] pub host: String,
    #[serde(default)] pub port: Option<u16>,
    #[serde(default)] pub options: Options,
    #[serde(default)] pub tags: Vec<String>,
    #[serde(default)] pub notes: String,
    #[serde(default)] pub icon: Option<String>,
    #[serde(default)] pub has_secret: bool,
    #[serde(default)] pub sort: i64,
    #[serde(default)] pub version: i64,
}

impl Connection {
    pub fn effective_port(&self) -> u16 { self.port.unwrap_or_else(|| self.protocol.default_port()) }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Forward {
    Local { bind_addr: String, bind_port: u16, remote_host: String, remote_port: u16 },
    Dynamic { bind_addr: String, bind_port: u16 },
    Remote { remote_bind_addr: String, remote_bind_port: u16, local_host: String, local_port: u16 },
}

impl Forward {
    pub fn exposes(&self) -> bool {
        let a = match self {
            Forward::Local { bind_addr, .. } | Forward::Dynamic { bind_addr, .. } => bind_addr,
            Forward::Remote { remote_bind_addr, .. } => remote_bind_addr,
        };
        !matches!(a.as_str(), "" | "127.0.0.1" | "::1" | "localhost")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TunnelProfile {
    pub id: String,
    pub name: String,
    #[serde(default)] pub chain: Vec<String>,
    #[serde(default)] pub forwards: Vec<Forward>,
    #[serde(default)] pub auto_start: bool,
    #[serde(default)] pub version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TerminalProfile {
    pub id: String,
    pub name: String,
    #[serde(default)] pub spec: Value,
    #[serde(default)] pub version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HostKey {
    pub host: String,
    pub port: u16,
    pub key_type: String,
    pub fingerprint: String,
    pub first_seen: String,
    pub trusted_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditEntry {
    pub id: String,
    pub at: String,
    pub actor: String,
    pub action: String,
    pub target_id: String,
    pub detail: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Snapshot {
    pub folders: Vec<Folder>,
    pub connections: Vec<Connection>,
    pub tunnels: Vec<TunnelProfile>,
    pub terminal_profiles: Vec<TerminalProfile>,
}

/// Credentials of a folder or connection. Empty strings count as unset.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Secret {
    #[serde(skip_serializing_if = "Option::is_none")] pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub private_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub key_passphrase: Option<String>,
}

pub const SECRET_FIELDS: [&str; 5] = ["username", "password", "domain", "private_key", "key_passphrase"];

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Secret")
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("domain", &self.domain)
            .field("private_key", &self.private_key.as_ref().map(|_| "<redacted>"))
            .field("key_passphrase", &self.key_passphrase.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        for f in [&mut self.username, &mut self.password, &mut self.domain, &mut self.private_key, &mut self.key_passphrase] {
            if let Some(s) = f { s.zeroize(); }
        }
    }
}

impl Secret {
    pub fn normalized(mut self) -> Self {
        for f in [&mut self.username, &mut self.password, &mut self.domain, &mut self.private_key, &mut self.key_passphrase] {
            if f.as_deref() == Some("") { *f = None; }
        }
        self
    }
    /// Builder for tests and importers (struct-update syntax is unavailable because Secret zeroizes on drop).
    pub fn with(mut self, field: &str, v: &str) -> Self { self.set(field, Some(v.to_string())); self }
    pub fn is_empty(&self) -> bool {
        self.username.is_none() && self.password.is_none() && self.domain.is_none() && self.private_key.is_none() && self.key_passphrase.is_none()
    }
    pub fn get(&self, field: &str) -> Option<&String> {
        match field {
            "username" => self.username.as_ref(),
            "password" => self.password.as_ref(),
            "domain" => self.domain.as_ref(),
            "private_key" => self.private_key.as_ref(),
            "key_passphrase" => self.key_passphrase.as_ref(),
            _ => None,
        }
    }
    pub fn set(&mut self, field: &str, v: Option<String>) {
        let v = v.filter(|s| !s.is_empty());
        match field {
            "username" => self.username = v,
            "password" => self.password = v,
            "domain" => self.domain = v,
            "private_key" => self.private_key = v,
            "key_passphrase" => self.key_passphrase = v,
            _ => {}
        }
    }
    /// Fill every unset field from `parent` (apply nearest ancestor first).
    pub fn inherit_from(&mut self, parent: &Secret) {
        for f in SECRET_FIELDS {
            if self.get(f).is_none() {
                self.set(f, parent.get(f).cloned());
            }
        }
    }
}

pub const NON_INHERITING: [&str; 2] = ["pinned", "auto_open"];

impl Snapshot {
    pub fn folder(&self, id: &str) -> Option<&Folder> { self.folders.iter().find(|f| f.id == id) }
    pub fn connection(&self, id: &str) -> Option<&Connection> { self.connections.iter().find(|c| c.id == id) }

    /// Ancestors of a folder, nearest first, guarded against cycles.
    pub fn ancestors(&self, folder_id: Option<&str>) -> Vec<&Folder> {
        let by_id: HashMap<&str, &Folder> = self.folders.iter().map(|f| (f.id.as_str(), f)).collect();
        let mut out = vec![];
        let mut cur = folder_id;
        for _ in 0..4096 {
            let Some(id) = cur else { break };
            let Some(f) = by_id.get(id) else { break };
            out.push(*f);
            cur = f.parent_id.as_deref();
        }
        out
    }

    pub fn folder_path(&self, folder_id: Option<&str>) -> String {
        let mut names: Vec<&str> = self.ancestors(folder_id).iter().map(|f| f.name.as_str()).collect();
        names.reverse();
        names.join("/")
    }

    pub fn resolve_options(&self, c: &Connection) -> Options {
        let mut merged = Options::new();
        for f in self.ancestors(c.folder_id.as_deref()).into_iter().rev() {
            for (k, v) in &f.defaults {
                if NON_INHERITING.contains(&k.as_str()) { continue; }
                if v.is_null() { merged.remove(k); } else { merged.insert(k.clone(), v.clone()); }
            }
        }
        for (k, v) in &c.options {
            if v.is_null() { merged.remove(k); } else { merged.insert(k.clone(), v.clone()); }
        }
        merged
    }

    /// Nearest folder color, for tab and tree chips.
    pub fn inherited_color(&self, folder_id: Option<&str>) -> Option<String> {
        self.ancestors(folder_id).iter().find_map(|f| f.color.clone())
    }
}

// ---------------------------------------------------------------- quick connect

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuickTarget {
    pub protocol: Protocol,
    pub host: String,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub url: Option<String>,
    pub name: String,
}

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() + 0 && i + 2 <= b.len() - 1 + 0 {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) { out.push(v); i += 3; continue; }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn parse_quick(input: &str) -> Result<QuickTarget, String> {
    let input = input.trim();
    if input.is_empty() { return Err("Enter a host to connect to".into()); }
    let (protocol, rest) = match input.find("://") {
        Some(i) => {
            let scheme = &input[..i];
            let p = Protocol::parse(scheme).filter(|p| *p != Protocol::External)
                .ok_or_else(|| format!("Unsupported scheme: {scheme}"))?;
            (p, &input[i + 3..])
        }
        None => (Protocol::Ssh, input),
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    if matches!(protocol, Protocol::Ssh | Protocol::Rdp | Protocol::Vnc) && !(tail.is_empty() || tail == "/") {
        return Err("Unexpected path in address".into());
    }
    let (user, hostport) = match authority.rfind('@') {
        Some(i) => {
            let ui = &authority[..i];
            let u = ui.split(':').next().unwrap_or("");
            (Some(pct_decode(u)).filter(|u| !u.is_empty()), &authority[i + 1..])
        }
        None => (None, authority),
    };
    let (host, port_s): (String, Option<&str>) = if let Some(stripped) = hostport.strip_prefix('[') {
        let close = stripped.find(']').ok_or("Unclosed [ in address")?;
        let h = &stripped[..close];
        let after = &stripped[close + 1..];
        (h.to_string(), after.strip_prefix(':'))
    } else if hostport.matches(':').count() > 1 {
        (hostport.to_string(), None)
    } else if let Some((h, p)) = hostport.split_once(':') {
        (h.to_string(), Some(p))
    } else {
        (hostport.to_string(), None)
    };
    if host.is_empty() { return Err("Enter a host to connect to".into()); }
    let port = match port_s {
        None => None,
        Some(p) => match p.parse::<u32>() {
            Ok(n) if (1..=65535).contains(&n) => Some(n as u16),
            _ => return Err(format!("Invalid port: {p}")),
        },
    };
    let url = matches!(protocol, Protocol::Http | Protocol::Https).then(|| {
        let hp = match port { Some(p) => format!("{}:{p}", if host.contains(':') { format!("[{host}]") } else { host.clone() }), None => host.clone() };
        format!("{}://{hp}{tail}", protocol.as_str())
    });
    let mut name = String::new();
    if let Some(u) = &user { name.push_str(u); name.push('@'); }
    name.push_str(&host);
    if let Some(p) = port { name.push_str(&format!(":{p}")); }
    if let Some(u) = &url { name = u.clone(); }
    Ok(QuickTarget { protocol, host, port, user, url, name })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn folder(id: &str, parent: Option<&str>, defaults: Value) -> Folder {
        Folder { id: id.into(), parent_id: parent.map(Into::into), name: id.into(), color: None, icon: None,
            defaults: defaults.as_object().unwrap().clone(), has_secret: false, sort: 0, version: 1 }
    }

    #[test]
    fn options_inherit_leaf_wins_and_skip_pinned() {
        let snap = Snapshot { folders: vec![
            folder("a", None, json!({"keepalive_seconds": 10, "pinned": true, "term_type": "vt100"})),
            folder("b", Some("a"), json!({"keepalive_seconds": 20, "term_type": null})),
        ], ..Default::default() };
        let c = Connection { id: "c".into(), folder_id: Some("b".into()), name: "c".into(), protocol: Protocol::Ssh,
            host: "h".into(), port: None, options: json!({"x": 1}).as_object().unwrap().clone(), tags: vec![], notes: "".into(),
            icon: None, has_secret: false, sort: 0, version: 1 };
        let o = snap.resolve_options(&c);
        assert_eq!(o["keepalive_seconds"], 20);
        assert!(!o.contains_key("pinned"));
        assert!(!o.contains_key("term_type"));
        assert_eq!(o["x"], 1);
    }

    #[test]
    fn secret_inherit_per_field() {
        let mut s = Secret::default().with("username", "u");
        s.inherit_from(&Secret::default().with("username", "p").with("password", "pw"));
        assert_eq!(s.username.as_deref(), Some("u"));
        assert_eq!(s.password.as_deref(), Some("pw"));
        assert!(!format!("{s:?}").contains("pw"));
    }

    #[test]
    fn quick_parse_rules() {
        let t = parse_quick("root@10.0.0.5:2222").unwrap();
        assert_eq!((t.user.as_deref(), t.host.as_str(), t.port), (Some("root"), "10.0.0.5", Some(2222)));
        assert_eq!(t.name, "root@10.0.0.5:2222");
        assert_eq!(parse_quick("u:secret@h").unwrap().user.as_deref(), Some("u"));
        assert_eq!(parse_quick("rdp://srv").unwrap().protocol, Protocol::Rdp);
        let w = parse_quick("https://host.example.com:8443/admin?x=1").unwrap();
        assert_eq!(w.url.as_deref(), Some("https://host.example.com:8443/admin?x=1"));
        let v6 = parse_quick("[::1]:22").unwrap();
        assert_eq!((v6.host.as_str(), v6.port), ("::1", Some(22)));
        let bare = parse_quick("fe80::1").unwrap();
        assert_eq!((bare.host.as_str(), bare.port), ("fe80::1", None));
        assert!(parse_quick("").is_err());
        assert!(parse_quick("external://x").is_err());
        assert!(parse_quick("ftp://x").is_err());
        assert!(parse_quick("host:0").is_err());
        assert!(parse_quick("host:70000").is_err());
        assert!(parse_quick("ssh://host/path").is_err());
        assert_eq!(parse_quick("a%40b@h").unwrap().user.as_deref(), Some("a@b"));
    }
}
