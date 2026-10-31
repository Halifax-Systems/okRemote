//! Importers (spec section 14): CSV, OpenSSH config, okRemote JSON. Parsing is in memory; nothing is written here.
use ok_model::*;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;

pub const MAX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct ReportItem { pub item: String, pub level: &'static str, pub message: String }

#[derive(Debug, Default, Clone)]
pub struct Parsed {
    pub folders: Vec<Folder>,
    pub connections: Vec<Connection>,
    pub tunnels: Vec<TunnelProfile>,
    /// connection id -> credentials
    pub secrets: HashMap<String, Secret>,
    pub report: Vec<ReportItem>,
}

impl Parsed {
    fn warn(&mut self, item: &str, m: impl Into<String>) { self.report.push(ReportItem { item: item.into(), level: "warning", message: m.into() }); }
    fn info(&mut self, item: &str, m: impl Into<String>) { self.report.push(ReportItem { item: item.into(), level: "info", message: m.into() }); }
}

/// Decode UTF-8 (with or without BOM) or UTF-16LE with BOM.
pub fn decode_text(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() > MAX_BYTES { return Err("file is too large to import".into()); }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let u: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return String::from_utf16(&u).map_err(|e| e.to_string());
    }
    let b = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8(b.to_vec()).map_err(|_| "the file is not valid text".into())
}

fn folder_for_path(p: &mut Parsed, cache: &mut HashMap<String, String>, path: &str) -> Option<String> {
    let mut parent: Option<String> = None;
    let mut key = String::new();
    for part in path.split(['/', '\\']).map(str::trim).filter(|s| !s.is_empty()) {
        key.push('/');
        key.push_str(part);
        let id = cache.entry(key.clone()).or_insert_with(|| {
            let id = new_id();
            p.folders.push(Folder { id: id.clone(), parent_id: parent.clone(), name: part.into(), color: None, icon: None, defaults: Default::default(), has_secret: false, sort: p.folders.len() as i64, version: 0 });
            id
        }).clone();
        parent = Some(id);
    }
    parent
}

fn blank_conn(name: String, folder: Option<String>, protocol: Protocol, host: String, sort: i64) -> Connection {
    Connection { id: new_id(), folder_id: folder, name, protocol, host, port: None, options: Default::default(), tags: vec![], notes: String::new(), icon: None, has_secret: false, sort, version: 0 }
}

// ------------------------------------------------------------------ CSV

pub fn parse_csv(text: &str) -> Result<Parsed, String> {
    let mut rdr = csv::ReaderBuilder::new().flexible(true).trim(csv::Trim::All).from_reader(text.as_bytes());
    let headers: Vec<String> = rdr.headers().map_err(|e| e.to_string())?.iter().map(|h| h.to_lowercase()).collect();
    let find = |names: &[&str]| headers.iter().position(|h| names.contains(&h.as_str()));
    let (c_name, c_host, c_port, c_proto) = (find(&["name", "display name", "displayname"]), find(&["host", "hostname", "address", "server"]),
        find(&["port"]), find(&["protocol", "type"]));
    let (c_user, c_pass, c_dom, c_folder, c_tags, c_notes) = (find(&["username", "user"]), find(&["password"]), find(&["domain"]),
        find(&["folder", "path", "group"]), find(&["tags", "tag"]), find(&["notes", "description", "descr"]));
    let c_host = c_host.ok_or("CSV needs a 'host' column")?;
    let mut p = Parsed::default();
    let mut cache = HashMap::new();
    let mut any_password = false;
    for (i, rec) in rdr.records().enumerate() {
        let line = i + 2;
        let rec = match rec { Ok(r) => r, Err(e) => { p.warn(&format!("line {line}"), e.to_string()); continue } };
        let get = |c: Option<usize>| c.and_then(|c| rec.get(c)).map(str::trim).unwrap_or("");
        let host = get(Some(c_host));
        if host.is_empty() { p.warn(&format!("line {line}"), "no host; row skipped"); continue; }
        let proto_s = get(c_proto);
        let protocol = if proto_s.is_empty() { Protocol::Ssh } else {
            Protocol::parse(proto_s).unwrap_or_else(|| { p.warn(&format!("line {line}"), format!("unknown protocol '{proto_s}'; imported as SSH")); Protocol::Ssh })
        };
        let name = if get(c_name).is_empty() { host.to_string() } else { get(c_name).to_string() };
        let folder = folder_for_path(&mut p, &mut cache, get(c_folder));
        let mut c = blank_conn(name, folder, protocol, host.to_string(), p.connections.len() as i64);
        c.port = get(c_port).parse::<u16>().ok().filter(|&n| n != protocol.default_port());
        c.tags = get(c_tags).split([';', ',']).map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
        c.notes = get(c_notes).to_string();
        let mut s = Secret::default();
        for (f, v) in [("username", get(c_user)), ("password", get(c_pass)), ("domain", get(c_dom))] {
            if !v.is_empty() { s.set(f, Some(v.into())); }
        }
        any_password |= s.password.is_some();
        if !s.is_empty() { c.has_secret = true; p.secrets.insert(c.id.clone(), s); }
        p.connections.push(c);
    }
    if any_password { p.warn("passwords", "the file holds plaintext passwords; delete it after importing"); }
    Ok(p)
}

