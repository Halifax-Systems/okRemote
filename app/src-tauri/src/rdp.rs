//! Embedded RDP client engine (spec section 9): connect, sign in, decode, pump input.
use crate::state::*;
use ironrdp_connector::connection_activation::ConnectionActivationState;
use sspi::generator::NetworkRequest;
use ironrdp_connector::{ClientConnector, ConnectorResult, ServerName};
use ironrdp_core::WriteBuf;
use ironrdp_displaycontrol::client::DisplayControlClient;
use ironrdp_displaycontrol::pdu::MonitorLayoutEntry;
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_input::{Database, MouseButton, MousePosition, Operation, Scancode, WheelRotations};
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{fast_path, ActiveStageBuilder, ActiveStageOutput};
use ironrdp_tokio::{single_sequence_step_read, split_tokio_framed, FramedWrite, NetworkClient, TokioFramed};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::time::Duration;
use tauri::ipc::{Channel, InvokeResponseBody};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};

#[derive(Deserialize, Debug)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum RdpIn {
    Move { x: u16, y: u16 },
    Button { x: u16, y: u16, button: u8, down: bool },
    Wheel { x: u16, y: u16, delta: i32, horizontal: bool },
    Key { code: u8, extended: bool, down: bool },
    Unicode { ch: char, down: bool },
    Resize { width: u32, height: u32 },
    Release,
    Close,
}

pub struct RdpParams {
    /// Address to connect to (a local forward when tunneled).
    pub connect: (String, u16),
    /// Name and port the user typed; used for TLS and identity pinning.
    pub host: String,
    pub port: u16,
    pub ws: Option<String>,
    pub user: String,
    pub password: String,
    pub domain: Option<String>,
    pub nla: bool,
    pub width: u16,
    pub height: u16,
}

struct NoNetwork;
impl NetworkClient for NoNetwork {
    async fn send(&mut self, _r: &NetworkRequest) -> ConnectorResult<Vec<u8>> {
        Err(ironrdp_connector::general_err!("Kerberos is not available in this build"))
    }
}

fn msg(tag: u8, parts: &[&[u8]]) -> InvokeResponseBody {
    let mut v = vec![tag];
    for p in parts { v.extend_from_slice(p); }
    InvokeResponseBody::Raw(v)
}

fn friendly(e: &str) -> String {
    let l = e.to_lowercase();
    if l.contains("logon") || l.contains("credssp") || l.contains("access denied") || l.contains("0xc000006d") {
        "sign-in was rejected; check the user name, password and domain".into()
    } else { format!("connection failed: {e}") }
}

/// Runs the engine on its own OS thread with a single-threaded runtime. The first result tells the caller whether the connection was made.
pub fn spawn(state: AppState, p: RdpParams, out: Channel<InvokeResponseBody>, rx: mpsc::UnboundedReceiver<RdpIn>) -> (oneshot::Receiver<Result<(u16, u16), String>>, std::thread::JoinHandle<()>) {
    let (first_tx, first_rx) = oneshot::channel();
    let th = std::thread::Builder::new().name("rdp".into()).spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
        rt.block_on(async move {
            let mut first = Some(first_tx);
            let reason = match run(&state, p, &out, rx, &mut first).await {
                Ok(r) => r,
                Err(e) => {
                    let m = friendly(&e);
                    match first.take() { Some(f) => { let _ = f.send(Err(m)); return; } None => m }
                }
            };
            let _ = out.send(msg(7, &[reason.as_bytes()]));
        });
    }).expect("spawn rdp thread");
    (first_rx, th)
}

