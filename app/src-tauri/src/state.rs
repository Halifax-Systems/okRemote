use ok_model::*;
use ok_store::{StoreError, Vault};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::Emitter;
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, Serialize)]
pub struct CmdErr {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

impl CmdErr {
    pub fn new(code: &str, message: impl Into<String>) -> Self { Self { code: code.into(), message: message.into(), detail: None } }
}

impl From<StoreError> for CmdErr {
    fn from(e: StoreError) -> Self {
        let msg = e.to_string();
        match e {
            StoreError::Locked => Self::new("locked", msg),
            StoreError::ReadOnly => Self::new("read_only", msg),
            StoreError::WrongPassphrase => Self::new("wrong_passphrase", msg),
            StoreError::NotAVault => Self::new("not_a_vault", msg),
            StoreError::TooNew => Self::new("too_new", msg),
            StoreError::NotFound => Self::new("not_found", msg),
            StoreError::Invalid(_) => Self::new("invalid", msg),
            StoreError::Conflict(cur) => CmdErr { code: "conflict".into(), message: msg, detail: Some(cur) },
            StoreError::Storage(_) => Self::new("storage", msg),
        }
    }
}
impl From<std::io::Error> for CmdErr { fn from(e: std::io::Error) -> Self { Self::new("io", e.to_string()) } }
impl From<serde_json::Error> for CmdErr { fn from(e: serde_json::Error) -> Self { Self::new("error", e.to_string()) } }

pub type CmdResult<T> = Result<T, CmdErr>;

pub enum SessionInput { Data(Vec<u8>), Resize(u32, u32), Close }

pub struct SessionHandle {
    pub tx: mpsc::UnboundedSender<SessionInput>,
    pub title: String,
    pub ws: Option<String>,
    pub conn_id: Option<String>,
}

pub struct PendingImport { pub parsed: ok_import::Parsed, pub source: String }

pub struct Inner {
    pub config_dir: PathBuf,
    pub settings: Mutex<Value>,
    pub vaults: Mutex<HashMap<String, Arc<Mutex<Vault>>>>,
    pub sessions: Mutex<HashMap<String, SessionHandle>>,
    pub prompts: Mutex<HashMap<String, oneshot::Sender<Value>>>,
    pub imports: Mutex<HashMap<String, PendingImport>>,
    pub startup_paths: Mutex<Vec<String>>,
    pub rdp: Mutex<HashMap<String, mpsc::UnboundedSender<crate::rdp::RdpIn>>>,
    pub tunnels: crate::tunnel::Manager,
    pub app: tauri::AppHandle,
}

#[derive(Clone)]
pub struct AppState(pub Arc<Inner>);

impl std::ops::Deref for AppState { type Target = Inner; fn deref(&self) -> &Inner { &self.0 } }

pub fn default_settings() -> Value {
    json!({
        "theme": "system", "accent": "#9fcdba", "density": "comfortable", "ui_scale": 1,
        "startup_mode": "last", "pinned_workspace": null, "recent_workspaces": [], "last_open_workspaces": [],
        "auto_lock_minutes": 15, "lock_on_sleep": true, "confirm_tab_close": true, "backup_retention": 10,
        "clipboard_clear_seconds": 30,
        "terminal": {
            "fontFamily": default_font(), "fontSize": 13, "fontWeight": "normal", "lineHeight": 1.15,
            "scheme": "OkRemote Dark", "cursorStyle": "block", "cursorBlink": true, "scrollback": 10000,
            "opacity": 1, "bell": "visual", "copyOnSelect": false, "rightClickPaste": true,
            "confirmMultilinePaste": true, "highlightRules": []
        },
        "custom_schemes": [], "keybindings": {}, "sidebar_width": 290, "sidebar_collapsed": false,
        "recents": [], "last_tabs": [], "restore_tabs": "ask", "expanded": []
    })
}

fn default_font() -> &'static str {
    if cfg!(target_os = "macos") { "Menlo" } else if cfg!(target_os = "windows") { "Cascadia Mono, Consolas" } else { "DejaVu Sans Mono" }
}

/// Defaults overlaid with the stored object; the terminal block merges field by field.
pub fn merge_settings(stored: &Value) -> Value {
    let mut base = default_settings();
    if let (Some(b), Some(s)) = (base.as_object_mut(), stored.as_object()) {
        for (k, v) in s {
            if k == "terminal" {
                if let (Some(bt), Some(st)) = (b.get_mut("terminal").and_then(Value::as_object_mut), v.as_object()) {
                    for (tk, tv) in st { bt.insert(tk.clone(), tv.clone()); }
                }
            } else {
                b.insert(k.clone(), v.clone());
            }
        }
        if b.get("accent").and_then(Value::as_str).is_some_and(|a| a == "#3b82f6" || a == "#d97757") {
            b.insert("accent".into(), json!("#9fcdba"));
        }
    }
    base
}

impl AppState {
    pub fn vault(&self, ws: &str) -> CmdResult<Arc<Mutex<Vault>>> {
        self.vaults.lock().unwrap().get(ws).cloned().ok_or_else(|| CmdErr::new("not_found", "workspace is not open"))
    }

    pub fn with_vault<T>(&self, ws: &str, f: impl FnOnce(&mut Vault) -> Result<T, StoreError>) -> CmdResult<T> {
        let v = self.vault(ws)?;
        let mut g = v.lock().unwrap();
        Ok(f(&mut g)?)
    }

    pub fn settings_get(&self) -> Value { self.settings.lock().unwrap().clone() }

    pub fn setting_u64(&self, key: &str, default: u64) -> u64 {
        self.settings.lock().unwrap().get(key).and_then(Value::as_u64).unwrap_or(default)
    }

    pub fn device_hostkeys_path(&self) -> PathBuf { self.config_dir.join("known_hosts.json") }

    pub fn device_hostkeys(&self) -> Vec<HostKey> {
        std::fs::read_to_string(self.device_hostkeys_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save_device_hostkeys(&self, keys: &[HostKey]) -> CmdResult<()> {
        write_atomic(&self.device_hostkeys_path(), serde_json::to_string_pretty(keys)?.as_bytes())?;
        Ok(())
    }

    /// Ask the user a question and wait (up to 600 s) for the answer. `None` means cancelled.
    pub async fn ask(&self, kind: &str, mut payload: Value) -> Option<Value> {
        let id = ok_crypto::random_token();
        let (tx, rx) = oneshot::channel();
        self.prompts.lock().unwrap().insert(id.clone(), tx);
        payload["id"] = json!(id);
        payload["kind"] = json!(kind);
        let _ = self.app.emit("prompt", payload);
        let r = tokio::time::timeout(std::time::Duration::from_secs(600), rx).await;
        self.prompts.lock().unwrap().remove(&id);
        match r { Ok(Ok(v)) if !v.is_null() => Some(v), _ => None }
    }
}

pub fn write_atomic(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(p) = path.parent() { std::fs::create_dir_all(p)?; }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(tmp, path)
}