// ------------------------------------------------------------------ OpenSSH config

#[derive(Default, Clone)]
struct HostBlock { patterns: Vec<String>, kv: Vec<(String, String)> }

fn split_kv(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    let idx = line.find(|c: char| c.is_whitespace() || c == '=')?;
    let (k, v) = line.split_at(idx);
    let v = v.trim_start_matches(|c: char| c.is_whitespace() || c == '=').trim().trim_matches('"');
    Some((k.to_lowercase(), v.to_string()))
}

pub fn parse_openssh(text: &str) -> Parsed {
    let mut p = Parsed::default();
    let mut blocks: Vec<HostBlock> = vec![];
    let mut in_match = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        let Some((k, v)) = split_kv(line) else { continue };
        match k.as_str() {
            "host" => { in_match = false; blocks.push(HostBlock { patterns: v.split_whitespace().map(|s| s.trim_matches('"').to_string()).collect(), kv: vec![] }); }
            "match" => { in_match = true; p.warn("Match", "Match blocks are skipped"); }
            "include" => p.warn("Include", "Include lines are not followed"),
            _ => if !in_match { if let Some(b) = blocks.last_mut() { b.kv.push((k, v)); } },
        }
    }
    let first = |kv: &[(String, String)], key: &str| kv.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
    let star: Vec<(String, String)> = blocks.iter().filter(|b| b.patterns.iter().any(|x| x == "*")).flat_map(|b| b.kv.clone()).collect();
    let mut applied_star = false;
    let mut alias_ids: HashMap<String, String> = HashMap::new();
    let concrete: Vec<(String, Vec<(String, String)>)> = blocks.iter().flat_map(|b| {
        b.patterns.iter().filter(|a| !a.contains(['*', '?', '!'])).map(|a| (a.clone(), b.kv.clone())).collect::<Vec<_>>()
    }).collect();
    for (alias, kv) in &concrete {
        let get = |key: &str| first(kv, key).or_else(|| first(&star, key));
        let host = get("hostname").map(|h| h.replace("%h", alias)).unwrap_or_else(|| alias.clone());
        let mut c = blank_conn(alias.clone(), None, Protocol::Ssh, host, p.connections.len() as i64);
        c.port = get("port").and_then(|v| v.parse().ok()).filter(|&n| n != 22);
        if let Some(k) = get("identityfile") { c.options.insert("key_file".into(), json!(k)); c.options.insert("auth_method".into(), json!("auto")); }
        if let Some(k) = get("serveraliveinterval").and_then(|v| v.parse::<i64>().ok()) { c.options.insert("keepalive_seconds".into(), json!(k)); }
        if let Some(u) = get("user") { c.has_secret = true; p.secrets.insert(c.id.clone(), Secret::default().with("username", &u)); }
        if get("proxycommand").is_some() && first(kv, "proxycommand").is_some() { p.warn(alias, "ProxyCommand is not supported"); }
        applied_star |= first(&star, "user").is_some() || first(&star, "identityfile").is_some();
        alias_ids.insert(alias.clone(), c.id.clone());
        p.connections.push(c);
    }
    if applied_star { p.info("Host *", "User and IdentityFile from 'Host *' were applied to every host"); }
    // ProxyJump chains and forwards
    let mut chain_profiles: HashMap<String, String> = HashMap::new();
    for (idx, (alias, kv)) in concrete.iter().enumerate() {
        let Some(jump) = first(kv, "proxyjump").filter(|j| !j.eq_ignore_ascii_case("none")) else { continue };
        let mut chain = vec![];
        for hop in jump.split(',').map(str::trim).filter(|h| !h.is_empty()) {
            if let Some(id) = alias_ids.get(hop) { chain.push(id.clone()); continue; }
            let (user, hp) = hop.rsplit_once('@').map(|(u, h)| (Some(u), h)).unwrap_or((None, hop));
            let (host, port) = match hp.rsplit_once(':') { Some((h, pt)) if pt.parse::<u16>().is_ok() => (h, pt.parse::<u16>().ok()), _ => (hp, None) };
            let mut c = blank_conn(host.to_string(), None, Protocol::Ssh, host.to_string(), p.connections.len() as i64);
            c.port = port.filter(|&n| n != 22);
            c.tags = vec!["jump".into()];
            if let Some(u) = user { c.has_secret = true; p.secrets.insert(c.id.clone(), Secret::default().with("username", u)); }
            alias_ids.insert(hop.to_string(), c.id.clone());
            chain.push(c.id.clone());
            p.connections.push(c);
        }
        if chain.is_empty() { continue; }
        let pid = chain_profiles.entry(jump.clone()).or_insert_with(|| {
            let t = TunnelProfile { id: new_id(), name: format!("Jump via {jump}"), chain, forwards: vec![], auto_start: false, version: 0 };
            let id = t.id.clone();
            p.tunnels.push(t);
            id
        }).clone();
        if let Some(c) = p.connections.iter_mut().find(|c| Some(&c.id) == alias_ids.get(alias)) { c.options.insert("tunnel_profile".into(), json!(pid)); }
        let _ = idx;
    }
    for (alias, kv) in &concrete {
        let mut fwds = vec![];
        for (k, v) in kv {
            let bind = |s: &str| -> (String, Option<u16>) {
                match s.rsplit_once(':') {
                    Some((a, pt)) if pt.parse::<u16>().is_ok() => (if a == "*" { "0.0.0.0".into() } else if a.is_empty() { "127.0.0.1".into() } else { a.into() }, pt.parse().ok()),
                    _ => ("127.0.0.1".into(), s.parse().ok()),
                }
            };
            let parts: Vec<&str> = v.split_whitespace().collect();
            let ok = match (k.as_str(), parts.as_slice()) {
                ("localforward", [l, r]) => {
                    let (ba, bp) = bind(l);
                    r.rsplit_once(':').and_then(|(h, pt)| Some((h, pt.parse::<u16>().ok()?))).zip(bp).map(|((h, rp), bp)|
                        fwds.push(Forward::Local { bind_addr: ba, bind_port: bp, remote_host: h.into(), remote_port: rp })).is_some()
                }
                ("remoteforward", [r, l]) => {
                    let (ba, bp) = bind(r);
                    l.rsplit_once(':').and_then(|(h, pt)| Some((h, pt.parse::<u16>().ok()?))).zip(bp).map(|((h, lp), bp)|
                        fwds.push(Forward::Remote { remote_bind_addr: ba, remote_bind_port: bp, local_host: h.into(), local_port: lp })).is_some()
                }
                ("dynamicforward", [b]) => { let (ba, bp) = bind(b); bp.map(|bp| fwds.push(Forward::Dynamic { bind_addr: ba, bind_port: bp })).is_some() }
                ("localforward" | "remoteforward" | "dynamicforward", _) => false,
                _ => continue,
            };
            if !ok { p.warn(alias, format!("could not parse {k} {v}")); }
        }
        if fwds.is_empty() { continue; }
        let Some(id) = alias_ids.get(alias).cloned() else { continue };
        p.tunnels.push(TunnelProfile { id: new_id(), name: format!("{alias} forwards"), chain: vec![id], forwards: fwds, auto_start: true, version: 0 });
        p.info(alias, "forwards were imported as a tunnel profile; start it from the Tunnels panel");
    }
    p
}

