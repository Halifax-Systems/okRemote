use crate::ssh;
use crate::state::*;
use ok_model::*;
use ok_store::Vault;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::State;
use tauri_plugin_clipboard_manager::ClipboardExt;

type S<'a> = State<'a, AppState>;

const KEYCHAIN_SERVICE: &str = "OkRemote";

fn keychain(ws: &str) -> Result<keyring::Entry, CmdErr> {
    keyring::Entry::new(KEYCHAIN_SERVICE, &format!("vault:{ws}")).map_err(|e| CmdErr::new("keychain", e.to_string()))
}

fn remember_key(ws: &str, key_b64: &str) -> CmdResult<()> {
    keychain(ws)?.set_password(key_b64).map_err(|e| CmdErr::new("keychain", e.to_string()))
}

fn push_recent(st: &AppState, path: &str) {
    let mut s = st.settings.lock().unwrap();
    if let Some(a) = s.get_mut("recent_workspaces").and_then(Value::as_array_mut) {
        a.retain(|v| v.as_str() != Some(path));
        a.insert(0, json!(path));
        a.truncate(12);
    }
}

// ------------------------------------------------------------------ app & settings

#[tauri::command]
pub async fn app_info(st: S<'_>) -> CmdResult<Value> {
    Ok(json!({
        "product": "okRemote", "version": env!("CARGO_PKG_VERSION"), "platform": std::env::consts::OS,
        "portable": crate::config_dir().ends_with("data"), "config_dir": st.config_dir.display().to_string(),
        "logs_dir": st.config_dir.join("logs").display().to_string(), "vault_ext": "okvault",
    }))
}

