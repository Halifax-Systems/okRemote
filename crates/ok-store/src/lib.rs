//! SQLite vault (spec section 4): schema, key hierarchy, versioned saves, host keys, audit, backup.
use chrono::Utc;
use ok_crypto::{aad, derive_kek, open, seal, Key, KdfParams, CANARY_PLAINTEXT};
use ok_model::*;
use rusqlite::{params, Connection as Db, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const APPLICATION_ID: i64 = 0x4F4B524D;
pub const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("the workspace is locked")] Locked,
    #[error("the workspace is open read-only")] ReadOnly,
    #[error("wrong passphrase")] WrongPassphrase,
    #[error("not an okRemote vault file")] NotAVault,
    #[error("this vault was written by a newer version of okRemote")] TooNew,
    #[error("not found")] NotFound,
    #[error("{0}")] Invalid(String),
    #[error("the item was changed elsewhere")] Conflict(Value),
    #[error("{0}")] Storage(String),
}

impl From<rusqlite::Error> for StoreError { fn from(e: rusqlite::Error) -> Self { Self::Storage(e.to_string()) } }
impl From<ok_crypto::CryptoError> for StoreError {
    fn from(e: ok_crypto::CryptoError) -> Self { match e { ok_crypto::CryptoError::Auth => Self::WrongPassphrase, o => Self::Storage(o.to_string()) } }
}
impl From<std::io::Error> for StoreError { fn from(e: std::io::Error) -> Self { Self::Storage(e.to_string()) } }
impl From<serde_json::Error> for StoreError { fn from(e: serde_json::Error) -> Self { Self::Storage(e.to_string()) } }

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceInfo {
    pub id: String,
    pub name: String,
    pub path: String,
    pub locked: bool,
    pub read_only: bool,
}

pub struct Vault {
    db: Db,
    path: PathBuf,
    pub id: String,
    pub name: String,
    key: Option<Key>,
    read_only: bool,
    actor: String,
}

const SCHEMA: &str = "
CREATE TABLE workspace (id TEXT PRIMARY KEY, name TEXT NOT NULL, schema_version INTEGER NOT NULL, kdf_params TEXT NOT NULL, wrapped_key TEXT NOT NULL, created_at TEXT NOT NULL);
CREATE TABLE folders (id TEXT PRIMARY KEY, parent_id TEXT REFERENCES folders(id) ON DELETE CASCADE, name TEXT NOT NULL, color TEXT, icon TEXT, defaults TEXT NOT NULL DEFAULT '{}', secret_blob TEXT, sort INTEGER NOT NULL DEFAULT 0, version INTEGER NOT NULL DEFAULT 1);
CREATE INDEX folders_parent ON folders(parent_id);
CREATE TABLE connections (id TEXT PRIMARY KEY, folder_id TEXT REFERENCES folders(id) ON DELETE CASCADE, name TEXT NOT NULL, protocol TEXT NOT NULL, host TEXT NOT NULL DEFAULT '', port INTEGER, options TEXT NOT NULL DEFAULT '{}', tags TEXT NOT NULL DEFAULT '[]', notes TEXT NOT NULL DEFAULT '', icon TEXT, secret_blob TEXT, sort INTEGER NOT NULL DEFAULT 0, version INTEGER NOT NULL DEFAULT 1);
CREATE INDEX connections_folder ON connections(folder_id);
CREATE TABLE tunnels (id TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL DEFAULT 'profile', spec TEXT NOT NULL DEFAULT '{}', chain TEXT NOT NULL DEFAULT '[]', version INTEGER NOT NULL DEFAULT 1);
CREATE TABLE terminal_profiles (id TEXT PRIMARY KEY, name TEXT NOT NULL, spec TEXT NOT NULL DEFAULT '{}', version INTEGER NOT NULL DEFAULT 1);
CREATE TABLE host_keys (host TEXT NOT NULL, port INTEGER NOT NULL, key_type TEXT NOT NULL, fingerprint TEXT NOT NULL, first_seen TEXT NOT NULL, trusted_by TEXT NOT NULL, PRIMARY KEY (host, port, key_type));
CREATE TABLE audit_log (id TEXT PRIMARY KEY, at TEXT NOT NULL, actor TEXT NOT NULL, action TEXT NOT NULL, target_id TEXT NOT NULL DEFAULT '', detail TEXT NOT NULL DEFAULT '{}');
CREATE INDEX audit_at ON audit_log(at);
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE users (id TEXT PRIMARY KEY, name TEXT);
CREATE TABLE groups (id TEXT PRIMARY KEY, name TEXT);
CREATE TABLE group_members (group_id TEXT, user_id TEXT);
CREATE TABLE folder_keys (folder_id TEXT, user_id TEXT, wrapped_key TEXT);
CREATE TABLE grants (id TEXT PRIMARY KEY, folder_id TEXT, subject TEXT, role TEXT);
";