// ------------------------------------------------------------------ okRemote JSON

pub fn parse_json(text: &str) -> Result<Parsed, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if v.get("format").and_then(Value::as_str) != Some("okremote-export") { return Err("not an okRemote export".into()); }
    let mut p = Parsed::default();
    let mut folders: Vec<Folder> = serde_json::from_value(v.get("folders").cloned().unwrap_or(json!([]))).map_err(|e| e.to_string())?;
    let ids: std::collections::HashSet<String> = folders.iter().map(|f| f.id.clone()).collect();
    for f in &mut folders {
        f.version = 0; f.has_secret = false;
        if f.parent_id.as_ref().is_some_and(|x| !ids.contains(x)) { f.parent_id = None; }
    }
    // parents first
    let mut ordered: Vec<Folder> = vec![];
    let mut seen = std::collections::HashSet::new();
    while ordered.len() < folders.len() {
        let before = ordered.len();
        for f in &folders {
            if !seen.contains(&f.id) && f.parent_id.as_ref().map_or(true, |x| seen.contains(x)) { seen.insert(f.id.clone()); ordered.push(f.clone()); }
        }
        if ordered.len() == before { for f in &folders { if seen.insert(f.id.clone()) { let mut f = f.clone(); f.parent_id = None; ordered.push(f); } } }
    }
    p.folders = ordered;
    let mut conns: Vec<Connection> = serde_json::from_value(v.get("connections").cloned().unwrap_or(json!([]))).map_err(|e| e.to_string())?;
    for c in &mut conns {
        c.version = 0; c.has_secret = false;
        if c.folder_id.as_ref().is_some_and(|x| !ids.contains(x)) { c.folder_id = None; }
    }
    p.connections = conns;
    p.tunnels = serde_json::from_value(v.get("tunnels").cloned().unwrap_or(json!([]))).unwrap_or_default();
    for t in &mut p.tunnels { t.version = 0; }
    Ok(p)
}