fn pin(state: &AppState, p: &RdpParams, key: &[u8]) -> Result<(), String> {
    let fp: String = Sha256::digest(key).iter().map(|b| format!("{b:02x}")).collect();
    let host = p.host.to_lowercase();
    let known: Vec<ok_model::HostKey> = match &p.ws {
        Some(ws) => state.with_vault(ws, |v| v.hostkeys()).map_err(|e| e.message)?,
        None => state.device_hostkeys(),
    };
    if let Some(k) = known.iter().find(|k| k.host == host && k.port == p.port && k.key_type == "rdp-tls") {
        if k.fingerprint != fp {
            return Err(format!("the server's identity changed (expected {}, got {fp}); connection blocked in case someone is intercepting it", k.fingerprint));
        }
        return Ok(());
    }
    match &p.ws {
        Some(ws) => { state.with_vault(ws, |v| v.hostkey_trust(&host, p.port, "rdp-tls", &fp)).map_err(|e| e.message)?; }
        None => {
            let mut all = known;
            all.push(ok_model::HostKey { host, port: p.port, key_type: "rdp-tls".into(), fingerprint: fp, first_seen: chrono::Utc::now().to_rfc3339(), trusted_by: ok_store::os_actor() });
            state.save_device_hostkeys(&all).map_err(|e| e.message)?;
        }
    }
    Ok(())
}

fn connector_config(p: &RdpParams) -> ironrdp_connector::Config {
    use ironrdp_pdu::rdp::capability_sets::client_codecs_capabilities;
    use ironrdp_pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};
    let codecs = client_codecs_capabilities(&[]).unwrap_or_default();
    ironrdp_connector::Config {
        desktop_size: ironrdp_connector::DesktopSize { width: p.width, height: p.height },
        monitor_layout: None,
        desktop_scale_factor: 0,
        desktop_physical_size: None,
        desktop_orientation: None,
        device_scale_factor: None,
        monitor_extended_layout: None,
        relative_mouse_input: false,
        enable_tls: true,
        enable_credssp: p.nla,
        enable_standard_rdp_security: false,
        credentials: ironrdp_connector::Credentials::UsernamePassword { username: p.user.clone(), password: p.password.clone() },
        domain: p.domain.clone(),
        client_build: 0,
        client_name: "okRemote".into(),
        keyboard_type: ironrdp_pdu::gcc::KeyboardType::IBM_ENHANCED,
        keyboard_subtype: 0,
        keyboard_functional_keys_count: 12,
        keyboard_layout: 0,
        connection_type: ironrdp_pdu::gcc::ConnectionType::Lan,
        ime_file_name: String::new(),
        bitmap: Some(ironrdp_connector::BitmapConfig { color_depth: 32, lossy_compression: true, codecs }),
        dig_product_id: String::new(),
        client_dir: String::new(),
        alternate_shell: String::new(),
        work_dir: String::new(),
        remote_application_mode: false,
        rail_support_level: ironrdp_pdu::rdp::capability_sets::RailSupportLevel::empty(),
        platform: ironrdp_pdu::rdp::capability_sets::MajorPlatformType::UNSPECIFIED,
        hardware_id: None,
        request_data: None,
        autologon: false,
        enable_audio_playback: false,
        enable_audio_capture: false,
        performance_flags: PerformanceFlags::default(),
        license_cache: None,
        timezone_info: TimezoneInfo::default(),
        compression_type: Some(ironrdp_pdu::rdp::client_info::CompressionType::K64),
        enable_server_pointer: true,
        pointer_software_rendering: false,
        multitransport_flags: None,
        support_dyn_vc_gfx_protocol: false,
    }
}