fn now() -> String { Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string() }
pub fn os_actor() -> String {
    std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "local".into())
}

impl Vault {
    pub fn create(path: &Path, name: &str, passphrase: &str) -> Result<Vault> {
        if path.exists() { return Err(StoreError::Invalid("a file already exists at that path".into())); }
        if passphrase.is_empty() { return Err(StoreError::Invalid("a passphrase is required".into())); }
        if let Some(p) = path.parent() { std::fs::create_dir_all(p)?; }
        let params = ok_crypto::calibrate();
        let kek = derive_kek(passphrase, &params)?;
        let vault_key = Key::random();
        let id = new_id();
        let wrapped = seal(&kek, vault_key.as_bytes(), &aad(&id, "wrapped_key"));
        let canary = seal(&vault_key, CANARY_PLAINTEXT, &aad(&id, "canary"));
        let mut db = Db::open(path)?;
        db.execute_batch(&format!("PRAGMA application_id = {APPLICATION_ID}; PRAGMA user_version = {SCHEMA_VERSION}; PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;"))?;
        let tx = db.transaction()?;
        tx.execute_batch(SCHEMA)?;
        tx.execute("INSERT INTO workspace VALUES (?,?,?,?,?,?)",
            params![id, name, SCHEMA_VERSION, serde_json::to_string(&params)?, wrapped, now()])?;
        tx.execute("INSERT INTO settings VALUES ('canary', ?)", params![canary])?;
        tx.commit()?;
        let v = Vault { db, path: path.into(), id, name: name.into(), key: Some(vault_key), read_only: false, actor: os_actor() };
        v.db.busy_timeout(std::time::Duration::from_secs(5))?;
        v.audit("workspace.create", &v.id.clone(), json!({"name": name}))?;
        Ok(v)
    }