// ------------------------------------------------------------------ PuTTY

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < b.len() {
        if b[i] == b'%' && i + 3 <= b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) { out.push(v); i += 3; continue; }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Sessions are (encoded name, settings). Pure so it can be tested without a registry or home folder.
pub fn parse_putty(sessions: Vec<(String, HashMap<String, String>)>) -> Parsed {
    let mut p = Parsed::default();
    for (raw, kv) in sessions {
        let name = pct_decode(&raw);
        if name == "Default Settings" { p.info(&name, "default settings are skipped"); continue; }
        let proto = kv.get("Protocol").map(|s| s.to_lowercase()).unwrap_or_else(|| "ssh".into());
        if proto != "ssh" { p.warn(&name, format!("{proto} sessions are not imported")); continue; }
        let host_field = kv.get("HostName").map(|s| s.trim().to_string()).unwrap_or_default();
        if host_field.is_empty() { p.info(&name, "no host name; skipped"); continue; }
        let (mut user, host) = match host_field.rsplit_once('@') { Some((u, h)) => (Some(u.to_string()), h.to_string()), None => (None, host_field) };
        if let Some(u) = kv.get("UserName").filter(|u| !u.is_empty()) { user = Some(u.clone()); }
        let mut c = blank_conn(name.clone(), None, Protocol::Ssh, host, p.connections.len() as i64);
        c.port = kv.get("PortNumber").and_then(|v| v.parse().ok()).filter(|&n| n != 22);
        if let Some(k) = kv.get("PublicKeyFile").filter(|k| !k.is_empty()) {
            if k.to_lowercase().ends_with(".ppk") { p.warn(&name, "key is a .ppk file; convert it with PuTTYgen"); }
            c.options.insert("key_file".into(), json!(k));
            c.options.insert("auth_method".into(), json!("auto"));
        }
        if let Some(n) = kv.get("PingIntervalSecs").and_then(|v| v.parse::<i64>().ok()).filter(|n| *n > 0) { c.options.insert("keepalive_seconds".into(), json!(n)); }
        if kv.get("ProxyMethod").map_or(false, |m| m != "0") { p.warn(&name, "a proxy configuration was not imported"); }
        if let Some(u) = user { c.has_secret = true; p.secrets.insert(c.id.clone(), Secret::default().with("username", &u)); }
        p.connections.push(c);
    }
    p
}