async fn run(state: &AppState, p: RdpParams, out: &Channel<InvokeResponseBody>, mut rx: mpsc::UnboundedReceiver<RdpIn>,
    first: &mut Option<oneshot::Sender<Result<(u16, u16), String>>>) -> Result<String, String> {
    let stream = tokio::time::timeout(Duration::from_secs(15), TcpStream::connect((p.connect.0.as_str(), p.connect.1)))
        .await.map_err(|_| "timed out connecting".to_string())?.map_err(|e| format!("could not reach {}:{}: {e}", p.host, p.port))?;
    let _ = stream.set_nodelay(true);
    let addr = stream.local_addr().map_err(|e| e.to_string())?;

    let drdynvc = ironrdp_dvc::DrdynvcClient::new().with_dynamic_channel(DisplayControlClient::new(|_| Ok(Vec::new())));
    let mut connector = ClientConnector::new(connector_config(&p), addr).with_static_channel(drdynvc);

    let mut framed = TokioFramed::new(stream);
    let upgrade = ironrdp_tokio::connect_begin(&mut framed, &mut connector).await.map_err(|e| e.to_string())?;
    let (initial, leftover) = framed.into_inner();
    let (tls, cert) = ironrdp_tls::upgrade(initial, &p.host).await.map_err(|e| format!("TLS: {e}"))?;
    let key = ironrdp_tls::extract_tls_server_public_key(&cert).ok_or("unable to read the server's TLS key")?.to_vec();
    pin(state, &p, &key)?;
    let upgraded = ironrdp_tokio::mark_as_upgraded(upgrade, &mut connector);
    let mut framed = TokioFramed::new_with_leftover(tls, leftover);
    let outcome = ironrdp_tokio::connect_finalize(upgraded, connector, &mut framed, &mut NoNetwork, ServerName::new(p.host.clone()), key, None)
            .await.map_err(|e| e.to_string())?.into_connected().map_err(|e| e.to_string())?;

    let (mut reader, mut writer) = split_tokio_framed(framed);
    let size = outcome.desktop_size;
    let mut image = DecodedImage::new(PixelFormat::RgbA32, size.width, size.height);
    let factory = outcome.activation_factory;
    let mut stage = ActiveStageBuilder {
        static_channels: outcome.static_channels,
        user_channel_id: outcome.user_channel_id,
        io_channel_id: outcome.io_channel_id,
        message_channel_id: outcome.message_channel_id,
        share_id: outcome.share_id,
        compression_type: outcome.compression_type,
        enable_server_pointer: outcome.enable_server_pointer,
        pointer_software_rendering: outcome.pointer_software_rendering,
    }.build();
    if let Some(f) = first.take() { let _ = f.send(Ok((size.width, size.height))); }
    let mut db = Database::new();

    loop {
        let outputs = tokio::select! {
            frame = reader.read_pdu() => {
                let (action, payload) = frame.map_err(|e| format!("read: {e}"))?;
                stage.process(&mut image, action, &payload, None).map_err(|e| e.to_string())?
            }
            ev = rx.recv() => {
                let Some(ev) = ev else { return Ok("closed".into()) };
                let at = |x: u16, y: u16| Operation::MouseMove(MousePosition { x, y });
                let btn = |b: u8| match b { 0 => MouseButton::Left, 1 => MouseButton::Middle, 2 => MouseButton::Right, 3 => MouseButton::X1, _ => MouseButton::X2 };
                let events = match ev {
                    RdpIn::Move { x, y } => db.apply([at(x, y)]),
                    RdpIn::Button { x, y, button, down } => db.apply([at(x, y), if down { Operation::MouseButtonPressed(btn(button)) } else { Operation::MouseButtonReleased(btn(button)) }]),
                    RdpIn::Wheel { x, y, delta, horizontal } => db.apply([at(x, y), Operation::WheelRotations(WheelRotations { is_vertical: !horizontal, rotation_units: delta.clamp(-255, 255) as i16 })]),
                    RdpIn::Key { code, extended, down } => {
                        let sc = Scancode::from_u8(extended, code);
                        db.apply([if down { Operation::KeyPressed(sc) } else { Operation::KeyReleased(sc) }])
                    }
                    RdpIn::Unicode { ch, down } => db.apply([if down { Operation::UnicodeKeyPressed(ch) } else { Operation::UnicodeKeyReleased(ch) }]),
                    RdpIn::Release => db.release_all(),
                    RdpIn::Resize { width, height } => {
                        // The server may not support display control yet; the request is then dropped.
                        let (w, h) = MonitorLayoutEntry::adjust_display_size(width.max(200), height.max(200));
                        match stage.encode_resize(w, h, None, None) {
                            Some(Ok(frame)) => { writer.write_all(&frame).await.map_err(|e| format!("write: {e}"))?; }
                            _ => {}
                        }
                        continue;
                    }
                    RdpIn::Close => {
                        let outs = stage.graceful_shutdown().map_err(|e| e.to_string())?;
                        for o in outs { if let ActiveStageOutput::ResponseFrame(f) = o { let _ = writer.write_all(&f).await; } }
                        return Ok("closed by user".into());
                    }
                };
                if events.is_empty() { continue; }
                stage.process_fastpath_input(&mut image, &events).map_err(|e| e.to_string())?
            }
        };
        let mut dirty: Option<(u16, u16, u16, u16)> = None;
        for o in outputs {
            match o {
                ActiveStageOutput::ResponseFrame(f) => writer.write_all(&f).await.map_err(|e| format!("write: {e}"))?,
                ActiveStageOutput::GraphicsUpdate(r) => {
                    dirty = Some(match dirty { None => (r.left, r.top, r.right, r.bottom), Some((l, t, rr, b)) => (l.min(r.left), t.min(r.top), rr.max(r.right), b.max(r.bottom)) });
                }
                ActiveStageOutput::PointerDefault => { let _ = out.send(msg(3, &[])); }
                ActiveStageOutput::PointerHidden => { let _ = out.send(msg(4, &[])); }
                ActiveStageOutput::PointerBitmap(ptr) => {
                    let _ = out.send(msg(5, &[&ptr.width.to_le_bytes(), &ptr.height.to_le_bytes(), &ptr.hotspot_x.to_le_bytes(), &ptr.hotspot_y.to_le_bytes(), &ptr.bitmap_data]));
                }
                ActiveStageOutput::DeactivateAll => {
                    let mut act = factory.create();
                    let mut buf = WriteBuf::new();
                    loop {
                        let w = single_sequence_step_read(&mut reader, &mut act, &mut buf).await.map_err(|e| e.to_string())?;
                        if w.size().is_some() { writer.write_all(buf.filled()).await.map_err(|e| e.to_string())?; }
                        if let ConnectionActivationState::Finalized { desktop_size, share_id, enable_server_pointer, pointer_software_rendering, .. } = act.connection_activation_state() {
                            image = DecodedImage::new(PixelFormat::RgbA32, desktop_size.width, desktop_size.height);
                            stage.set_fastpath_processor(fast_path::ProcessorBuilder {
                                io_channel_id: act.io_channel_id(), user_channel_id: act.user_channel_id(), share_id, enable_server_pointer,
                                pointer_software_rendering,
                            }.build());
                            stage.set_share_id(share_id);
                            stage.set_enable_server_pointer(enable_server_pointer);
                            let _ = out.send(msg(2, &[&desktop_size.width.to_le_bytes(), &desktop_size.height.to_le_bytes()]));
                            dirty = Some((0, 0, desktop_size.width - 1, desktop_size.height - 1));
                            break;
                        }
                    }
                }
                ActiveStageOutput::Terminate(r) => return Ok(format!("{r}").to_lowercase()),
                _ => {}
            }
        }
        if let Some((l, t, r, b)) = dirty {
            let (w, h) = (image.width(), image.height());
            let (r, b) = (r.min(w - 1), b.min(h - 1));
            if l <= r && t <= b {
                let (rw, rh) = ((r - l + 1) as usize, (b - t + 1) as usize);
                let data = image.data();
                let mut px = Vec::with_capacity(rw * rh * 4);
                for row in t as usize..=b as usize {
                    let s = (row * w as usize + l as usize) * 4;
                    px.extend_from_slice(&data[s..s + rw * 4]);
                }
                for a in px.chunks_exact_mut(4) { a[3] = 255; }
                let _ = out.send(msg(1, &[&l.to_le_bytes(), &t.to_le_bytes(), &(rw as u16).to_le_bytes(), &(rh as u16).to_le_bytes(), &px]));
            }
        }
    }
}