#[tauri::command]
pub async fn settings_get(st: S<'_>) -> CmdResult<Value> { Ok(st.settings_get()) }

#[tauri::command]
pub async fn settings_save(st: S<'_>, settings: Value) -> CmdResult<()> {
    if !settings.is_object() { return Err(CmdErr::new("invalid", "settings must be an object")); }
    let merged = merge_settings(&settings);
    write_atomic(&st.config_dir.join("settings.json"), serde_json::to_string_pretty(&merged)?.as_bytes())?;
    *st.settings.lock().unwrap() = merged;
    Ok(())
}

#[tauri::command]
pub async fn startup_requests(st: S<'_>) -> CmdResult<Vec<String>> { Ok(std::mem::take(&mut *st.startup_paths.lock().unwrap())) }

#[tauri::command]
pub async fn default_vault_path() -> CmdResult<String> {
    Ok(dirs::document_dir().unwrap_or_default().join("OkRemote").join("personal.okvault").display().to_string())
}

#[tauri::command]
pub async fn quick_parse(input: String) -> CmdResult<QuickTarget> { parse_quick(&input).map_err(|m| CmdErr::new("invalid", m)) }

#[tauri::command]
pub async fn prompt_respond(st: S<'_>, id: String, value: Value) -> CmdResult<()> {
    if let Some(tx) = st.prompts.lock().unwrap().remove(&id) { let _ = tx.send(value); }
    Ok(())
}

// ------------------------------------------------------------------ workspaces

#[tauri::command]
pub async fn ws_list(st: S<'_>) -> CmdResult<Vec<ok_store::WorkspaceInfo>> {
    let mut v: Vec<_> = st.vaults.lock().unwrap().values().map(|v| v.lock().unwrap().info()).collect();
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(v)
}

fn register(st: &AppState, v: Vault) -> ok_store::WorkspaceInfo {
    let info = v.info();
    push_recent(st, &info.path);
    st.vaults.lock().unwrap().insert(info.id.clone(), std::sync::Arc::new(std::sync::Mutex::new(v)));
    info
}

#[tauri::command]
pub async fn ws_create(st: S<'_>, path: String, name: String, passphrase: String, remember: bool) -> CmdResult<ok_store::WorkspaceInfo> {
    let (p, n) = (PathBuf::from(&path), name.clone());
    let v = tokio::task::spawn_blocking(move || Vault::create(&p, &n, &passphrase)).await.map_err(|e| CmdErr::new("error", e.to_string()))??;
    v.folder_save(Folder { id: new_id(), parent_id: None, name: "Example".into(), color: None, icon: None, defaults: Default::default(), has_secret: false, sort: 0, version: 0 })?;
    if remember { remember_key(&v.id, &v.export_key()?)?; }
    Ok(register(&st, v))
}

#[tauri::command]
pub async fn ws_open(st: S<'_>, path: String, passphrase: Option<String>, remember: Option<bool>) -> CmdResult<ok_store::WorkspaceInfo> {
    if let Some(existing) = st.vaults.lock().unwrap().values().map(|v| v.lock().unwrap().info()).find(|i| i.path == path) { return Ok(existing); }
    let p = PathBuf::from(&path);
    let mut v = Vault::open(&p, None)?;
    let id = v.id.clone();
    if let Some(pass) = passphrase.filter(|s| !s.is_empty()) {
        let v2 = tokio::task::spawn_blocking(move || v.unlock(&pass).map(|_| v)).await.map_err(|e| CmdErr::new("error", e.to_string()))??;
        v = v2;
        if remember.unwrap_or(false) { remember_key(&id, &v.export_key()?)?; }
    } else if let Ok(b64) = keychain(&id).and_then(|e| e.get_password().map_err(|e| CmdErr::new("keychain", e.to_string()))) {
        match ok_crypto::Key::from_base64(&b64).ok().map(|k| v.unlock_with_key(k)) {
            Some(Ok(())) => {}
            _ => { let _ = keychain(&id).map(|e| e.delete_credential()); }
        }
    }
    if !v.is_locked() { let _ = v.backup_rotating(&st.config_dir, st.setting_u64("backup_retention", 10) as usize); }
    Ok(register(&st, v))
}

#[tauri::command]
pub async fn ws_unlock(st: S<'_>, id: String, passphrase: String, remember: bool) -> CmdResult<ok_store::WorkspaceInfo> {
    let vault = st.vault(&id)?;
    let retention = st.setting_u64("backup_retention", 10) as usize;
    let dir = st.config_dir.clone();
    tokio::task::spawn_blocking(move || -> CmdResult<ok_store::WorkspaceInfo> {
        let mut v = vault.lock().unwrap();
        v.unlock(&passphrase)?;
        if remember { remember_key(&v.id, &v.export_key()?)?; }
        let _ = v.backup_rotating(&dir, retention);
        Ok(v.info())
    }).await.map_err(|e| CmdErr::new("error", e.to_string()))?
}

#[tauri::command]
pub async fn ws_lock(st: S<'_>, id: String) -> CmdResult<ok_store::WorkspaceInfo> { st.with_vault(&id, |v| { v.lock(); Ok(v.info()) }) }

#[tauri::command]
pub async fn ws_lock_all(st: S<'_>) -> CmdResult<()> {
    for v in st.vaults.lock().unwrap().values() { v.lock().unwrap().lock(); }
    Ok(())
}

#[tauri::command]
pub async fn ws_close(st: S<'_>, id: String) -> CmdResult<()> { st.vaults.lock().unwrap().remove(&id); Ok(()) }

#[tauri::command]
pub async fn ws_snapshot(st: S<'_>, id: String) -> CmdResult<Snapshot> { st.with_vault(&id, |v| v.snapshot()) }

#[tauri::command]
pub async fn ws_rename(st: S<'_>, id: String, name: String) -> CmdResult<ok_store::WorkspaceInfo> {
    st.with_vault(&id, |v| { v.rename(&name)?; Ok(v.info()) })
}

#[tauri::command]
pub async fn ws_change_passphrase(st: S<'_>, id: String, old: String, new: String) -> CmdResult<()> {
    let vault = st.vault(&id)?;
    tokio::task::spawn_blocking(move || -> CmdResult<()> {
        let mut v = vault.lock().unwrap();
        v.change_passphrase(&old, &new)?;
        if keychain(&v.id).and_then(|e| e.get_password().map_err(|e| CmdErr::new("keychain", e.to_string()))).is_ok() {
            remember_key(&v.id, &v.export_key()?)?;
        }
        Ok(())
    }).await.map_err(|e| CmdErr::new("error", e.to_string()))?
}

#[tauri::command]
pub async fn ws_forget_device(id: String) -> CmdResult<()> { let _ = keychain(&id)?.delete_credential(); Ok(()) }

#[tauri::command]
pub async fn ws_remembered(id: String) -> CmdResult<bool> { Ok(keychain(&id).map(|e| e.get_password().is_ok()).unwrap_or(false)) }

#[tauri::command]
pub async fn ws_backup_now(st: S<'_>, id: String) -> CmdResult<String> {
    let keep = st.setting_u64("backup_retention", 10).max(1) as usize;
    let dir = st.config_dir.clone();
    st.with_vault(&id, |v| v.backup_rotating(&dir, keep)).map(|p| p.map(|p| p.display().to_string()).unwrap_or_default())
}

// ------------------------------------------------------------------ records

#[tauri::command]
pub async fn folder_save(st: S<'_>, ws: String, folder: Folder) -> CmdResult<Folder> { st.with_vault(&ws, |v| v.folder_save(folder)) }
#[tauri::command]
pub async fn conn_save(st: S<'_>, ws: String, conn: Connection) -> CmdResult<Connection> { st.with_vault(&ws, |v| v.conn_save(conn)) }
#[tauri::command]
pub async fn folder_delete(st: S<'_>, ws: String, id: String) -> CmdResult<()> { st.with_vault(&ws, |v| v.folder_delete(&id)) }
#[tauri::command]
pub async fn conn_delete(st: S<'_>, ws: String, id: String) -> CmdResult<()> { st.with_vault(&ws, |v| v.conn_delete(&id)) }

fn dup_conn(v: &Vault, c: &Connection, parent: Option<String>, suffix: bool) -> Result<Connection, ok_store::StoreError> {
    let mut n = c.clone();
    n.id = new_id();
    n.folder_id = parent;
    n.version = 0;
    n.has_secret = false;
    if suffix { n.name = format!("{} (copy)", c.name); }
    n.options.remove("pinned");
    n.options.remove("auto_open");
    let saved = v.conn_save(n)?;
    let s = v.secret_get("connection", &c.id)?;
    if !s.is_empty() { v.secret_set("connection", &saved.id, Some(s))?; }
    v.conn_get(&saved.id)?.ok_or(ok_store::StoreError::NotFound)
}

fn dup_folder(v: &Vault, snap: &Snapshot, f: &Folder, parent: Option<String>, suffix: bool) -> Result<Folder, ok_store::StoreError> {
    let mut n = f.clone();
    n.id = new_id();
    n.parent_id = parent;
    n.version = 0;
    n.has_secret = false;
    if suffix { n.name = format!("{} (copy)", f.name); }
    let saved = v.folder_save(n)?;
    let s = v.secret_get("folder", &f.id)?;
    if !s.is_empty() { v.secret_set("folder", &saved.id, Some(s))?; }
    for c in snap.connections.iter().filter(|c| c.folder_id.as_deref() == Some(&f.id)) { dup_conn(v, c, Some(saved.id.clone()), false)?; }
    for sub in snap.folders.iter().filter(|x| x.parent_id.as_deref() == Some(&f.id)) { dup_folder(v, snap, sub, Some(saved.id.clone()), false)?; }
    v.folder_get(&saved.id)?.ok_or(ok_store::StoreError::NotFound)
}

/// `target`: absent keeps the same parent, empty text is top level, otherwise a folder id.
fn target_parent(target: Option<String>, same: Option<String>) -> Option<String> {
    match target { None => same, Some(t) if t.is_empty() => None, Some(t) => Some(t) }
}

#[tauri::command]
pub async fn conn_duplicate(st: S<'_>, ws: String, id: String, target: Option<String>) -> CmdResult<Connection> {
    st.with_vault(&ws, |v| {
        let c = v.conn_get(&id)?.ok_or(ok_store::StoreError::NotFound)?;
        let parent = target_parent(target.clone(), c.folder_id.clone());
        dup_conn(v, &c, parent, target.is_none())
    })
}

#[tauri::command]
pub async fn folder_duplicate(st: S<'_>, ws: String, id: String, target: Option<String>) -> CmdResult<Folder> {
    st.with_vault(&ws, |v| {
        let snap = v.snapshot()?;
        let f = snap.folder(&id).cloned().ok_or(ok_store::StoreError::NotFound)?;
        let parent = target_parent(target.clone(), f.parent_id.clone());
        dup_folder(v, &snap, &f, parent, target.is_none())
    })
}

#[derive(Deserialize, Clone)]
pub struct ItemRef { pub kind: String, pub id: String }

#[tauri::command]
pub async fn items_move(st: S<'_>, ws: String, items: Vec<ItemRef>, parent: Option<String>, before: Option<ItemRef>) -> CmdResult<()> {
    st.with_vault(&ws, |v| {
        let snap = v.snapshot()?;
        let moving = |kind: &str, id: &str| items.iter().any(|i| i.kind == kind && i.id == id);
        for i in items.iter().filter(|i| i.kind == "folder") {
            let mut cur = parent.clone();
            for _ in 0..4096 {
                match cur {
                    Some(c) if c == i.id => return Err(ok_store::StoreError::Invalid("A folder cannot be moved into itself".into())),
                    Some(c) => cur = snap.folder(&c).and_then(|f| f.parent_id.clone()),
                    None => break,
                }
            }
        }
        // folders
        let mut order: Vec<String> = {
            let mut f: Vec<&Folder> = snap.folders.iter().filter(|f| f.parent_id == parent && !moving("folder", &f.id)).collect();
            f.sort_by(|a, b| a.sort.cmp(&b.sort).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
            f.iter().map(|f| f.id.clone()).collect()
        };
        let moved: Vec<String> = items.iter().filter(|i| i.kind == "folder").map(|i| i.id.clone()).collect();
        let at = before.as_ref().filter(|b| b.kind == "folder").and_then(|b| order.iter().position(|x| *x == b.id)).unwrap_or(order.len());
        for (k, m) in moved.iter().enumerate() { order.insert(at + k, m.clone()); }
        for (i, id) in order.iter().enumerate() {
            if let Some(f) = snap.folder(id) {
                if f.sort != i as i64 || f.parent_id != parent { v.folder_save(Folder { sort: i as i64, parent_id: parent.clone(), ..f.clone() })?; }
            }
        }
        // connections
        let mut corder: Vec<String> = {
            let mut c: Vec<&Connection> = snap.connections.iter().filter(|c| c.folder_id == parent && !moving("connection", &c.id)).collect();
            c.sort_by(|a, b| a.sort.cmp(&b.sort).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
            c.iter().map(|c| c.id.clone()).collect()
        };
        let cmoved: Vec<String> = items.iter().filter(|i| i.kind == "connection").map(|i| i.id.clone()).collect();
        let at = before.as_ref().filter(|b| b.kind == "connection").and_then(|b| corder.iter().position(|x| *x == b.id)).unwrap_or(corder.len());
        for (k, m) in cmoved.iter().enumerate() { corder.insert(at + k, m.clone()); }
        for (i, id) in corder.iter().enumerate() {
            if let Some(c) = snap.connection(id) {
                if c.sort != i as i64 || c.folder_id != parent { v.conn_save(Connection { sort: i as i64, folder_id: parent.clone(), ..c.clone() })?; }
            }
        }
        Ok(())
    })
}

#[tauri::command]
pub async fn conns_set_option(st: S<'_>, ws: String, ids: Vec<String>, key: String, value: Value) -> CmdResult<usize> {
    st.with_vault(&ws, |v| {
        let mut n = 0;
        for id in ids {
            if let Some(mut c) = v.conn_get(&id)? {
                if value.is_null() { c.options.remove(&key); } else { c.options.insert(key.clone(), value.clone()); }
                v.conn_save(c)?;
                n += 1;
            }
        }
        Ok(n)
    })
}

// ------------------------------------------------------------------ secrets

fn secret_json(s: &Secret) -> Value { serde_json::to_value(s).unwrap_or(Value::Null) }

#[tauri::command]
pub async fn secret_get(st: S<'_>, ws: String, owner: String, id: String) -> CmdResult<Value> {
    st.with_vault(&ws, |v| {
        let s = v.secret_get(&owner, &id)?;
        v.audit("secret.reveal", &id, json!({"owner": owner}))?;
        Ok(secret_json(&s))
    })
}

#[tauri::command]
pub async fn secret_set(st: S<'_>, ws: String, owner: String, id: String, secret: Option<Value>) -> CmdResult<()> {
    let s = secret.and_then(|v| serde_json::from_value::<Secret>(v).ok());
    st.with_vault(&ws, |v| v.secret_set(&owner, &id, s))
}

#[tauri::command]
pub async fn secret_patch(st: S<'_>, ws: String, owner: String, id: String, patch: Map<String, Value>) -> CmdResult<()> {
    st.with_vault(&ws, |v| v.secret_patch(&owner, &id, &patch))
}

#[tauri::command]
pub async fn secret_summary(st: S<'_>, ws: String, owner: String, id: String) -> CmdResult<Value> {
    st.with_vault(&ws, |v| { let snap = v.snapshot()?; v.secret_summary(&snap, &owner, &id) })
}

fn effective_secret(v: &Vault, owner: &str, id: &str) -> Result<Secret, ok_store::StoreError> {
    let snap = v.snapshot()?;
    if owner == "connection" { return v.secret_effective(&snap, id); }
    let mut s = v.secret_get("folder", id)?;
    let start = snap.folder(id).and_then(|f| f.parent_id.clone());
    for a in snap.ancestors(start.as_deref()) { s.inherit_from(&v.secret_get("folder", &a.id)?); }
    Ok(s)
}

/// Writes a credential to the clipboard and clears it later if it is still there (17.2).
pub fn clipboard_with_clear(app: &tauri::AppHandle, value: String, seconds: u64) {
    let _ = app.clipboard().write_text(value.clone());
    if seconds == 0 { return; }
    let app = app.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(seconds)).await;
        if app.clipboard().read_text().ok().as_deref() == Some(value.as_str()) { let _ = app.clipboard().write_text(String::new()); }
    });
}

#[tauri::command]
pub async fn secret_copy(st: S<'_>, ws: String, owner: String, id: String, field: String) -> CmdResult<u64> {
    if !["username", "password", "domain", "private_key"].contains(&field.as_str()) {
        return Err(CmdErr::new("invalid", "that field cannot be copied"));
    }
    let value = st.with_vault(&ws, |v| {
        let s = effective_secret(v, &owner, &id)?;
        let val = s.get(&field).cloned().ok_or_else(|| ok_store::StoreError::Invalid(format!("no {} is set", field.replace('_', " "))))?;
        v.audit("secret.copy", &id, json!({"field": field}))?;
        Ok(val)
    })?;
    let secs = st.setting_u64("clipboard_clear_seconds", 30);
    clipboard_with_clear(&st.app, value, secs);
    Ok(secs)
}

// ------------------------------------------------------------------ tunnels, profiles, host keys, audit

#[tauri::command]
pub async fn tunnel_save(st: S<'_>, ws: String, tunnel: TunnelProfile) -> CmdResult<TunnelProfile> { st.with_vault(&ws, |v| v.tunnel_save(tunnel)) }
#[tauri::command]
pub async fn tunnel_delete(st: S<'_>, ws: String, id: String) -> CmdResult<()> { st.with_vault(&ws, |v| v.tunnel_delete(&id)) }
#[tauri::command]
pub async fn profile_save(st: S<'_>, ws: String, profile: TerminalProfile) -> CmdResult<TerminalProfile> { st.with_vault(&ws, |v| v.profile_save(profile)) }
#[tauri::command]
pub async fn profile_delete(st: S<'_>, ws: String, id: String) -> CmdResult<()> { st.with_vault(&ws, |v| v.profile_delete(&id)) }

#[tauri::command]
pub async fn hostkeys_list(st: S<'_>, ws: Option<String>) -> CmdResult<Vec<HostKey>> {
    match ws { Some(w) => st.with_vault(&w, |v| v.hostkeys()), None => Ok(st.device_hostkeys()) }
}

#[tauri::command]
pub async fn hostkey_delete(st: S<'_>, ws: Option<String>, host: String, port: u16, key_type: String) -> CmdResult<()> {
    match ws {
        Some(w) => st.with_vault(&w, |v| v.hostkey_delete(&host, port, &key_type)),
        None => {
            let mut all = st.device_hostkeys();
            all.retain(|k| !(k.host == host.to_lowercase() && k.port == port && k.key_type == key_type));
            st.save_device_hostkeys(&all)
        }
    }
}

#[tauri::command]
pub async fn audit_list(st: S<'_>, ws: String, limit: Option<usize>) -> CmdResult<Vec<AuditEntry>> {
    st.with_vault(&ws, |v| v.audit_list(limit.unwrap_or(1000)))
}

fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

#[tauri::command]
pub async fn audit_export_csv(st: S<'_>, ws: String, path: String) -> CmdResult<usize> {
    let rows = st.with_vault(&ws, |v| v.audit_list(100_000))?;
    let mut out = String::from("at,actor,action,target_id,detail\n");
    for r in &rows { out += &format!("{},{},{},{},{}\n", csv_cell(&r.at), csv_cell(&r.actor), csv_cell(&r.action), csv_cell(&r.target_id), csv_cell(&r.detail.to_string())); }
    write_atomic(Path::new(&path), out.as_bytes())?;
    Ok(rows.len())
}

// ------------------------------------------------------------------ import / export

#[tauri::command]
pub async fn import_preview(st: S<'_>, kind: String, path: Option<String>, _password: Option<String>) -> CmdResult<Value> {
    let read = |p: &Option<String>| -> CmdResult<String> {
        let p = p.as_ref().ok_or_else(|| CmdErr::new("import", "choose a file first"))?;
        let bytes = std::fs::read(p)?;
        ok_import::decode_text(&bytes).map_err(|m| CmdErr::new("import", m))
    };
    let parsed = match kind.as_str() {
        "csv" => ok_import::parse_csv(&read(&path)?).map_err(|m| CmdErr::new("import", m))?,
        "putty" => ok_import::parse_putty(ok_import::load_putty().map_err(|m| CmdErr::new("import", m))?),
        "rdg" => ok_import::parse_rdg(&read(&path)?).map_err(|m| CmdErr::new("import", m))?,
        "json" => ok_import::parse_json(&read(&path)?).map_err(|m| CmdErr::new("import", m))?,
        "openssh" => {
            let p = path.clone().filter(|s| !s.is_empty()).map(PathBuf::from).unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".ssh").join("config"));
            let text = ok_import::decode_text(&std::fs::read(&p)?).map_err(|m| CmdErr::new("import", m))?;
            ok_import::parse_openssh(&text)
        }
        other => return Err(CmdErr::new("import", format!("the '{other}' importer is not available in this build"))),
    };
    let summary = json!({
        "connections": parsed.connections.len(), "folders": parsed.folders.len(), "tunnels": parsed.tunnels.len(),
        "with_credentials": parsed.secrets.len(), "report": parsed.report,
    });
    let token = ok_crypto::random_token();
    st.imports.lock().unwrap().insert(token.clone(), PendingImport { parsed, source: kind });
    let mut out = summary;
    out["token"] = json!(token);
    Ok(out)
}

#[tauri::command]
pub async fn import_commit(st: S<'_>, ws: String, token: String, parent: Option<String>, wrap: Option<String>) -> CmdResult<Value> {
    let pending = st.imports.lock().unwrap().remove(&token).ok_or_else(|| CmdErr::new("import", "the import preview expired; run it again"))?;
    let p = pending.parsed;
    st.with_vault(&ws, |v| {
        let mut top = parent.filter(|s| !s.is_empty());
        if let Some(w) = wrap.filter(|s| !s.trim().is_empty()) {
            let f = v.folder_save(Folder { id: new_id(), parent_id: top.clone(), name: w, color: None, icon: None, defaults: Default::default(), has_secret: false, sort: 9999, version: 0 })?;
            top = Some(f.id);
        }
        let ids: std::collections::HashSet<&str> = p.folders.iter().map(|f| f.id.as_str()).collect();
        for f in &p.folders {
            let mut f = f.clone();
            if f.parent_id.as_deref().map_or(true, |x| !ids.contains(x)) { f.parent_id = top.clone(); }
            f.version = 0;
            v.folder_save(f)?;
        }
        for c in &p.connections {
            let mut c = c.clone();
            if c.folder_id.as_deref().map_or(true, |x| !ids.contains(x)) { c.folder_id = top.clone(); }
            c.version = 0;
            c.has_secret = false;
            let id = c.id.clone();
            v.conn_save(c)?;
            if let Some(s) = p.secrets.get(&id) { v.secret_set("connection", &id, Some(s.clone()))?; }
        }
        for t in &p.tunnels { let mut t = t.clone(); t.version = 0; v.tunnel_save(t)?; }
        v.audit("import", &v.id.clone(), json!({"source": pending.source, "folders": p.folders.len(), "connections": p.connections.len()}))?;
        Ok(json!({"folders": p.folders.len(), "connections": p.connections.len(), "tunnels": p.tunnels.len()}))
    })
}

#[tauri::command]
pub async fn export_json(st: S<'_>, ws: String, path: String) -> CmdResult<usize> {
    let (snap, name) = st.with_vault(&ws, |v| Ok((v.snapshot()?, v.name.clone())))?;
    let doc = json!({
        "format": "okremote-export", "version": 1, "exported_at": chrono::Utc::now().to_rfc3339(), "workspace": name,
        "secrets_included": false, "folders": snap.folders, "connections": snap.connections, "tunnels": snap.tunnels,
        "terminal_profiles": snap.terminal_profiles,
    });
    write_atomic(Path::new(&path), serde_json::to_string_pretty(&doc)?.as_bytes())?;
    Ok(snap.connections.len())
}

// ------------------------------------------------------------------ sessions

#[derive(Deserialize)]
pub struct Target { pub ws: Option<String>, pub conn_id: Option<String>, pub quick: Option<String> }

fn resolve_in(v: &Vault, snap: &Snapshot, ws: &str, id: &str) -> Result<ssh::Resolved, ok_store::StoreError> {
    let c = snap.connections.iter().find(|c| c.id == id).ok_or(ok_store::StoreError::NotFound)?;
    let secret = v.secret_effective(snap, id)?;
    Ok(ssh::Resolved { title: c.name.clone(), host: c.host.clone(), port: c.effective_port(), user: None, secret, options: snap.resolve_options(c),
        ws: Some(ws.to_string()), conn_id: Some(id.to_string()), color: snap.inherited_color(c.folder_id.as_deref()) })
}

fn resolve(st: &AppState, t: &Target) -> CmdResult<ssh::Resolved> {
    if let Some(q) = &t.quick {
        let qt = parse_quick(q).map_err(|m| CmdErr::new("invalid", m))?;
        return Ok(ssh::Resolved { title: qt.name.clone(), port: qt.port.unwrap_or(qt.protocol.default_port()), host: qt.host, user: qt.user,
            secret: Secret::default(), options: Default::default(), ws: None, conn_id: None, color: None });
    }
    let (ws, id) = (t.ws.clone().ok_or_else(|| CmdErr::new("invalid", "missing workspace"))?, t.conn_id.clone().ok_or_else(|| CmdErr::new("invalid", "missing connection"))?);
    st.with_vault(&ws, |v| { let snap = v.snapshot()?; resolve_in(v, &snap, &ws, &id) })
}

/// The shared transport this connection routes through, if its resolved options name a tunnel profile (11.2).
async fn route_for(st: &AppState, r: &ssh::Resolved) -> CmdResult<Option<std::sync::Arc<crate::tunnel::Transport>>> {
    let Some(pid) = r.options.get("tunnel_profile").and_then(Value::as_str).filter(|s| !s.is_empty() && *s != "none").map(String::from) else { return Ok(None) };
    let Some(ws) = r.ws.clone() else { return Ok(None) };
    let (profile, chain) = st.with_vault(&ws, |v| {
        let snap = v.snapshot()?;
        let p = snap.tunnels.iter().find(|t| t.id == pid).cloned().ok_or(ok_store::StoreError::NotFound)?;
        let mut chain = vec![];
        for hop in &p.chain {
            let c = snap.connections.iter().find(|c| c.id == *hop).ok_or_else(|| ok_store::StoreError::Invalid("a hop of the tunnel profile no longer exists".into()))?;
            if c.protocol != Protocol::Ssh { return Err(ok_store::StoreError::Invalid(format!("hop '{}' is not an SSH connection", c.name))); }
            chain.push((hop.clone(), resolve_in(v, &snap, &ws, hop)?));
        }
        Ok((p, chain))
    })?;
    let t = crate::tunnel::transport_for(st, &ws, chain).await?;
    let owner = format!("profile:{ws}:{}", profile.id);
    if profile.auto_start && !profile.forwards.is_empty() && !crate::tunnel::owner_running(st, &owner) {
        for f in profile.forwards.iter().filter(|f| !f.exposes()) {
            if let Err(e) = crate::tunnel::start_forward(st, &t, f, &owner, true, &profile.name, false).await { toast_err(st, &e.message); }
        }
    }
    Ok(Some(t))
}

fn toast_err(st: &AppState, m: &str) { use tauri::Emitter; let _ = st.app.emit("toast", json!({"kind": "error", "text": m})); }

#[tauri::command]
pub async fn session_open(st: S<'_>, target: Target, cols: u32, rows: u32, binary: Channel<InvokeResponseBody>) -> CmdResult<Value> {
    let r = resolve(&st, &target)?;
    if target.conn_id.is_some() {
        let proto = st.with_vault(target.ws.as_deref().unwrap_or(""), |v| Ok(v.conn_get(target.conn_id.as_deref().unwrap_or(""))?.map(|c| c.protocol)))?;
        if proto != Some(Protocol::Ssh) { return Err(CmdErr::new("ssh", "not an SSH connection")); }
    }
    let color = r.color.clone();
    let (via, lease, route) = match route_for(&st, &r).await? {
        Some(t) => (t.last().await, Some(crate::tunnel::lease(&st, &t)), Some(t.label.clone())),
        None => (None, None, None),
    };
    let o = ssh::open_session(&st, r, via, lease, cols, rows, binary).await?;
    Ok(json!({"id": o.id, "title": o.title, "route": route, "terminal_profile": o.terminal_profile, "osc52": o.osc52,
        "fixed_cols": o.fixed_cols, "fixed_rows": o.fixed_rows, "log_file": o.log_file, "color": color}))
}

#[tauri::command]
pub async fn rdp_open(st: S<'_>, target: Target, width: u16, height: u16, binary: Channel<InvokeResponseBody>) -> CmdResult<Value> {
    let r = resolve(&st, &target)?;
    if let (Some(ws), Some(cid)) = (target.ws.as_deref(), target.conn_id.as_deref()) {
        let proto = st.with_vault(ws, |v| Ok(v.conn_get(cid)?.map(|c| c.protocol)))?;
        if proto != Some(Protocol::Rdp) { return Err(CmdErr::new("rdp", "not an RDP connection")); }
    }
    if r.options.get("rdp_client_path").and_then(Value::as_str).is_some_and(|s| !s.is_empty()) { return Err(CmdErr::new("external", "this connection uses an external RDP client")); }
    let mut user = r.user.clone().or_else(|| r.secret.username.clone()).unwrap_or_default();
    let mut password = r.secret.password.clone().unwrap_or_default();
    let mut domain = r.secret.domain.clone();
    if user.is_empty() || password.is_empty() {
        let Some(a) = st.ask("credentials", json!({"host": r.host, "name": r.title, "protocol": "rdp", "need_user": user.is_empty(), "need_password": password.is_empty(),
            "user": user, "default_user": "", "domain": domain.clone().unwrap_or_default()})).await
            else { return Err(CmdErr::new("cancelled", format!("{} was not opened: no credentials were entered", r.title))) };
        let get = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        if user.is_empty() { user = get("user"); }
        if password.is_empty() { password = get("password"); }
        if domain.is_none() { domain = Some(get("domain")).filter(|d| !d.is_empty()); }
    }
    let id = ok_crypto::random_token();
    let owner = format!("rdp:{id}");
    let route = route_for(&st, &r).await?;
    let mut connect = (r.host.clone(), r.port);
    let mut label = None;
    if let Some(t) = &route {
        let spec = Forward::Local { bind_addr: "127.0.0.1".into(), bind_port: 0, remote_host: r.host.clone(), remote_port: r.port };
        let (_, p) = crate::tunnel::start_forward(&st, t, &spec, &owner, true, &format!("{} (RDP {}:{})", r.title, r.host, r.port), false).await?;
        connect = ("127.0.0.1".into(), p);
        label = Some(t.label.clone());
    }
    let params = crate::rdp::RdpParams {
        connect, host: r.host.clone(), port: r.port, ws: r.ws.clone(), user, password, domain,
        nla: r.options.get("rdp_nla").and_then(Value::as_bool).unwrap_or(true),
        width: width.max(640), height: height.max(480),
    };
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let (first, thread) = crate::rdp::spawn(st.inner().clone(), params, binary, rx);
    let res = match tokio::time::timeout(std::time::Duration::from_secs(45), first).await {
        Ok(Ok(r)) => r,
        Ok(Err(_)) => Err("the connection ended unexpectedly".to_string()),
        Err(_) => { let _ = tx.send(crate::rdp::RdpIn::Close); Err("timed out waiting for the server".to_string()) }
    };
    match res {
        Ok((w, h)) => {
            st.rdp.lock().unwrap().insert(id.clone(), tx);
            let (st2, id2) = (st.inner().clone(), id.clone());
            tokio::task::spawn_blocking(move || {
                let _ = thread.join();
                st2.rdp.lock().unwrap().remove(&id2);
                tauri::async_runtime::block_on(crate::tunnel::stop_owner(&st2, &format!("rdp:{id2}")));
            });
            Ok(json!({"id": id, "title": r.title, "route": label, "width": w, "height": h, "color": r.color}))
        }
        Err(m) => { crate::tunnel::stop_owner(&st, &owner).await; Err(CmdErr::new("rdp", m)) }
    }
}

#[tauri::command]
pub async fn rdp_input(st: S<'_>, id: String, event: Value) -> CmdResult<()> {
    if let (Some(tx), Ok(ev)) = (st.rdp.lock().unwrap().get(&id), serde_json::from_value::<crate::rdp::RdpIn>(event)) { let _ = tx.send(ev); }
    Ok(())
}

#[tauri::command]
pub async fn rdp_close(st: S<'_>, id: String) -> CmdResult<()> {
    if let Some(tx) = st.rdp.lock().unwrap().get(&id) { let _ = tx.send(crate::rdp::RdpIn::Close); }
    Ok(())
}

#[tauri::command]
pub async fn session_write(st: S<'_>, id: String, data: Vec<u8>) -> CmdResult<()> {
    if let Some(s) = st.sessions.lock().unwrap().get(&id) { let _ = s.tx.send(SessionInput::Data(data)); }
    Ok(())
}
#[tauri::command]
pub async fn session_resize(st: S<'_>, id: String, cols: u32, rows: u32) -> CmdResult<()> {
    if let Some(s) = st.sessions.lock().unwrap().get(&id) { let _ = s.tx.send(SessionInput::Resize(cols, rows)); }
    Ok(())
}
#[tauri::command]
pub async fn session_close(st: S<'_>, id: String) -> CmdResult<()> {
    if let Some(s) = st.sessions.lock().unwrap().get(&id) { let _ = s.tx.send(SessionInput::Close); }
    Ok(())
}
#[tauri::command]
pub async fn sessions_list(st: S<'_>) -> CmdResult<Value> {
    Ok(Value::Array(st.sessions.lock().unwrap().iter().map(|(id, s)| json!({"id": id, "title": s.title, "ws": s.ws, "conn_id": s.conn_id})).collect()))
}

// ------------------------------------------------------------------ tunnels

#[tauri::command]
pub async fn tunnels_status(st: S<'_>) -> CmdResult<Value> { Ok(crate::tunnel::status(&st)) }

#[tauri::command]
pub async fn tunnel_start(st: S<'_>, ws: String, id: String, allow_exposed: bool) -> CmdResult<Vec<String>> {
    let (profile, chain) = st.with_vault(&ws, |v| {
        let snap = v.snapshot()?;
        let p = snap.tunnels.iter().find(|t| t.id == id).cloned().ok_or(ok_store::StoreError::NotFound)?;
        let mut chain = vec![];
        for hop in &p.chain {
            let c = snap.connections.iter().find(|c| c.id == *hop).ok_or_else(|| ok_store::StoreError::Invalid("a hop of the tunnel profile no longer exists".into()))?;
            if c.protocol != Protocol::Ssh { return Err(ok_store::StoreError::Invalid(format!("hop '{}' is not an SSH connection", c.name))); }
            chain.push((hop.clone(), resolve_in(v, &snap, &ws, hop)?));
        }
        Ok((p, chain))
    })?;
    let t = crate::tunnel::transport_for(&st, &ws, chain).await?;
    let owner = format!("profile:{ws}:{id}");
    let mut listening = vec![];
    for f in &profile.forwards {
        let (_, port) = crate::tunnel::start_forward(&st, &t, f, &owner, false, &profile.name, allow_exposed).await?;
        listening.push(format!("{}:{port}", match f { Forward::Local { bind_addr, .. } | Forward::Dynamic { bind_addr, .. } => if bind_addr.is_empty() { "127.0.0.1" } else { bind_addr }, Forward::Remote { remote_bind_addr, .. } => remote_bind_addr }));
    }
    Ok(listening)
}

#[tauri::command]
pub async fn tunnel_stop(st: S<'_>, ws: String, id: String) -> CmdResult<()> { crate::tunnel::stop_owner(&st, &format!("profile:{ws}:{id}")).await; Ok(()) }

#[tauri::command]
pub async fn forward_stop(st: S<'_>, id: String) -> CmdResult<()> { crate::tunnel::stop_forward(&st, &id).await; Ok(()) }

// ------------------------------------------------------------------ non-embedded launches

/// Opens a URL or file with the platform's default handler (works the same on macOS, Windows and Linux).
fn open_with_system(app: &tauri::AppHandle, arg: &str) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    if arg.contains("://") { app.opener().open_url(arg, None::<&str>).map_err(|e| e.to_string()) }
    else { app.opener().open_path(arg, None::<&str>).map_err(|e| e.to_string()) }
}

/// Finds an executable by name on PATH, adding `.exe` on Windows.
fn find_program(names: &[&str]) -> Option<std::path::PathBuf> {
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        for n in names {
            let cands = if cfg!(windows) { vec![dir.join(format!("{n}.exe")), dir.join(n)] } else { vec![dir.join(n)] };
            if let Some(c) = cands.into_iter().find(|c| c.is_file()) { return Some(c); }
        }
    }
    None
}

fn find_vnc_viewer() -> Option<std::path::PathBuf> {
    if let Some(p) = find_program(&["vncviewer", "tigervnc", "xtigervncviewer", "vncviewer64"]) { return Some(p); }
    let fixed: &[&str] = if cfg!(windows) {
        &[r"C:\Program Files\TigerVNC\vncviewer.exe", r"C:\Program Files (x86)\TigerVNC\vncviewer.exe", r"C:\Program Files\RealVNC\VNC Viewer\vncviewer.exe", r"C:\Program Files\uvnc bvba\UltraVNC\vncviewer.exe"]
    } else if cfg!(target_os = "macos") { &["/Applications/TigerVNC Viewer.app/Contents/MacOS/TigerVNC Viewer"] } else { &["/usr/bin/vncviewer", "/usr/local/bin/vncviewer"] };
    fixed.iter().map(std::path::PathBuf::from).find(|p| p.is_file())
}

fn freerdp_args(host: &str, port: u16, user: &str, domain: Option<&str>, pw: bool, v3: bool, opts: &Options, name: &str) -> Vec<String> {
    let flag = |k: &str, d: bool| opts.get(k).and_then(Value::as_bool).unwrap_or(d);
    let mut a = vec![format!("/v:{host}:{port}")];
    if !user.is_empty() { a.push(format!("/u:{user}")); }
    if let Some(d) = domain.filter(|d| !d.is_empty()) { a.push(format!("/d:{d}")); }
    if pw && !user.is_empty() { a.push("/from-stdin:force".into()); }
    a.push(if v3 { "/cert:tofu".into() } else { "/cert-tofu".into() });
    let size = |o: &Options| Some((o.get("rdp_width")?.as_u64()?, o.get("rdp_height")?.as_u64()?));
    match opts.get("rdp_scaling").and_then(Value::as_str).unwrap_or("dynamic") {
        "smart" => a.push("/smart-sizing".into()),
        "fixed" | "scroll" => if let Some((w, h)) = size(opts) { a.push(format!("/size:{w}x{h}")) },
        _ => a.push("/dynamic-resolution".into()),
    }
    if flag("rdp_fullscreen", false) { a.push("/f".into()); }
    if let Some(n) = opts.get("rdp_scale_factor").and_then(Value::as_u64).filter(|n| *n != 100) { a.push(format!("/scale-desktop:{n}")); }
    a.push(format!("/bpp:{}", opts.get("rdp_color_depth").and_then(Value::as_u64).unwrap_or(32)));
    a.push(if flag("rdp_clipboard", true) { "+clipboard".into() } else { "-clipboard".into() });
    if flag("rdp_drives", false) { a.push("/drive:home,~".into()); }
    if flag("rdp_printers", false) { a.push("/printer".into()); }
    if !flag("rdp_nla", true) { a.push("-sec-nla".into()); }
    if flag("rdp_admin", false) { a.push("/admin".into()); }
    a.push(format!("/title:{name}"));
    a
}

fn split_command(cmd: &str) -> Result<Vec<String>, String> {
    let (mut out, mut cur, mut quote, mut has) = (vec![], String::new(), None::<char>, false);
    for ch in cmd.chars() {
        match (quote, ch) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => { quote = Some(ch); has = true; }
            (None, c) if c.is_whitespace() => { if has || !cur.is_empty() { out.push(std::mem::take(&mut cur)); has = false; } }
            (None, c) => cur.push(c),
        }
    }
    if quote.is_some() { return Err("unbalanced quotes in command".into()); }
    if has || !cur.is_empty() { out.push(cur); }
    Ok(out)
}

fn substitute(arg: &str, vars: &[(&str, String)]) -> String {
    let mut out = arg.to_string();
    for (k, v) in vars {
        let pat = format!("%{k}%");
        while let Some(i) = out.to_uppercase().find(&pat) { out.replace_range(i..i + pat.len(), v); }
    }
    out
}

/// VNC, RDP, web and external tools launch outside the app (spec sections 9.5, 10).
#[tauri::command]
pub async fn launch_external(st: S<'_>, target: Target) -> CmdResult<Value> {
    let t = &target;
    let (ws, id) = (t.ws.clone().unwrap_or_default(), t.conn_id.clone().unwrap_or_default());
    let (c, opts, secret) = st.with_vault(&ws, |v| {
        let snap = v.snapshot()?;
        let c = snap.connection(&id).cloned().ok_or(ok_store::StoreError::NotFound)?;
        Ok((c.clone(), snap.resolve_options(&c), v.secret_effective(&snap, &id)?))
    })?;
    let user = secret.username.clone().unwrap_or_default();
    let secs = st.setting_u64("clipboard_clear_seconds", 30);
    let (mut host, mut port) = (c.host.clone(), c.effective_port());
    let r0 = ssh::Resolved { title: c.name.clone(), host: host.clone(), port, user: None, secret: Secret::default(), options: opts.clone(), ws: Some(ws.clone()), conn_id: Some(id.clone()), color: None };
    let route = route_for(&st, &r0).await?;
    let owner = format!("launch:{}", new_id());
    let web = matches!(c.protocol, Protocol::Http | Protocol::Https);
    if let (Some(t), false) = (&route, web) {
        let spec = Forward::Local { bind_addr: "127.0.0.1".into(), bind_port: 0, remote_host: host.clone(), remote_port: port };
        let label = format!("{} ({} {host}:{port})", c.name, c.protocol.as_str().to_uppercase());
        let (_, p) = crate::tunnel::start_forward(&st, t, &spec, &owner, true, &label, false).await?;
        host = "127.0.0.1".into();
        port = p;
    }
    let track = |mut child: std::process::Child| {
        let (st2, owner) = (st.inner().clone(), owner.clone());
        tokio::task::spawn_blocking(move || { let _ = child.wait(); tauri::async_runtime::block_on(crate::tunnel::stop_owner(&st2, &owner)); });
    };
    let launched = |client: &str, copied: bool| Ok(json!({"client": client, "route": null, "password_copied": copied,
        "message": format!("Opened with {client}{}", if copied { "; the password was copied to the clipboard" } else { "" })}));
    let fail = |m: String| CmdErr::new("launch", m);
    match c.protocol {
        Protocol::Http | Protocol::Https => {
            let url = if c.host.contains("://") { c.host.clone() } else {
                let port = c.port.filter(|p| *p != c.protocol.default_port()).map(|p| format!(":{p}")).unwrap_or_default();
                let path = opts.get("http_path").and_then(Value::as_str).filter(|s| !s.is_empty())
                    .map(|p| if p.starts_with('/') { p.to_string() } else { format!("/{p}") }).unwrap_or_default();
                format!("{}://{}{port}{path}", c.protocol.as_str(), c.host)
            };
            let system = opts.get("http_open_in").and_then(Value::as_str) == Some("system") && route.is_none();
            let copy = !user.is_empty() && secret.password.is_some();
            if copy { clipboard_with_clear(&st.app, secret.password.clone().unwrap(), secs); }
            if system {
                open_with_system(&st.app, &url).map_err(fail)?;
                return launched("system browser", copy);
            }
            let label = format!("web-{}", ok_crypto::random_token());
            let mut b = tauri::WebviewWindowBuilder::new(&st.app, &label, tauri::WebviewUrl::External(url.parse().map_err(|_| fail("invalid URL".into()))?))
                .title(format!("{} — {url}", c.name)).inner_size(1280.0, 860.0);
            if let Some(t) = &route {
                let spec = Forward::Dynamic { bind_addr: "127.0.0.1".into(), bind_port: 0 };
                let (_, p) = crate::tunnel::start_forward(&st, t, &spec, &owner, true, &format!("{} (web proxy)", c.name), false).await?;
                b = b.proxy_url(format!("socks5://127.0.0.1:{p}").parse().map_err(|_| fail("invalid proxy".into()))?);
            }
            let w = b.build().map_err(|e| fail(e.to_string()))?;
            let zoom = opts.get("http_zoom").and_then(Value::as_f64).unwrap_or(1.0);
            if zoom != 1.0 { let _ = w.set_zoom(zoom.clamp(0.25, 5.0)); }
            let (st2, owner2) = (st.inner().clone(), owner.clone());
            w.on_window_event(move |e| {
                if matches!(e, tauri::WindowEvent::Destroyed) { let (s, o) = (st2.clone(), owner2.clone()); tauri::async_runtime::spawn(async move { crate::tunnel::stop_owner(&s, &o).await; }); }
            });
            launched("okRemote window", copy)
        }
        Protocol::Vnc if cfg!(target_os = "macos") => {
            let u = if user.is_empty() { String::new() } else { format!("{user}@") };
            open_with_system(&st.app, &format!("vnc://{u}{host}:{port}")).map_err(|_| fail("Screen Sharing could not be opened".into()))?;
            let copy = secret.password.is_some();
            if copy { clipboard_with_clear(&st.app, secret.password.clone().unwrap(), secs); }
            launched("Screen Sharing", copy)
        }
        Protocol::Vnc => {
            let custom = opts.get("vnc_client_path").and_then(Value::as_str).filter(|s| !s.is_empty()).map(std::path::PathBuf::from);
            let viewer = custom.or_else(find_vnc_viewer).ok_or_else(|| fail("no VNC viewer was found; install TigerVNC or set the viewer path on the connection".into()))?;
            let mut cmd = std::process::Command::new(&viewer);
            cmd.arg(format!("{host}::{port}"));
            if opts.get("vnc_view_only").and_then(Value::as_bool).unwrap_or(false) { cmd.arg("-ViewOnly"); }
            track(cmd.spawn().map_err(|e| fail(format!("could not start the VNC viewer: {e}")))?);
            let copy = secret.password.is_some();
            if copy { clipboard_with_clear(&st.app, secret.password.clone().unwrap(), secs); }
            launched("VNC viewer", copy)
        }
        Protocol::Rdp => {
            let custom = opts.get("rdp_client_path").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
            let custom_name = custom.as_deref().map(|c| std::path::Path::new(c).file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default());
            let custom_is_freerdp = custom_name.as_deref().is_some_and(|n| n.contains("freerdp"));
            let unix_other = cfg!(all(unix, not(target_os = "macos")));
            let mac_no_native = cfg!(target_os = "macos") && !std::path::Path::new("/Applications/Windows App.app").exists() && !std::path::Path::new("/Applications/Microsoft Remote Desktop.app").exists();
            if custom_is_freerdp || (custom.is_none() && (unix_other || mac_no_native)) {
                let prog = match &custom {
                    Some(c) => std::path::PathBuf::from(c),
                    None => find_program(&["xfreerdp3", "sdl-freerdp3", "sdl3-freerdp", "sdl-freerdp", "xfreerdp", "wlfreerdp"])
                        .ok_or_else(|| fail("FreeRDP was not found; install the freerdp3-x11 or freerdp2-x11 package".into()))?,
                };
                let pname = prog.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
                let v3 = pname.contains('3') || pname.starts_with("sdl-");
                let args = freerdp_args(&host, port, &user, secret.domain.as_deref(), secret.password.is_some(), v3, &opts, &c.name);
                let log_dir = if cfg!(target_os = "macos") { dirs::home_dir().unwrap_or_default().join("Library/Logs/OkRemote") } else { dirs::home_dir().unwrap_or_default().join(".local/state/okremote") };
                std::fs::create_dir_all(&log_dir).ok();
                let log_path = log_dir.join("freerdp-last.log");
                let log = std::fs::File::create(&log_path).ok();
                let mut cmd = std::process::Command::new(&prog);
                cmd.args(&args).stdin(std::process::Stdio::piped());
                if let Some(l) = log { if let Ok(l2) = l.try_clone() { cmd.stdout(l).stderr(l2); } }
                let mut child = cmd.spawn().map_err(|e| fail(format!("could not start FreeRDP: {e}")))?;
                let piped = secret.password.is_some() && !user.is_empty();
                if let Some(mut sin) = child.stdin.take() { if let (true, Some(pw)) = (piped, &secret.password) { use std::io::Write; let _ = writeln!(sin, "{pw}"); } }
                let early = tokio::task::spawn_blocking(move || { std::thread::sleep(std::time::Duration::from_secs(5)); child.try_wait().ok().flatten().map(|s| (s, child)) }).await.ok().flatten();
                if let Some((status, _)) = early {
                    if !status.success() {
                        let tail: Vec<String> = std::fs::read_to_string(&log_path).unwrap_or_default().lines().rev().filter(|l| l.to_lowercase().contains("error") || l.contains("ERRCONNECT")).take(3).map(String::from).collect();
                        return Err(fail(format!("FreeRDP exited with an error:\n{}\nLog: {}", tail.join("\n"), log_path.display())));
                    }
                }
                let clip = secret.password.is_some() && user.is_empty();
                if clip { clipboard_with_clear(&st.app, secret.password.clone().unwrap(), secs); }
                return launched("FreeRDP", clip);
            }
            let mut body = format!("full address:s:{host}:{port}\r\n");
            if !user.is_empty() {
                let u = match &secret.domain { Some(d) if !user.contains(['\\', '@']) => format!("{d}\\{user}"), _ => user.clone() };
                body += &format!("username:s:{u}\r\n");
            }
            body += &format!("screen mode id:i:{}\r\nenablecredsspsupport:i:{}\r\nprompt for credentials:i:0\r\nauthentication level:i:2\r\n",
                if opts.get("rdp_fullscreen").and_then(Value::as_bool).unwrap_or(false) { 2 } else { 1 },
                if opts.get("rdp_nla").and_then(Value::as_bool).unwrap_or(true) { 1 } else { 0 });
            let file = std::env::temp_dir().join(format!("okremote-{}.rdp", new_id()));
            std::fs::write(&file, body)?;
            #[cfg(unix)]
            { use std::os::unix::fs::PermissionsExt; let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)); }
            if let Some(c) = &custom { std::process::Command::new(c).arg(&file).spawn().map_err(|e| fail(format!("could not start {c}: {e}")))?; }
            else if cfg!(target_os = "windows") { std::process::Command::new("mstsc.exe").arg(&file).spawn().map_err(|e| fail(e.to_string()))?; }
            else { open_with_system(&st.app, &file.display().to_string()).map_err(|_| fail("no application is installed to open .rdp files; install Windows App from the App Store".into()))?; }
            let cleanup = file.clone();
            tokio::spawn(async move { tokio::time::sleep(std::time::Duration::from_secs(30)).await; let _ = std::fs::remove_file(cleanup); });
            let copy = secret.password.is_some();
            if copy { clipboard_with_clear(&st.app, secret.password.clone().unwrap(), secs); }
            launched("RDP client", copy)
        }
        Protocol::External => {
            let cmd = opts.get("ext_command").and_then(Value::as_str).unwrap_or("").trim().to_string();
            if cmd.is_empty() { return Err(fail("this connection has no command line".into())); }
            let args = split_command(&cmd).map_err(fail)?;
            if cmd.to_uppercase().contains("%PASSWORD%") { return Err(fail("%PASSWORD% is not allowed on a command line; read $OKREMOTE_PASSWORD or stdin instead".into())); }
            let vars = [("NAME", c.name.clone()), ("HOST", host.clone()), ("PORT", port.to_string()), ("USER", user.clone()), ("DOMAIN", secret.domain.clone().unwrap_or_default())];
            let args: Vec<String> = args.iter().map(|a| substitute(a, &vars)).collect();
            let Some((prog, rest)) = args.split_first() else { return Err(fail("this connection has no command line".into())) };
            let mut p = std::process::Command::new(prog);
            p.args(rest).env("OKREMOTE_HOST", &host).env("OKREMOTE_PORT", port.to_string());
            if !user.is_empty() { p.env("OKREMOTE_USER", &user); }
            let via = opts.get("ext_password_via").and_then(Value::as_str).unwrap_or("env");
            if via == "env" { if let Some(pw) = &secret.password { p.env("OKREMOTE_PASSWORD", pw); } }
            if let Some(dir) = opts.get("ext_working_dir").and_then(Value::as_str).filter(|s| !s.is_empty()) { p.current_dir(dir); }
            if via == "stdin" { p.stdin(std::process::Stdio::piped()); } else { p.stdin(std::process::Stdio::null()); }
            let mut child = p.spawn().map_err(|e| fail(format!("could not start {prog}: {e}")))?;
            let _ = &mut child;
            if via == "stdin" {
                if let (Some(mut sin), Some(pw)) = (child.stdin.take(), &secret.password) { use std::io::Write; let _ = writeln!(sin, "{pw}"); }
            }
            track(child);
            launched(prog, false)
        }
        Protocol::Ssh => Err(fail("SSH connections open in a tab".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_quoted_commands() {
        assert_eq!(split_command(r#"putty.exe -telnet "my host" '' x"#).unwrap(), vec!["putty.exe", "-telnet", "my host", "", "x"]);
        assert!(split_command("a \"b").is_err());
    }

    #[test]
    fn substitutes_case_insensitively() {
        let v = [("HOST", "h".to_string()), ("PORT", "22".to_string())];
        assert_eq!(substitute("%host%:%Port%", &v), "h:22");
    }

    #[test]
    fn settings_merge_terminal_fieldwise() {
        let m = merge_settings(&json!({"terminal": {"fontSize": 20}, "accent": "#3b82f6"}));
        assert_eq!(m["terminal"]["fontSize"], 20);
        assert_eq!(m["terminal"]["scrollback"], 10000);
        assert_eq!(m["accent"], "#9fcdba");
    }
}