#[cfg_attr(windows, allow(dead_code))]
fn parse_kv_file(text: &str) -> HashMap<String, String> {
    text.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect()
}

/// Reads saved sessions from this computer: the registry on Windows, `~/.putty/sessions` elsewhere.
pub fn load_putty() -> Result<Vec<(String, HashMap<String, String>)>, String> {
    #[cfg(windows)]
    {
        use winreg::{enums::HKEY_CURRENT_USER, RegKey};
        let root = RegKey::predef(HKEY_CURRENT_USER).open_subkey(r"Software\SimonTatham\PuTTY\Sessions").map_err(|_| "no PuTTY sessions were found".to_string())?;
        let mut out = vec![];
        for name in root.enum_keys().flatten() {
            let Ok(k) = root.open_subkey(&name) else { continue };
            let mut kv = HashMap::new();
            for (n, v) in k.enum_values().flatten() { kv.insert(n, v.to_string()); }
            out.push((name, kv));
        }
        return Ok(out);
    }
    #[cfg(not(windows))]
    {
        let dir = dirs::home_dir().ok_or("no home folder")?.join(".putty").join("sessions");
        let rd = std::fs::read_dir(&dir).map_err(|_| "no PuTTY sessions were found in ~/.putty/sessions".to_string())?;
        let mut out = vec![];
        for e in rd.flatten() {
            let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
            out.push((e.file_name().to_string_lossy().into_owned(), parse_kv_file(&text)));
        }
        Ok(out)
    }
}

// ------------------------------------------------------------------ RDCMan