    /// Opens a vault file locked, or unlocked when `cached_key` passes the canary check.
    pub fn open(path: &Path, cached_key: Option<Key>) -> Result<Vault> {
        let db = Db::open(path).map_err(|_| StoreError::NotAVault)?;
        let app: i64 = db.query_row("PRAGMA application_id", [], |r| r.get(0)).map_err(|_| StoreError::NotAVault)?;
        if app != APPLICATION_ID { return Err(StoreError::NotAVault); }
        let ver: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let read_only = ver > SCHEMA_VERSION;
        if ver < SCHEMA_VERSION {
            let stamp = Utc::now().format("%Y%m%d-%H%M%S");
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("vault");
            std::fs::copy(path, path.with_file_name(format!("{stem}.pre-migration-{stamp}.okvault")))?;
            db.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))?;
        }
        db.execute_batch("PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        let (id, name): (String, String) = db.query_row("SELECT id, name FROM workspace", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|_| StoreError::NotAVault)?;
        let mut v = Vault { db, path: path.into(), id, name, key: None, read_only, actor: os_actor() };
        if let Some(k) = cached_key { let _ = v.unlock_with_key(k); }
        Ok(v)
    }

    fn check_canary(&self, key: &Key) -> bool {
        let blob: Option<String> = self.db.query_row("SELECT value FROM settings WHERE key='canary'", [], |r| r.get(0)).optional().ok().flatten();
        blob.and_then(|b| open(key, &b, &aad(&self.id, "canary")).ok()).as_deref() == Some(CANARY_PLAINTEXT)
    }

    pub fn unlock_with_key(&mut self, key: Key) -> Result<()> {
        if !self.check_canary(&key) { return Err(StoreError::WrongPassphrase); }
        self.key = Some(key);
        Ok(())
    }

    pub fn unlock(&mut self, passphrase: &str) -> Result<()> {
        let (kdf, wrapped): (String, String) = self.db.query_row("SELECT kdf_params, wrapped_key FROM workspace", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let params: KdfParams = serde_json::from_str(&kdf)?;
        let kek = derive_kek(passphrase, &params).map_err(|e| StoreError::Storage(e.to_string()))?;
        let raw = open(&kek, &wrapped, &aad(&self.id, "wrapped_key")).map_err(|_| StoreError::WrongPassphrase)?;
        self.key = Some(Key::from_bytes(&raw)?);
        Ok(())
    }

    pub fn lock(&mut self) { self.key = None; }
    pub fn is_locked(&self) -> bool { self.key.is_none() }
    pub fn is_read_only(&self) -> bool { self.read_only }
    pub fn path(&self) -> &Path { &self.path }
    pub fn export_key(&self) -> Result<String> { Ok(self.key()?.to_base64()) }
    pub fn info(&self) -> WorkspaceInfo {
        WorkspaceInfo { id: self.id.clone(), name: self.name.clone(), path: self.path.display().to_string(), locked: self.is_locked(), read_only: self.read_only }
    }

    fn key(&self) -> Result<&Key> { self.key.as_ref().ok_or(StoreError::Locked) }
    fn writable(&self) -> Result<()> {
        self.key()?;
        if self.read_only { Err(StoreError::ReadOnly) } else { Ok(()) }
    }

    pub fn change_passphrase(&mut self, old: &str, new: &str) -> Result<()> {
        self.writable()?;
        let (kdf, wrapped): (String, String) = self.db.query_row("SELECT kdf_params, wrapped_key FROM workspace", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let params: KdfParams = serde_json::from_str(&kdf)?;
        let kek = derive_kek(old, &params).map_err(|e| StoreError::Storage(e.to_string()))?;
        open(&kek, &wrapped, &aad(&self.id, "wrapped_key")).map_err(|_| StoreError::WrongPassphrase)?;
        let np = ok_crypto::calibrate();
        let nk = derive_kek(new, &np).map_err(|e| StoreError::Storage(e.to_string()))?;
        let rewrapped = seal(&nk, self.key()?.as_bytes(), &aad(&self.id, "wrapped_key"));
        self.db.execute("UPDATE workspace SET kdf_params=?, wrapped_key=?", params![serde_json::to_string(&np)?, rewrapped])?;
        self.audit("workspace.passphrase", &self.id.clone(), json!({}))
    }

    pub fn rename(&mut self, name: &str) -> Result<()> {
        self.writable()?;
        self.db.execute("UPDATE workspace SET name=?", params![name])?;
        self.name = name.into();
        Ok(())
    }

    // ------------------------------------------------------------ audit

    pub fn audit(&self, action: &str, target: &str, detail: Value) -> Result<()> {
        if self.read_only { return Ok(()); }
        self.db.execute("INSERT INTO audit_log VALUES (?,?,?,?,?,?)",
            params![new_id(), now(), self.actor, action, target, detail.to_string()])?;
        Ok(())
    }

    pub fn audit_list(&self, limit: usize) -> Result<Vec<AuditEntry>> {
        let mut st = self.db.prepare("SELECT id, at, actor, action, target_id, detail FROM audit_log ORDER BY at DESC, id DESC LIMIT ?")?;
        let rows = st.query_map(params![limit as i64], |r| {
            let d: String = r.get(5)?;
            Ok(AuditEntry { id: r.get(0)?, at: r.get(1)?, actor: r.get(2)?, action: r.get(3)?, target_id: r.get(4)?, detail: serde_json::from_str(&d).unwrap_or(Value::Null) })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    // ------------------------------------------------------------ snapshot

    pub fn snapshot(&self) -> Result<Snapshot> {
        self.key()?;
        let mut snap = Snapshot::default();
        let mut st = self.db.prepare("SELECT id,parent_id,name,color,icon,defaults,secret_blob IS NOT NULL,sort,version FROM folders")?;
        snap.folders = st.query_map([], |r| Ok(Folder {
            id: r.get(0)?, parent_id: r.get(1)?, name: r.get(2)?, color: r.get(3)?, icon: r.get(4)?,
            defaults: serde_json::from_str::<Value>(&r.get::<_, String>(5)?).ok().and_then(|v| v.as_object().cloned()).unwrap_or_default(),
            has_secret: r.get(6)?, sort: r.get(7)?, version: r.get(8)?,
        }))?.collect::<std::result::Result<_, _>>()?;
        let mut st = self.db.prepare("SELECT id,folder_id,name,protocol,host,port,options,tags,notes,icon,secret_blob IS NOT NULL,sort,version FROM connections")?;
        snap.connections = st.query_map([], |r| Ok(Connection {
            id: r.get(0)?, folder_id: r.get(1)?, name: r.get(2)?,
            protocol: Protocol::parse(&r.get::<_, String>(3)?).unwrap_or(Protocol::Ssh),
            host: r.get(4)?, port: r.get::<_, Option<i64>>(5)?.map(|p| p as u16),
            options: serde_json::from_str::<Value>(&r.get::<_, String>(6)?).ok().and_then(|v| v.as_object().cloned()).unwrap_or_default(),
            tags: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or_default(),
            notes: r.get(8)?, icon: r.get(9)?, has_secret: r.get(10)?, sort: r.get(11)?, version: r.get(12)?,
        }))?.collect::<std::result::Result<_, _>>()?;
        let mut st = self.db.prepare("SELECT id,name,spec,chain,version FROM tunnels")?;
        snap.tunnels = st.query_map([], |r| {
            let spec: Value = serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or(Value::Null);
            Ok(TunnelProfile {
                id: r.get(0)?, name: r.get(1)?,
                chain: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_default(),
                forwards: serde_json::from_value(spec.get("forwards").cloned().unwrap_or(json!([]))).unwrap_or_default(),
                auto_start: spec.get("auto_start").and_then(Value::as_bool).unwrap_or(false),
                version: r.get(4)?,
            })
        })?.collect::<std::result::Result<_, _>>()?;
        let mut st = self.db.prepare("SELECT id,name,spec,version FROM terminal_profiles")?;
        snap.terminal_profiles = st.query_map([], |r| Ok(TerminalProfile {
            id: r.get(0)?, name: r.get(1)?, spec: serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or(Value::Null), version: r.get(3)?,
        }))?.collect::<std::result::Result<_, _>>()?;
        Ok(snap)
    }

    // ------------------------------------------------------------ saves

    fn check_cycle(&self, id: &str, parent: Option<&str>) -> Result<()> {
        let mut cur = parent.map(str::to_string);
        for _ in 0..4096 {
            let Some(c) = cur else { return Ok(()) };
            if c == id { return Err(StoreError::Invalid("A folder cannot be moved into itself".into())); }
            cur = self.db.query_row("SELECT parent_id FROM folders WHERE id=?", params![c], |r| r.get(0)).optional()?.flatten();
        }
        Ok(())
    }

    pub fn folder_get(&self, id: &str) -> Result<Option<Folder>> {
        Ok(self.snapshot()?.folders.into_iter().find(|f| f.id == id))
    }
    pub fn conn_get(&self, id: &str) -> Result<Option<Connection>> {
        Ok(self.snapshot()?.connections.into_iter().find(|c| c.id == id))
    }

    pub fn folder_save(&self, mut f: Folder) -> Result<Folder> {
        self.writable()?;
        f.name = f.name.trim().to_string();
        if f.name.is_empty() { return Err(StoreError::Invalid("folder name is required".into())); }
        self.check_cycle(&f.id, f.parent_id.as_deref())?;
        let before = self.folder_get(&f.id)?;
        let defaults = serde_json::to_string(&f.defaults)?;
        if f.version == 0 {
            self.db.execute("INSERT INTO folders (id,parent_id,name,color,icon,defaults,sort,version) VALUES (?,?,?,?,?,?,?,1)",
                params![f.id, f.parent_id, f.name, f.color, f.icon, defaults, f.sort])?;
            f.version = 1;
            self.audit("folder.create", &f.id, json!({"after": f}))?;
        } else {
            let n = self.db.execute("UPDATE folders SET parent_id=?,name=?,color=?,icon=?,defaults=?,sort=?,version=version+1 WHERE id=? AND version=?",
                params![f.parent_id, f.name, f.color, f.icon, defaults, f.sort, f.id, f.version])?;
            if n == 0 {
                return Err(match before { Some(cur) => StoreError::Conflict(serde_json::to_value(cur)?), None => StoreError::NotFound });
            }
            f.version += 1;
            self.audit("folder.edit", &f.id, json!({"before": before, "after": f}))?;
        }
        f.has_secret = self.db.query_row("SELECT secret_blob IS NOT NULL FROM folders WHERE id=?", params![f.id], |r| r.get(0))?;
        Ok(f)
    }

    pub fn conn_save(&self, mut c: Connection) -> Result<Connection> {
        self.writable()?;
        c.name = c.name.trim().to_string();
        if c.name.is_empty() { return Err(StoreError::Invalid("connection name is required".into())); }
        let before = self.conn_get(&c.id)?;
        let (opts, tags) = (serde_json::to_string(&c.options)?, serde_json::to_string(&c.tags)?);
        if c.version == 0 {
            self.db.execute("INSERT INTO connections (id,folder_id,name,protocol,host,port,options,tags,notes,icon,sort,version) VALUES (?,?,?,?,?,?,?,?,?,?,?,1)",
                params![c.id, c.folder_id, c.name, c.protocol.as_str(), c.host, c.port.map(i64::from), opts, tags, c.notes, c.icon, c.sort])?;
            c.version = 1;
            self.audit("connection.create", &c.id, json!({"after": c}))?;
        } else {
            let n = self.db.execute("UPDATE connections SET folder_id=?,name=?,protocol=?,host=?,port=?,options=?,tags=?,notes=?,icon=?,sort=?,version=version+1 WHERE id=? AND version=?",
                params![c.folder_id, c.name, c.protocol.as_str(), c.host, c.port.map(i64::from), opts, tags, c.notes, c.icon, c.sort, c.id, c.version])?;
            if n == 0 {
                return Err(match before { Some(cur) => StoreError::Conflict(serde_json::to_value(cur)?), None => StoreError::NotFound });
            }
            c.version += 1;
            self.audit("connection.edit", &c.id, json!({"before": before, "after": c}))?;
        }
        c.has_secret = self.db.query_row("SELECT secret_blob IS NOT NULL FROM connections WHERE id=?", params![c.id], |r| r.get(0))?;
        Ok(c)
    }

    pub fn folder_delete(&self, id: &str) -> Result<()> {
        self.writable()?;
        let before = self.folder_get(id)?;
        self.db.execute("DELETE FROM folders WHERE id=?", params![id])?;
        self.audit("folder.delete", id, json!({"before": before}))
    }

    pub fn conn_delete(&self, id: &str) -> Result<()> {
        self.writable()?;
        let before = self.conn_get(id)?;
        self.db.execute("DELETE FROM connections WHERE id=?", params![id])?;
        self.audit("connection.delete", id, json!({"before": before}))
    }

    // ------------------------------------------------------------ secrets

    fn table(owner: &str) -> Result<&'static str> {
        match owner { "folder" => Ok("folders"), "connection" => Ok("connections"), _ => Err(StoreError::Invalid("unknown owner kind".into())) }
    }

    /// The record's own stored secret (no inheritance). Not audited here; callers audit reveal/copy.
    pub fn secret_get(&self, owner: &str, id: &str) -> Result<Secret> {
        let key = self.key()?;
        let blob: Option<String> = self.db
            .query_row(&format!("SELECT secret_blob FROM {} WHERE id=?", Self::table(owner)?), params![id], |r| r.get(0))
            .optional()?.ok_or(StoreError::NotFound)?;
        match blob {
            None => Ok(Secret::default()),
            Some(b) => Ok(serde_json::from_slice(&open(key, &b, &aad(id, "secret"))?)?),
        }
    }

    pub fn secret_set(&self, owner: &str, id: &str, secret: Option<Secret>) -> Result<()> {
        self.writable()?;
        let key = self.key()?;
        let s = secret.map(Secret::normalized).filter(|s| !s.is_empty());
        let cleared = s.is_none();
        let blob = match &s { None => None, Some(s) => Some(seal(key, &serde_json::to_vec(s)?, &aad(id, "secret"))) };
        let n = self.db.execute(&format!("UPDATE {} SET secret_blob=? WHERE id=?", Self::table(owner)?), params![blob, id])?;
        if n == 0 { return Err(StoreError::NotFound); }
        self.audit("secret.change", id, json!({"owner": owner, "cleared": cleared}))
    }

    /// Patch: Some(Some(v)) sets, Some(None) clears, absent leaves unchanged.
    pub fn secret_patch(&self, owner: &str, id: &str, patch: &serde_json::Map<String, Value>) -> Result<()> {
        let mut s = self.secret_get(owner, id)?;
        for f in SECRET_FIELDS {
            match patch.get(f) {
                Some(Value::String(v)) => s.set(f, Some(v.clone())),
                Some(Value::Null) => s.set(f, None),
                _ => {}
            }
        }
        self.secret_set(owner, id, Some(s))
    }

    /// Effective secret as a connection would see it: own fields, then nearest ancestor first.
    pub fn secret_effective(&self, snap: &Snapshot, conn_id: &str) -> Result<Secret> {
        let c = snap.connection(conn_id).ok_or(StoreError::NotFound)?;
        let mut s = self.secret_get("connection", conn_id)?;
        for f in snap.ancestors(c.folder_id.as_deref()) {
            s.inherit_from(&self.secret_get("folder", &f.id)?);
        }
        Ok(s)
    }

    /// Where each field comes from, without revealing secret values.
    pub fn secret_summary(&self, snap: &Snapshot, owner: &str, id: &str) -> Result<Value> {
        let own = self.secret_get(owner, id)?;
        let start = if owner == "folder" { snap.folder(id).and_then(|f| f.parent_id.clone()) } else { snap.connection(id).and_then(|c| c.folder_id.clone()) };
        let chain = snap.ancestors(start.as_deref());
        let mut out = serde_json::Map::new();
        for f in SECRET_FIELDS {
            let show_value = matches!(f, "username" | "domain");
            if let Some(v) = own.get(f) {
                out.insert(f.into(), json!({"own": true, "inherited_from": null, "value": show_value.then(|| v.clone())}));
                continue;
            }
            let mut found = Value::Null;
            for a in &chain {
                if let Some(v) = self.secret_get("folder", &a.id)?.get(f) {
                    found = json!({"own": false, "inherited_from": a.name, "value": show_value.then(|| v.clone())});
                    break;
                }
            }
            out.insert(f.into(), if found.is_null() { json!({"own": false, "inherited_from": null, "value": null}) } else { found });
        }
        Ok(Value::Object(out))
    }

    // ------------------------------------------------------------ tunnels & profiles

    pub fn tunnel_save(&self, mut t: TunnelProfile) -> Result<TunnelProfile> {
        self.writable()?;
        let spec = serde_json::to_string(&json!({"forwards": t.forwards, "auto_start": t.auto_start}))?;
        let chain = serde_json::to_string(&t.chain)?;
        if t.version == 0 {
            self.db.execute("INSERT INTO tunnels (id,name,kind,spec,chain,version) VALUES (?,?,'profile',?,?,1)", params![t.id, t.name, spec, chain])?;
            t.version = 1;
        } else {
            let n = self.db.execute("UPDATE tunnels SET name=?,spec=?,chain=?,version=version+1 WHERE id=? AND version=?", params![t.name, spec, chain, t.id, t.version])?;
            if n == 0 { return Err(StoreError::NotFound); }
            t.version += 1;
        }
        self.audit("tunnel.save", &t.id, json!({"after": t}))?;
        Ok(t)
    }
    pub fn tunnel_delete(&self, id: &str) -> Result<()> {
        self.writable()?;
        self.db.execute("DELETE FROM tunnels WHERE id=?", params![id])?;
        self.audit("tunnel.delete", id, json!({}))
    }
    pub fn profile_save(&self, mut p: TerminalProfile) -> Result<TerminalProfile> {
        self.writable()?;
        let spec = serde_json::to_string(&p.spec)?;
        if p.version == 0 {
            self.db.execute("INSERT INTO terminal_profiles VALUES (?,?,?,1)", params![p.id, p.name, spec])?;
            p.version = 1;
        } else {
            let n = self.db.execute("UPDATE terminal_profiles SET name=?,spec=?,version=version+1 WHERE id=? AND version=?", params![p.name, spec, p.id, p.version])?;
            if n == 0 { return Err(StoreError::NotFound); }
            p.version += 1;
        }
        self.audit("profile.save", &p.id, json!({"name": p.name}))?;
        Ok(p)
    }
    pub fn profile_delete(&self, id: &str) -> Result<()> {
        self.writable()?;
        self.db.execute("DELETE FROM terminal_profiles WHERE id=?", params![id])?;
        self.audit("profile.delete", id, json!({}))
    }

    // ------------------------------------------------------------ host keys

    pub fn hostkeys(&self) -> Result<Vec<HostKey>> {
        let mut st = self.db.prepare("SELECT host,port,key_type,fingerprint,first_seen,trusted_by FROM host_keys ORDER BY host, port")?;
        let rows = st.query_map([], |r| Ok(HostKey { host: r.get(0)?, port: r.get::<_, i64>(1)? as u16, key_type: r.get(2)?, fingerprint: r.get(3)?, first_seen: r.get(4)?, trusted_by: r.get(5)? }))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
    pub fn hostkey_trust(&self, host: &str, port: u16, key_type: &str, fp: &str) -> Result<()> {
        self.writable()?;
        let host = host.to_lowercase();
        self.db.execute("INSERT OR REPLACE INTO host_keys VALUES (?,?,?,?,?,?)", params![host, port, key_type, fp, now(), self.actor])?;
        self.audit("hostkey.trust", &format!("{host}:{port}"), json!({"key_type": key_type, "fingerprint": fp}))
    }
    pub fn hostkey_delete(&self, host: &str, port: u16, key_type: &str) -> Result<()> {
        self.writable()?;
        let host = host.to_lowercase();
        self.db.execute("DELETE FROM host_keys WHERE host=? AND port=? AND key_type=?", params![host, port, key_type])?;
        self.audit("hostkey.remove", &format!("{host}:{port}"), json!({"key_type": key_type}))
    }

    // ------------------------------------------------------------ backup

    /// VACUUM INTO a new file; safe while open.
    pub fn backup_to(&self, dest: &Path) -> Result<()> {
        if let Some(p) = dest.parent() { std::fs::create_dir_all(p)?; }
        self.db.execute("VACUUM INTO ?", params![dest.to_string_lossy()])?;
        Ok(())
    }

    /// Timestamped backup under `<config>/backups/<id>/`, pruned to `keep` newest (0 = off).
    pub fn backup_rotating(&self, config_dir: &Path, keep: usize) -> Result<Option<PathBuf>> {
        if keep == 0 { return Ok(None); }
        let dir = config_dir.join("backups").join(&self.id);
        let file = dir.join(format!("{}.okvault", Utc::now().format("%Y%m%d-%H%M%S")));
        self.backup_to(&file)?;
        let mut all: Vec<_> = std::fs::read_dir(&dir)?.filter_map(|e| e.ok()).map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "okvault")).collect();
        all.sort();
        while all.len() > keep { let _ = std::fs::remove_file(all.remove(0)); }
        Ok(Some(dir))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast_vault(dir: &Path) -> Vault { Vault::create(&dir.join("t.okvault"), "T", "correct horse").unwrap() }

    #[test]
    fn create_lock_unlock_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let mut v = fast_vault(d.path());
        let f = v.folder_save(Folder { id: new_id(), parent_id: None, name: "Example".into(), color: None, icon: None, defaults: Default::default(), has_secret: false, sort: 0, version: 0 }).unwrap();
        v.secret_set("folder", &f.id, Some(Secret::default().with("username", "root").with("password", "pw"))).unwrap();
        v.lock();
        assert!(matches!(v.snapshot(), Err(StoreError::Locked)));
        drop(v);
        let mut v = Vault::open(&d.path().join("t.okvault"), None).unwrap();
        assert!(v.is_locked());
        assert!(matches!(v.unlock("nope"), Err(StoreError::WrongPassphrase)));
        v.unlock("correct horse").unwrap();
        assert_eq!(v.secret_get("folder", &f.id).unwrap().password.as_deref(), Some("pw"));
        assert!(v.folder_get(&f.id).unwrap().unwrap().has_secret);
        let k = v.export_key().unwrap();
        let v2 = Vault::open(&d.path().join("t.okvault"), Some(Key::from_base64(&k).unwrap())).unwrap();
        assert!(!v2.is_locked());
    }

    #[test]
    fn version_conflicts_and_cycles() {
        let d = tempfile::tempdir().unwrap();
        let v = fast_vault(d.path());
        let a = v.folder_save(Folder { id: new_id(), parent_id: None, name: "a".into(), color: None, icon: None, defaults: Default::default(), has_secret: false, sort: 0, version: 0 }).unwrap();
        let b = v.folder_save(Folder { id: new_id(), parent_id: Some(a.id.clone()), name: "b".into(), color: None, icon: None, defaults: Default::default(), has_secret: false, sort: 0, version: 0 }).unwrap();
        let mut stale = a.clone();
        v.folder_save(Folder { name: "a2".into(), ..a.clone() }).unwrap();
        stale.name = "mine".into();
        assert!(matches!(v.folder_save(stale), Err(StoreError::Conflict(_))));
        assert!(v.folder_save(Folder { parent_id: Some(b.id.clone()), ..v.folder_get(&a.id).unwrap().unwrap() }).is_err());
        assert!(matches!(v.folder_save(Folder { name: " ".into(), ..b }), Err(StoreError::Invalid(_))));
    }

    #[test]
    fn secrets_inherit_and_cascade() {
        let d = tempfile::tempdir().unwrap();
        let v = fast_vault(d.path());
        let f = v.folder_save(Folder { id: new_id(), parent_id: None, name: "F".into(), color: None, icon: None, defaults: Default::default(), has_secret: false, sort: 0, version: 0 }).unwrap();
        let c = v.conn_save(Connection { id: new_id(), folder_id: Some(f.id.clone()), name: "c".into(), protocol: Protocol::Ssh, host: "h".into(), port: None, options: Default::default(), tags: vec![], notes: "".into(), icon: None, has_secret: false, sort: 0, version: 0 }).unwrap();
        v.secret_set("folder", &f.id, Some(Secret::default().with("username", "fu").with("password", "fp"))).unwrap();
        let patch = json!({"username": "me"}); 
        v.secret_patch("connection", &c.id, patch.as_object().unwrap()).unwrap();
        let snap = v.snapshot().unwrap();
        let eff = v.secret_effective(&snap, &c.id).unwrap();
        assert_eq!((eff.username.as_deref(), eff.password.as_deref()), (Some("me"), Some("fp")));
        let sum = v.secret_summary(&snap, "connection", &c.id).unwrap();
        assert_eq!(sum["password"]["inherited_from"], "F");
        assert!(sum["password"]["value"].is_null());
        v.secret_patch("connection", &c.id, json!({"username": null}).as_object().unwrap()).unwrap();
        assert!(!v.conn_get(&c.id).unwrap().unwrap().has_secret);
        v.folder_delete(&f.id).unwrap();
        assert!(v.conn_get(&c.id).unwrap().is_none());
        let log = v.audit_list(100).unwrap();
        assert!(log.iter().any(|e| e.action == "secret.change"));
        assert!(!serde_json::to_string(&log).unwrap().contains("fp"));
    }

    #[test]
    fn ciphertext_not_portable_between_rows() {
        let d = tempfile::tempdir().unwrap();
        let v = fast_vault(d.path());
        let mk = |n: &str| v.folder_save(Folder { id: new_id(), parent_id: None, name: n.into(), color: None, icon: None, defaults: Default::default(), has_secret: false, sort: 0, version: 0 }).unwrap();
        let (a, b) = (mk("a"), mk("b"));
        v.secret_set("folder", &a.id, Some(Secret::default().with("password", "x"))).unwrap();
        v.db.execute("UPDATE folders SET secret_blob=(SELECT secret_blob FROM folders WHERE id=?) WHERE id=?", params![a.id, b.id]).unwrap();
        assert!(v.secret_get("folder", &b.id).is_err());
    }

    #[test]
    fn backups_rotate_and_hostkeys() {
        let d = tempfile::tempdir().unwrap();
        let v = fast_vault(d.path());
        v.hostkey_trust("Example.COM", 22, "ssh-ed25519", "SHA256:abc").unwrap();
        assert_eq!(v.hostkeys().unwrap()[0].host, "example.com");
        v.hostkey_delete("example.com", 22, "ssh-ed25519").unwrap();
        assert!(v.hostkeys().unwrap().is_empty());
        assert!(v.backup_rotating(d.path(), 0).unwrap().is_none());
        let dir = v.backup_rotating(d.path(), 1).unwrap().unwrap();
        assert_eq!(std::fs::read_dir(dir).unwrap().count(), 1);
        assert!(matches!(Vault::open(&d.path().join("missing"), None), Err(_)));
    }
}