fn child<'a>(n: roxmltree::Node<'a, 'a>, name: &str) -> Option<roxmltree::Node<'a, 'a>> { n.children().find(|c| c.is_element() && c.tag_name().name() == name) }
fn child_text(n: roxmltree::Node, path: &[&str]) -> Option<String> {
    let mut cur = n;
    for p in path { cur = child(cur, p)?; }
    cur.text().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

pub fn parse_rdg(text: &str) -> Result<Parsed, String> {
    let doc = roxmltree::Document::parse_with_options(text, roxmltree::ParsingOptions { allow_dtd: false, ..Default::default() }).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    if root.tag_name().name() != "RDCMan" { return Err("not a Remote Desktop Connection Manager file".into()); }
    let mut p = Parsed::default();
    let mut stored_pw = 0usize;
    // The root holds a <file> element which behaves like a group but is not itself a folder.
    let top = child(root, "file").unwrap_or(root);
    fn walk(n: roxmltree::Node, parent: Option<String>, p: &mut Parsed, pw: &mut usize) {
        for g in n.children().filter(|c| c.is_element() && c.tag_name().name() == "group") {
            let name = child_text(g, &["properties", "name"]).unwrap_or_else(|| "Group".into());
            let id = new_id();
            p.folders.push(Folder { id: id.clone(), parent_id: parent.clone(), name, color: None, icon: None, defaults: Default::default(), has_secret: false, sort: p.folders.len() as i64, version: 0 });
            walk(g, Some(id), p, pw);
        }
        for sv in n.children().filter(|c| c.is_element() && c.tag_name().name() == "server") {
            let host = child_text(sv, &["properties", "name"]).or_else(|| child_text(sv, &["name"])).unwrap_or_default();
            if host.is_empty() { continue; }
            let title = child_text(sv, &["displayName"]).or_else(|| child_text(sv, &["properties", "displayName"])).unwrap_or_else(|| host.clone());
            let mut c = blank_conn(title.clone(), parent.clone(), Protocol::Rdp, host, p.connections.len() as i64);
            c.notes = child_text(sv, &["properties", "comment"]).unwrap_or_default();
            c.port = child_text(sv, &["connectionSettings", "port"]).and_then(|v| v.parse().ok()).filter(|&n| n != 3389);
            if let Some(lc) = child(sv, "logonCredentials") {
                let inherit = lc.attribute("inherit").unwrap_or("None");
                if inherit == "None" {
                    let mut s = Secret::default();
                    if let Some(u) = child_text(lc, &["userName"]) { s.set("username", Some(u)); }
                    if let Some(d) = child_text(lc, &["domain"]) { s.set("domain", Some(d)); }
                    if child_text(lc, &["password"]).is_some() { *pw += 1; }
                    if !s.is_empty() { c.has_secret = true; p.secrets.insert(c.id.clone(), s); }
                }
            }
            if let Some(gw) = child(sv, "gatewaySettings") {
                if child_text(gw, &["enabled"]).as_deref() == Some("True") {
                    if let Some(h) = child_text(gw, &["hostName"]) {
                        c.notes = format!("{}RD Gateway: {h}", if c.notes.is_empty() { String::new() } else { format!("{}\n", c.notes) });
                        p.warn(&title, "Remote Desktop gateways are not supported; the gateway host was saved in the notes");
                    }
                }
            }
            p.connections.push(c);
        }
    }
    walk(top, None, &mut p, &mut stored_pw);
    if stored_pw > 0 { p.warn("passwords", format!("{stored_pw} stored passwords are protected by Windows and cannot be imported")); }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_maps_columns_and_folders() {
        let p = parse_csv("Name,Host,Port,Protocol,User,Password,Folder,Tags\nweb,10.0.0.1,22,ssh,root,pw,Prod/Web,a;b\n,srv2,3390,rdp,,,Prod/Web,\n,,1,ssh,,,,\nx,h,,bogus,,,,\n").unwrap();
        assert_eq!(p.connections.len(), 3);
        assert_eq!(p.folders.len(), 2);
        let web = &p.connections[0];
        assert_eq!((web.port, web.tags.len()), (None, 2));
        assert_eq!(p.connections[1].name, "srv2");
        assert_eq!(p.connections[1].port, Some(3390));
        assert_eq!(p.connections[2].protocol, Protocol::Ssh);
        assert_eq!(p.connections[0].folder_id, p.connections[1].folder_id);
        assert!(p.report.iter().any(|r| r.message.contains("plaintext")));
        assert!(parse_csv("name\nx").is_err());
    }

    #[test]
    fn openssh_hosts_jumps_forwards() {
        let cfg = "Host *\n  User admin\nHost bastion\n  HostName b.example.com\n  Port 2200\nHost app\n  HostName 10.1.1.1\n  ProxyJump bastion,jump@j.example.com:2022\n  LocalForward 8080 localhost:80\n  IdentityFile ~/.ssh/id\nHost *.wild\n  User x\nMatch host foo\n  User y\n";
        let p = parse_openssh(cfg);
        assert_eq!(p.connections.iter().filter(|c| c.tags.is_empty()).count(), 2);
        let app = p.connections.iter().find(|c| c.name == "app").unwrap();
        assert_eq!(app.options["key_file"], "~/.ssh/id");
        assert!(app.options.contains_key("tunnel_profile"));
        assert_eq!(p.connections.iter().find(|c| c.name == "bastion").unwrap().port, Some(2200));
        assert_eq!(p.secrets[&app.id].username.as_deref(), Some("admin"));
        let jump = p.tunnels.iter().find(|t| t.name.starts_with("Jump via")).unwrap();
        assert_eq!(jump.chain.len(), 2);
        let fw = p.tunnels.iter().find(|t| t.name == "app forwards").unwrap();
        assert!(fw.auto_start);
        assert!(p.report.iter().any(|r| r.item == "Match"));
    }

    #[test]
    fn json_roundtrip_fixes_orphans() {
        let doc = json!({"format":"okremote-export","version":1,"folders":[
            {"id":"b","parent_id":"a","name":"B","version":4},{"id":"a","parent_id":"zz","name":"A","version":2}],
            "connections":[{"id":"c","folder_id":"nope","name":"c","protocol":"ssh","host":"h","version":3,"has_secret":true}]});
        let p = parse_json(&doc.to_string()).unwrap();
        assert_eq!(p.folders[0].id, "a");
        assert!(p.folders.iter().all(|f| f.version == 0));
        assert_eq!(p.connections[0].folder_id, None);
        assert!(!p.connections[0].has_secret);
        assert!(parse_json("{}").is_err());
    }

    #[test]
    fn putty_sessions() {
        let mk = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<HashMap<_, _>>();
        let p = parse_putty(vec![
            ("web%20box".into(), mk(&[("Protocol", "ssh"), ("HostName", "bob@10.0.0.2"), ("PortNumber", "2200"), ("PublicKeyFile", "C:\\k.ppk"), ("PingIntervalSecs", "20")])),
            ("Default%20Settings".into(), mk(&[("HostName", "x")])),
            ("tel".into(), mk(&[("Protocol", "telnet"), ("HostName", "t")])),
            ("empty".into(), mk(&[("Protocol", "ssh")])),
        ]);
        assert_eq!(p.connections.len(), 1);
        let c = &p.connections[0];
        assert_eq!((c.name.as_str(), c.host.as_str(), c.port), ("web box", "10.0.0.2", Some(2200)));
        assert_eq!(p.secrets[&c.id].username.as_deref(), Some("bob"));
        assert_eq!(c.options["keepalive_seconds"], 20);
        assert!(p.report.iter().any(|r| r.message.contains("PuTTYgen")));
        assert!(p.report.iter().any(|r| r.message.contains("telnet")));
    }

    #[test]
    fn rdg_groups_servers_credentials() {
        let xml = r#"<RDCMan><file><group><properties><name>Prod</name></properties>
          <server><properties><name>srv1</name><comment>db</comment></properties><displayName>SRV One</displayName>
            <logonCredentials inherit="None"><userName>admin</userName><domain>CORP</domain><password>AAAA</password></logonCredentials>
            <connectionSettings><port>3390</port></connectionSettings></server>
          <server><properties><name>srv2</name></properties><logonCredentials inherit="FromParent"><userName>x</userName></logonCredentials></server>
        </group></file></RDCMan>"#;
        let p = parse_rdg(xml).unwrap();
        assert_eq!((p.folders.len(), p.connections.len()), (1, 2));
        let c = &p.connections[0];
        assert_eq!((c.name.as_str(), c.port, c.notes.as_str()), ("SRV One", Some(3390), "db"));
        assert_eq!(p.secrets[&c.id].domain.as_deref(), Some("CORP"));
        assert!(p.secrets.get(&p.connections[1].id).is_none());
        assert!(p.report.iter().any(|r| r.message.contains("cannot be imported")));
        assert!(parse_rdg("<Other/>").is_err());
    }

    #[test]
    fn decodes_utf16_and_bom() {
        assert_eq!(decode_text(b"\xEF\xBB\xBFhi").unwrap(), "hi");
        assert_eq!(decode_text(&[0xFF, 0xFE, b'h', 0, b'i', 0]).unwrap(), "hi");
    }
}
