import { useEffect, useMemo, useState } from 'react';
import { useStore } from '../store';
import { call, errText, isApiError } from '../api';
import {
  applyTerminalSettings, bindingFor, startTunnelProfile, refreshTunnels, chooseVaultFile, closeDialog, comboOf, createWorkspace, DEFAULT_KEYS, folderPath, loadSnapshot, openDialog,
  openVaultFromPath, pickSavePath, respondPrompt, toast, toastError, unlockWorkspace, updateSettings, snapOf,
} from '../actions';
import { Field, Icon, Modal, hint } from '../ui';
import { BUILTIN_SCHEMES, findScheme } from '../schemes';
import { open as pickOpen } from '@tauri-apps/plugin-dialog';
import type { AuditEntry, HostKey, Settings, TermProfile, TermSettings, TunnelProfile } from '../types';
import { ConflictDialog } from './Editor';

export function Dialogs() {
  const d = useStore((s) => s.dialog);
  if (!d) return null;
  const p = d.props ?? {};
  switch (d.name) {
    case 'unlock': return <UnlockDialog ws={p.ws} />;
    case 'vaults': return <VaultsDialog mode={p.mode} />;
    case 'confirm': return <ConfirmDialog {...p} />;
    case 'input': return <InputDialog {...p} />;
    case 'bulk': return <BulkDialog {...p} />;
    case 'conflict': return <ConflictDialog {...p} />;
    case 'import': return <ImportDialog ws={p.ws} />;
    case 'settings': return <SettingsDialog />;
    case 'tunnels': return <TunnelsDialog ws={p.ws} />;
    case 'profiles': return <ProfilesDialog ws={p.ws} />;
    case 'hostkeys': return <HostKeysDialog />;
    case 'audit': return <AuditDialog ws={p.ws} />;
    case 'about': return <AboutDialog />;
  }
  return null;
}

function wsList() {
  const s = useStore((x) => x.workspaces);
  const o = useStore((x) => x.wsOrder);
  return o.map((id) => s[id]).filter(Boolean);
}

// ------------------------------------------------------------------ unlock / vaults

function UnlockDialog({ ws }: { ws: string }) {
  const info = useStore((s) => s.workspaces[ws]?.info);
  const [pass, setPass] = useState('');
  const [remember, setRemember] = useState(false);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState('');
  if (!info) return null;
  const go = async () => {
    if (!pass || busy) return;
    setBusy(true); setErr('');
    try { await unlockWorkspace(ws, pass, remember); closeDialog(); }
    catch (e) { setErr(isApiError(e) && e.code === 'wrong_passphrase' ? 'That passphrase is not correct.' : errText(e)); setBusy(false); }
  };
  return (
    <Modal title={`Unlock ${info.name}`} onClose={closeDialog}
      footer={<><button onClick={closeDialog}>Cancel</button><button className="primary" disabled={!pass || busy} onClick={() => void go()}>{busy ? 'Unlocking…' : 'Unlock'}</button></>}>
      <div className="small dim mono">{info.path}</div>
      <Field label="Passphrase"><input type="password" autoFocus value={pass} onChange={(e) => setPass(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && void go()} /></Field>
      <label className="check"><input type="checkbox" checked={remember} onChange={(e) => setRemember(e.target.checked)} /> Remember on this device (OS keychain)</label>
      {err && <div className="danger-text">{err}</div>}
    </Modal>
  );
}

function VaultsDialog({ mode }: { mode?: 'open' | 'create' }) {
  const [tab, setTab] = useState<'open' | 'create'>(mode ?? 'open');
  const recents = useStore((s) => s.settings.recent_workspaces);
  const workspaces = useStore((s) => s.workspaces);
  const openPaths = new Set(Object.values(workspaces).map((w) => w.info.path));
  const [name, setName] = useState('Personal');
  const [path, setPath] = useState('');
  const [pass, setPass] = useState('');
  const [pass2, setPass2] = useState('');
  const [remember, setRemember] = useState(false);
  const [busy, setBusy] = useState(false);
  useEffect(() => { void call<string>('default_vault_path').then(setPath); }, []);
  const create = async () => {
    if (pass.length < 8) return toast('Use a passphrase of at least 8 characters', 'error');
    if (pass !== pass2) return toast('The passphrases do not match', 'error');
    setBusy(true);
    try { await createWorkspace(path, name || 'Personal', pass, remember); closeDialog(); } catch (e) { toastError(e); setBusy(false); }
  };
  return (
    <Modal title="Vault files" width="wide" onClose={closeDialog}
      footer={tab === 'create' ? <><button onClick={closeDialog}>Cancel</button><button className="primary" disabled={busy || !path || !pass} onClick={() => void create()}>{busy ? 'Creating…' : 'Create vault'}</button></> : <button onClick={closeDialog}>Close</button>}>
      <div className="inline">
        <button className={tab === 'open' ? 'primary' : ''} onClick={() => setTab('open')}>Open</button>
        <button className={tab === 'create' ? 'primary' : ''} onClick={() => setTab('create')}>Create new</button>
      </div>
      {tab === 'open' ? (<>
        <button className="primary" style={{ alignSelf: 'flex-start' }} onClick={() => void chooseVaultFile()}>Choose a vault file…</button>
        {recents.length > 0 && <b>Recent</b>}
        {recents.map((p) => (
          <button key={p} disabled={openPaths.has(p)} style={{ justifyContent: 'flex-start' }} onClick={() => void openVaultFromPath(p)}>
            <Icon name="vault" /> <span className="mono small">{p}</span> {openPaths.has(p) && <span className="badge">open</span>}
          </button>
        ))}
      </>) : (<>
        <Field label="Workspace name"><input type="text" value={name} onChange={(e) => setName(e.target.value)} /></Field>
        <Field label="File">
          <div className="inline"><input type="text" className="mono" value={path} onChange={(e) => setPath(e.target.value)} />
            <button onClick={async () => { const p = await pickSavePath(path, 'okRemote vault', 'okvault'); if (p) setPath(p.endsWith('.okvault') ? p : `${p}.okvault`); }}>Browse…</button></div>
        </Field>
        <Field label="Passphrase" hint="There is no recovery if you forget it. Use a long phrase."><input type="password" value={pass} onChange={(e) => setPass(e.target.value)} /></Field>
        <Field label="Repeat passphrase"><input type="password" value={pass2} onChange={(e) => setPass2(e.target.value)} /></Field>
        <label className="check"><input type="checkbox" checked={remember} onChange={(e) => setRemember(e.target.checked)} /> Remember on this device (OS keychain)</label>
      </>)}
    </Modal>
  );
}

// ------------------------------------------------------------------ confirm / input / bulk

function ConfirmDialog({ title, body, ok, danger, checkbox, resolve }: any) {
  const [checked, setChecked] = useState(false);
  const done = (v: boolean) => { closeDialog(); resolve({ ok: v, checked }); };
  return (
    <Modal title={title} onClose={() => done(false)}
      footer={<><button onClick={() => done(false)}>Cancel</button><button className={danger ? 'danger' : 'primary'} autoFocus onClick={() => done(true)}>{ok ?? 'OK'}</button></>}>
      {body && <pre className="pre">{body}</pre>}
      {checkbox && <label className="check"><input type="checkbox" checked={checked} onChange={(e) => setChecked(e.target.checked)} /> {checkbox}</label>}
    </Modal>
  );
}

function InputDialog({ title, label, initial, resolve }: any) {
  const [v, setV] = useState(initial ?? '');
  const done = (val: string | null) => { closeDialog(); resolve(val); };
  return (
    <Modal title={title} onClose={() => done(null)} footer={<><button onClick={() => done(null)}>Cancel</button><button className="primary" onClick={() => done(v)}>OK</button></>}>
      <Field label={label}><input type="text" autoFocus value={v} onFocus={(e) => e.target.select()} onChange={(e) => setV(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && done(v)} /></Field>
    </Modal>
  );
}

function BulkDialog({ ws, ids }: { ws: string; ids: string[] }) {
  const snap = useStore((s) => s.workspaces[ws]?.snap);
  const [what, setWhat] = useState('tunnel_profile');
  const [val, setVal] = useState('');
  const [tag, setTag] = useState('');
  const apply = async () => {
    try {
      let n = 0;
      if (what === 'tunnel_profile' || what === 'terminal_profile') n = await call('conns_set_option', { ws, ids, key: what, value: val === '' ? null : val });
      else if (what === 'creds') { for (const id of ids) await call('secret_set', { ws, owner: 'connection', id, secret: null }); n = ids.length; }
      else {
        for (const id of ids) {
          const c = snap?.connections.find((x) => x.id === id);
          if (!c || !tag.trim()) continue;
          const tags = what === 'tag_add' ? [...new Set([...c.tags, tag.trim()])] : c.tags.filter((t) => t !== tag.trim());
          await call('conn_save', { ws, conn: { ...c, tags } }); n++;
        }
      }
      await loadSnapshot(ws);
      toast(`Updated ${n} connections`, 'success');
      closeDialog();
    } catch (e) { toastError(e); }
  };
  return (
    <Modal title={`Edit ${ids.length} connections`} onClose={closeDialog} footer={<><button onClick={closeDialog}>Cancel</button><button className="primary" onClick={() => void apply()}>Apply</button></>}>
      <Field label="Change">
        <select value={what} onChange={(e) => { setWhat(e.target.value); setVal(''); }}>
          <option value="tunnel_profile">Tunnel profile</option><option value="terminal_profile">Terminal profile</option>
          <option value="creds">Credentials: clear, and inherit from the folder</option><option value="tag_add">Add a tag</option><option value="tag_remove">Remove a tag</option>
        </select>
      </Field>
      {what === 'tunnel_profile' && <select value={val} onChange={(e) => setVal(e.target.value)}><option value="">Inherit from folder</option><option value="none">None</option>{snap?.tunnels.map((t) => <option key={t.id} value={t.id}>{t.name}</option>)}</select>}
      {what === 'terminal_profile' && <select value={val} onChange={(e) => setVal(e.target.value)}><option value="">Inherit</option>{snap?.terminal_profiles.map((t) => <option key={t.id} value={t.id}>{t.name}</option>)}</select>}
      {what === 'creds' && <div className="warn-text">The stored credentials on these connections are deleted.</div>}
      {(what === 'tag_add' || what === 'tag_remove') && <Field label="Tag"><input type="text" value={tag} onChange={(e) => setTag(e.target.value)} /></Field>}
    </Modal>
  );
}

// ------------------------------------------------------------------ import

const SOURCES: [string, string, string][] = [
  ['openssh', 'OpenSSH config', 'Hosts, users, keys, ProxyJump chains and forwards. Leave the file empty to use ~/.ssh/config'],
  ['putty', 'PuTTY saved sessions', 'Read from this computer (Windows registry or ~/.putty/sessions)'],
  ['rdg', 'Remote Desktop Connection Manager (.rdg)', 'Groups and servers. Stored passwords are DPAPI-protected and cannot be imported'],
  ['csv', 'CSV', 'Columns: name, host, port, protocol, username, password, domain, folder, tags, notes'],
  ['json', 'okRemote JSON export', 'A plain export from okRemote (without secrets)'],
];

function ImportDialog({ ws: initialWs }: { ws?: string }) {
  const list = wsList().filter((w) => w.snap && !w.info.read_only);
  const [kind, setKind] = useState('openssh');
  const [file, setFile] = useState('');
  const [prev, setPrev] = useState<any>(null);
  const [ws, setWs] = useState(initialWs && list.some((w) => w.info.id === initialWs) ? initialWs : list[0]?.info.id ?? '');
  const [parent, setParent] = useState('');
  const [wrap, setWrap] = useState('');
  const [busy, setBusy] = useState(false);
  const src = SOURCES.find((s) => s[0] === kind)!;
  const snap = list.find((w) => w.info.id === ws)?.snap;
  useEffect(() => setWrap(`Imported from ${src[1]}`), [kind]); // eslint-disable-line react-hooks/exhaustive-deps
  if (!list.length) return <Modal title="Import connections" width="wide" onClose={closeDialog} footer={<button onClick={closeDialog}>Cancel</button>}><div className="danger-text">Open and unlock a workspace first; imports go into a workspace.</div></Modal>;
  const needsFile = kind !== 'openssh' && kind !== 'putty';
  const preview = async () => {
    setBusy(true);
    try { setPrev(await call('import_preview', { kind, path: file || null, password: null })); } catch (e) { toast(errText(e), 'error', 8000); } finally { setBusy(false); }
  };
  const commit = async () => {
    setBusy(true);
    try {
      const r = await call<any>('import_commit', { ws, token: prev.token, parent: parent || null, wrap: wrap || null });
      await loadSnapshot(ws);
      toast(`Imported ${r.connections} connections, ${r.folders} folders${r.tunnels ? `, ${r.tunnels} tunnel profiles` : ''}`, 'success', 7000);
      closeDialog();
    } catch (e) { toast(errText(e), 'error', 8000); setBusy(false); }
  };
  const warnings = (prev?.report ?? []).filter((r: any) => r.level === 'warning');
  const notes = (prev?.report ?? []).filter((r: any) => r.level === 'info');
  return (
    <Modal title="Import connections" width="wide" onClose={closeDialog}
      footer={<><button onClick={closeDialog}>Cancel</button>
        {prev ? <button className="primary" disabled={busy} onClick={() => void commit()}>Import {prev.connections} connections</button>
          : <button className="primary" disabled={busy || (needsFile && !file)} onClick={() => void preview()}>Preview</button>}</>}>
      {!prev ? (<>
        <Field label="Source" hint={src[2]}><select value={kind} onChange={(e) => setKind(e.target.value)}>{SOURCES.map((s) => <option key={s[0]} value={s[0]}>{s[1]}</option>)}</select></Field>
        {kind !== 'putty' && <Field label="File">
          <div className="inline"><input type="text" className="mono" value={file} placeholder={kind === 'openssh' ? '~/.ssh/config' : ''} onChange={(e) => setFile(e.target.value)} />
            <button onClick={async () => { const p = await pickOpen({ multiple: false }); if (typeof p === 'string') setFile(p); }}>Browse…</button></div>
        </Field>}
      </>) : (<>
        <div className="inline" style={{ gap: 24 }}>
          {[['connections', 'connections'], ['folders', 'folders'], ['tunnels', 'tunnel profiles'], ['with_credentials', 'with credentials']].map(([k, l]) => <div key={k}><b style={{ fontSize: '1.4rem' }}>{prev[k]}</b><div className="dim small">{l}</div></div>)}
        </div>
        {warnings.length > 0 && <div><b>{warnings.length} items need attention</b><ul>{warnings.map((w: any, i: number) => <li key={i} className="small">{w.item}: {w.message}</li>)}</ul></div>}
        {notes.length > 0 && <details><summary>{notes.length} notes</summary><ul>{notes.map((w: any, i: number) => <li key={i} className="small">{w.item}: {w.message}</li>)}</ul></details>}
        <Field label="Into workspace"><select value={ws} onChange={(e) => { setWs(e.target.value); setParent(''); }}>{list.map((w) => <option key={w.info.id} value={w.info.id}>{w.info.name}</option>)}</select></Field>
        <Field label="Under folder"><select value={parent} onChange={(e) => setParent(e.target.value)}><option value="">Top level</option>{snap?.folders.map((f) => <option key={f.id} value={f.id}>{folderPath(snap, f.id)}</option>)}</select></Field>
        <Field label="Wrap in a new folder" hint="Empty means import directly."><input type="text" value={wrap} onChange={(e) => setWrap(e.target.value)} /></Field>
      </>)}
    </Modal>
  );
}

// ------------------------------------------------------------------ settings

function TerminalEditor({ value, onChange }: { value: TermSettings; onChange: (v: TermSettings) => void }) {
  const custom = useStore((s) => s.settings.custom_schemes);
  const set = (p: Partial<TermSettings>) => onChange({ ...value, ...p });
  const scheme = findScheme(value.scheme, custom);
  const num = (k: keyof TermSettings, label: string, min: number, max: number, step = 1) => (
    <Field label={label}><input type="number" min={min} max={max} step={step} value={value[k] as number} onChange={(e) => set({ [k]: Math.min(max, Math.max(min, Number(e.target.value) || min)) } as any)} /></Field>
  );
  return (
    <div className="two" style={{ gridTemplateColumns: '1fr 1fr' }}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
        <Field label="Font family"><input type="text" value={value.fontFamily} onChange={(e) => set({ fontFamily: e.target.value })} /></Field>
        {num('fontSize', 'Font size', 6, 48)}
        <Field label="Font weight"><select value={value.fontWeight} onChange={(e) => set({ fontWeight: e.target.value })}>{['light', 'normal', 'medium', 'semibold', 'bold'].map((w) => <option key={w}>{w}</option>)}</select></Field>
        {num('lineHeight', 'Line height', 0.8, 2, 0.05)}
        <Field label="Color scheme">
          <select value={value.scheme} onChange={(e) => set({ scheme: e.target.value })}>
            <optgroup label="Dark">{BUILTIN_SCHEMES.filter((s) => s.dark).map((s) => <option key={s.name}>{s.name}</option>)}</optgroup>
            <optgroup label="Light">{BUILTIN_SCHEMES.filter((s) => !s.dark).map((s) => <option key={s.name}>{s.name}</option>)}</optgroup>
            {custom.length > 0 && <optgroup label="Imported">{custom.map((s) => <option key={s.name}>{s.name}</option>)}</optgroup>}
          </select>
        </Field>
        <Field label="Cursor"><select value={value.cursorStyle} onChange={(e) => set({ cursorStyle: e.target.value as any })}><option>block</option><option>underline</option><option>bar</option></select></Field>
        <label className="check"><input type="checkbox" checked={value.cursorBlink} onChange={(e) => set({ cursorBlink: e.target.checked })} /> Blinking cursor</label>
        {num('scrollback', 'Scrollback (lines)', 0, 200000, 1000)}
        {num('opacity', 'Opacity', 0.5, 1, 0.05)}
        <Field label="Bell"><select value={value.bell} onChange={(e) => set({ bell: e.target.value as any })}><option>off</option><option>visual</option><option>audible</option></select></Field>
        <label className="check"><input type="checkbox" checked={value.copyOnSelect} onChange={(e) => set({ copyOnSelect: e.target.checked })} /> Copy on select</label>
        <label className="check"><input type="checkbox" checked={value.rightClickPaste} onChange={(e) => set({ rightClickPaste: e.target.checked })} /> Right-click copies or pastes</label>
        <label className="check"><input type="checkbox" checked={value.confirmMultilinePaste} onChange={(e) => set({ confirmMultilinePaste: e.target.checked })} /> Confirm multi-line paste</label>
      </div>
      <div>
        <div className="lbl small dim2" style={{ marginBottom: 4 }}>Preview</div>
        <pre className="mono" style={{ background: scheme.background, color: scheme.foreground, padding: 10, borderRadius: 6, margin: 0, fontSize: value.fontSize, lineHeight: value.lineHeight, fontFamily: `${value.fontFamily}, monospace`, overflow: 'hidden' }}>
{`user@host:~$ ls -la
drwxr-xr-x  4 user user 4096 `}<span style={{ color: scheme.ansi[1] }}>error</span> <span style={{ color: scheme.ansi[3] }}>warn</span> <span style={{ color: scheme.ansi[6] }}>info</span>{`
user@host:~$ `}<span style={{ background: scheme.foreground, color: scheme.background }}> </span>{'\n'}
          {scheme.ansi.map((c, i) => <span key={i} style={{ background: c, display: 'inline-block', width: 14, height: 14, marginRight: 2 }} />)}
        </pre>
      </div>
    </div>
  );
}

function SettingsDialog() {
  const s = useStore((x) => x.settings);
  const workspaces = wsList();
  const [page, setPage] = useState('General');
  const [capturing, setCapturing] = useState<string | null>(null);
  const [pw, setPw] = useState<{ ws: string; old: string; next: string } | null>(null);
  const [remembered, setRemembered] = useState<Record<string, boolean>>({});
  useEffect(() => { workspaces.forEach((w) => void call<boolean>('ws_remembered', { id: w.info.id }).then((r) => setRemembered((m) => ({ ...m, [w.info.id]: r })))); }, []); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (!capturing) return;
    const k = (e: KeyboardEvent) => {
      e.preventDefault(); e.stopPropagation();
      if (e.key === 'Escape') return setCapturing(null);
      const c = comboOf(e);
      if (!c) return;
      updateSettings({ keybindings: { ...s.keybindings, [capturing]: c } });
      setCapturing(null);
    };
    window.addEventListener('keydown', k, true);
    return () => window.removeEventListener('keydown', k, true);
  }, [capturing, s.keybindings]);
  const known = [...new Set([...workspaces.map((w) => w.info.path), ...s.recent_workspaces])];
  const set = (p: Partial<Settings>) => updateSettings(p);
  const pages = ['General', 'Appearance', 'Security', 'Terminal', 'Keyboard'];
  return (
    <Modal title="Settings" width="xwide" onClose={closeDialog} footer={<button className="primary" onClick={closeDialog}>Done</button>}>
      <div className="settings">
        <nav role="tablist">{pages.map((p) => <button key={p} role="tab" aria-selected={page === p} onClick={() => setPage(p)}>{p}</button>)}</nav>
        <div className="page">
          {page === 'General' && (<>
            <Field label="On startup"><select value={s.startup_mode} onChange={(e) => set({ startup_mode: e.target.value as any })}><option value="last">Reopen the workspaces that were open last time</option><option value="pinned">Open the pinned workspace only</option><option value="chooser">Show the workspace chooser</option></select></Field>
            <Field label="Pinned workspace" hint="Always opens first and is listed at the top of the tree."><select value={s.pinned_workspace ?? ''} onChange={(e) => set({ pinned_workspace: e.target.value || null })}><option value="">None</option>{known.map((p) => <option key={p} value={p}>{p}</option>)}</select></Field>
            <Field label="Restore tabs from the last session"><select value={s.restore_tabs} onChange={(e) => set({ restore_tabs: e.target.value as any })}><option value="ask">Offer on the welcome screen</option><option value="always">Always</option><option value="never">Never</option></select></Field>
            <label className="check"><input type="checkbox" checked={s.confirm_tab_close} onChange={(e) => set({ confirm_tab_close: e.target.checked })} /> Ask before closing a connected tab</label>
          </>)}
          {page === 'Appearance' && (<>
            <Field label="Theme"><select value={s.theme} onChange={(e) => set({ theme: e.target.value as any })}><option value="system">Follow the system</option><option value="light">Light</option><option value="dark">Dark</option></select></Field>
            <Field label="Accent color"><input type="color" value={s.accent} onChange={(e) => set({ accent: e.target.value })} style={{ width: 60, height: 30 }} /></Field>
            <Field label="Density"><select value={s.density} onChange={(e) => set({ density: e.target.value as any })}><option value="comfortable">Comfortable</option><option value="compact">Compact (more rows in the tree)</option></select></Field>
            <Field label={`Interface scale: ${Math.round(s.ui_scale * 100)}%`} hint="On top of the operating system's scale factor. Terminals keep their own font size."><input type="range" min={0.8} max={2} step={0.05} value={s.ui_scale} onChange={(e) => set({ ui_scale: Number(e.target.value) })} /></Field>
          </>)}
          {page === 'Security' && (<>
            <Field label="Lock after inactivity (minutes, 0 = never)"><input type="number" min={0} max={1440} value={s.auto_lock_minutes} onChange={(e) => set({ auto_lock_minutes: Math.min(1440, Math.max(0, Number(e.target.value) || 0)) })} /></Field>
            <label className="check"><input type="checkbox" checked={s.lock_on_sleep} onChange={(e) => set({ lock_on_sleep: e.target.checked })} /> Lock when the computer sleeps</label>
            <Field label="Clear copied passwords after (seconds)"><input type="number" min={0} max={600} value={s.clipboard_clear_seconds} onChange={(e) => set({ clipboard_clear_seconds: Math.min(600, Math.max(0, Number(e.target.value) || 0)) })} /></Field>
            <Field label="Automatic vault backups to keep" hint="A backup is taken each time a vault is unlocked."><input type="number" min={0} max={200} value={s.backup_retention} onChange={(e) => set({ backup_retention: Math.min(200, Math.max(0, Number(e.target.value) || 0)) })} /></Field>
            <b>Workspaces</b>
            {workspaces.map((w) => (
              <div key={w.info.id} style={{ border: '1px solid var(--border)', borderRadius: 6, padding: 10, display: 'flex', flexDirection: 'column', gap: 8 }}>
                <b>{w.info.name}</b>
                <div className="inline" style={{ flexWrap: 'wrap' }}>
                  {remembered[w.info.id] && <button className="sm" onClick={async () => { await call('ws_forget_device', { id: w.info.id }); setRemembered({ ...remembered, [w.info.id]: false }); toast('Forgotten on this device', 'success', 3000); }}>Forget on this device</button>}
                  <button className="sm" disabled={w.info.locked} onClick={() => setPw({ ws: w.info.id, old: '', next: '' })}>Change passphrase</button>
                  <button className="sm" disabled={w.info.locked} onClick={async () => { try { const dir = await call<string>('ws_backup_now', { id: w.info.id }); toast(`Backup saved in ${dir}`, 'success', 7000); } catch (e) { toastError(e); } }}>Back up now</button>
                </div>
                {pw?.ws === w.info.id && (
                  <div className="inline" style={{ flexWrap: 'wrap' }}>
                    <input type="password" placeholder="Current passphrase" value={pw.old} onChange={(e) => setPw({ ...pw, old: e.target.value })} />
                    <input type="password" placeholder="New passphrase" value={pw.next} onChange={(e) => setPw({ ...pw, next: e.target.value })} />
                    <button className="primary sm" onClick={async () => {
                      if (pw.next.length < 8) return toast('Use at least 8 characters', 'error');
                      try { await call('ws_change_passphrase', { id: pw.ws, old: pw.old, new: pw.next }); toast('Passphrase changed', 'success', 3000); setPw(null); }
                      catch (e) { toast(isApiError(e) && e.code === 'wrong_passphrase' ? 'That passphrase is not correct.' : errText(e), 'error'); }
                    }}>Change</button>
                  </div>
                )}
              </div>
            ))}
            <div className="dim small">Telemetry: none. okRemote sends nothing anywhere.</div>
          </>)}
          {page === 'Terminal' && (<>
            <div className="dim small">These are the defaults for every terminal. A terminal profile on a folder or connection overrides them.</div>
            <TerminalEditor value={s.terminal} onChange={(t) => { set({ terminal: t }); applyTerminalSettings(); }} />
            <button style={{ alignSelf: 'flex-start' }} onClick={() => openDialog('profiles', {})}>Workspace terminal profiles…</button>
          </>)}
          {page === 'Keyboard' && (
            <table>
              <thead><tr><th>Action</th><th>Shortcut</th><th /></tr></thead>
              <tbody>{Object.entries(DEFAULT_KEYS).map(([a, d]) => (
                <tr key={a}><td>{d.label}</td>
                  <td><kbd className="mono">{capturing === a ? 'Press keys…' : hint(bindingFor(a) ?? '')}</kbd></td>
                  <td className="inline"><button className="sm" onClick={() => setCapturing(a)}>Change</button>
                    {s.keybindings[a] && <button className="sm" onClick={() => { const k = { ...s.keybindings }; delete k[a]; set({ keybindings: k }); }}>Reset</button>}</td></tr>
              ))}</tbody>
            </table>
          )}
        </div>
      </div>
    </Modal>
  );
}

// ------------------------------------------------------------------ tunnels & profiles

function TunnelsDialog({ ws: initial }: { ws?: string }) {
  const list = wsList().filter((w) => w.snap);
  const [ws, setWs] = useState(initial ?? list[0]?.info.id ?? '');
  const snap = list.find((w) => w.info.id === ws)?.snap;
  const status = useStore((s) => s.tunnels) ?? { forwards: [], transports: [] };
  const [editing, setEditing] = useState<TunnelProfile | null>(null);
  useEffect(() => { void refreshTunnels(); }, []);
  if (editing && snap) return <TunnelEditor ws={ws} profile={editing} onDone={() => setEditing(null)} />;
  const fwText = (f: any) => f.remote_bind_port !== undefined ? `-R ${f.remote_bind_addr}:${f.remote_bind_port} → ${f.local_host}:${f.local_port}`
    : f.remote_host !== undefined ? `-L ${f.bind_addr}:${f.bind_port} → ${f.remote_host}:${f.remote_port}` : `-D ${f.bind_addr}:${f.bind_port} (SOCKS5)`;
  const exposes = (f: any) => !['', '127.0.0.1', '::1', 'localhost'].includes(f.bind_addr ?? f.remote_bind_addr ?? '');
  const running = (id: string) => status.forwards.some((f: any) => f.owner === `profile:${ws}:${id}`);
  return (
    <Modal title="Tunnels" width="xwide" onClose={closeDialog} footer={<button onClick={closeDialog}>Close</button>}>
      <div className="inline">
        <select style={{ maxWidth: 260 }} value={ws} onChange={(e) => setWs(e.target.value)}>{list.map((w) => <option key={w.info.id} value={w.info.id}>{w.info.name}</option>)}</select>
        <span className="spacer" />
        <button className="primary" disabled={!snap} onClick={() => setEditing({ id: crypto.randomUUID(), name: '', chain: [], forwards: [], auto_start: false, version: 0 })}>New profile</button>
      </div>
      {!snap || snap.tunnels.length === 0 ? <div className="dim">A tunnel profile is a chain of SSH jump hosts plus optional forwards, attached to a folder or connection under Edit → Tunnel.</div> : (
        <table>
          <thead><tr><th>Profile</th><th>Forwards</th><th /></tr></thead>
          <tbody>{snap.tunnels.map((t) => (
            <tr key={t.id}>
              <td><b>{t.name}</b><div className="dim small">{t.chain.map((id) => snap.connections.find((c) => c.id === id)?.name ?? '?').join(' → ') || 'no hops'}</div></td>
              <td>{t.forwards.length === 0 ? <span className="dim small">jump chain only</span> : t.forwards.map((f, i) => <div key={i} className="mono small">{fwText(f)} {exposes(f) && <span className="badge warn">network</span>}</div>)}</td>
              <td className="inline">
                {running(t.id)
                  ? <button className="sm" onClick={() => void call('tunnel_stop', { ws, id: t.id })}>Stop</button>
                  : <button className="sm" disabled={t.forwards.length === 0} onClick={() => void startTunnelProfile(ws, t.id, t.forwards.some(exposes))}>Start</button>}
                <button className="sm" onClick={() => setEditing(t)}>Edit</button></td>
            </tr>))}
          </tbody>
        </table>
      )}
      <b>Running</b>
      {status.forwards.length === 0 ? <div className="dim small">No forwards are running.</div> : (
        <table><tbody>{status.forwards.map((f: any) => (
          <tr key={f.id}>
            <td><span className={`dot ${f.health}`} /></td>
            <td>{f.label} {f.auto && <span className="badge">auto</span>}<div className="mono small">{f.kind} {f.listen} → {f.target}</div></td>
            <td className="small">{f.transport_label} · {f.active} active{f.retry ? ` · retry ${f.retry}` : ''}{f.error ? ` · ${f.error}` : ''}</td>
            <td><button className="sm" onClick={() => void call('forward_stop', { id: f.id })}>Stop</button></td>
          </tr>))}</tbody></table>
      )}
      <b>SSH transports</b>
      {status.transports.length === 0 ? <div className="dim small">None.</div> : (
        <table><tbody>{status.transports.map((t: any) => (
          <tr key={t.key}><td><span className={`dot ${t.health}`} /></td><td>{t.label}</td><td className="small">{t.health} · {t.users} users{t.error ? ` · ${t.error}` : ''}</td></tr>
        ))}</tbody></table>
      )}
    </Modal>
  );
}

function TunnelEditor({ ws, profile, onDone }: { ws: string; profile: TunnelProfile; onDone: () => void }) {
  const snap = useStore((s) => s.workspaces[ws]?.snap)!;
  const [t, setT] = useState<TunnelProfile>(structuredClone(profile));
  const sshConns = snap.connections.filter((c) => c.protocol === 'ssh');
  const kindOf = (f: any) => (f.remote_bind_port !== undefined ? 'remote' : f.remote_host !== undefined ? 'local' : 'dynamic');
  const setF = (i: number, f: any) => setT({ ...t, forwards: t.forwards.map((x, j) => (j === i ? f : x)) });
  const mk = (k: string): any => k === 'local' ? { kind: 'local', bind_addr: '127.0.0.1', bind_port: 8080, remote_host: 'localhost', remote_port: 80 }
    : k === 'dynamic' ? { kind: 'dynamic', bind_addr: '127.0.0.1', bind_port: 1080 } : { kind: 'remote', remote_bind_addr: '127.0.0.1', remote_bind_port: 9000, local_host: 'localhost', local_port: 9000 };
  const save = async () => {
    if (!t.name.trim()) return toast('A name is required', 'error');
    if (t.chain.length === 0) return toast('Add at least one hop', 'error');
    try { await call('tunnel_save', { ws, tunnel: { ...t, name: t.name.trim() } }); await loadSnapshot(ws); onDone(); } catch (e) { toastError(e); }
  };
  const num = (v: any, f: (n: number) => void) => <input type="number" style={{ width: 80 }} value={v} onChange={(e) => f(Number(e.target.value))} />;
  return (
    <Modal title={profile.version ? `Edit ${profile.name}` : 'New tunnel profile'} width="wide" onClose={onDone}
      footer={<>{profile.version > 0 && <button className="danger" onClick={async () => { const { confirmDialog } = await import('../actions'); const r = await confirmDialog({ title: 'Delete tunnel profile?', ok: 'Delete', danger: true }); if (!r.ok) return; await call('tunnel_delete', { ws, id: t.id }); await loadSnapshot(ws); onDone(); }}>Delete</button>}<span className="spacer" /><button onClick={onDone}>Cancel</button><button className="primary" onClick={() => void save()}>Save</button></>}>
      <Field label="Name"><input type="text" autoFocus value={t.name} onChange={(e) => setT({ ...t, name: e.target.value })} /></Field>
      <Field label="Jump chain">
        {t.chain.map((id, i) => {
          const c = snap.connections.find((x) => x.id === id);
          return (
            <div key={i} className="inline"><span style={{ flex: 1 }}>{c ? `${c.name} (${c.host})` : 'missing connection'}</span>
              <button className="icon" disabled={i === 0} onClick={() => { const ch = [...t.chain]; [ch[i - 1], ch[i]] = [ch[i], ch[i - 1]]; setT({ ...t, chain: ch }); }}>↑</button>
              <button className="icon" disabled={i === t.chain.length - 1} onClick={() => { const ch = [...t.chain]; [ch[i + 1], ch[i]] = [ch[i], ch[i + 1]]; setT({ ...t, chain: ch }); }}>↓</button>
              <button className="icon" onClick={() => setT({ ...t, chain: t.chain.filter((_, j) => j !== i) })}><Icon name="close" size={14} /></button></div>
          );
        })}
        <select value="" onChange={(e) => e.target.value && setT({ ...t, chain: [...t.chain, e.target.value] })}>
          <option value="">Add a hop…</option>{sshConns.map((c) => <option key={c.id} value={c.id}>{folderPath(snap, c.folder_id) ? folderPath(snap, c.folder_id) + ' / ' : ''}{c.name} ({c.host})</option>)}
        </select>
      </Field>
      <Field label="Forwards">
        {t.forwards.map((f: any, i) => {
          const k = kindOf(f);
          const exposed = !['', '127.0.0.1', '::1', 'localhost'].includes(f.bind_addr ?? f.remote_bind_addr ?? '');
          return (
            <div key={i}>
              <div className="inline">
                <select style={{ width: 150, flex: 'none' }} value={k} onChange={(e) => setF(i, mk(e.target.value))}><option value="local">Local (-L)</option><option value="dynamic">Dynamic SOCKS (-D)</option><option value="remote">Remote (-R)</option></select>
                <input type="text" value={f.bind_addr ?? f.remote_bind_addr} onChange={(e) => setF(i, k === 'remote' ? { ...f, remote_bind_addr: e.target.value } : { ...f, bind_addr: e.target.value })} />
                {num(f.bind_port ?? f.remote_bind_port, (n) => setF(i, k === 'remote' ? { ...f, remote_bind_port: n } : { ...f, bind_port: n }))}
                {k !== 'dynamic' && <>→<input type="text" value={f.remote_host ?? f.local_host} onChange={(e) => setF(i, k === 'remote' ? { ...f, local_host: e.target.value } : { ...f, remote_host: e.target.value })} />
                  {num(f.remote_port ?? f.local_port, (n) => setF(i, k === 'remote' ? { ...f, local_port: n } : { ...f, remote_port: n }))}</>}
                <button className="icon" onClick={() => setT({ ...t, forwards: t.forwards.filter((_, j) => j !== i) })}><Icon name="close" size={14} /></button>
              </div>
              {exposed && <div className="warn-text small">Reachable from the network</div>}
            </div>
          );
        })}
        <button className="sm" style={{ alignSelf: 'flex-start' }} onClick={() => setT({ ...t, forwards: [...t.forwards, mk('local')] })}>Add forward</button>
      </Field>
      <label className="check"><input type="checkbox" checked={t.auto_start} onChange={(e) => setT({ ...t, auto_start: e.target.checked })} /> Start the forwards whenever a connection using this profile opens</label>
    </Modal>
  );
}

function ProfilesDialog({ ws: initial }: { ws?: string }) {
  const list = wsList().filter((w) => w.snap);
  const global = useStore((s) => s.settings.terminal);
  const [ws, setWs] = useState(initial ?? list[0]?.info.id ?? '');
  const snap = list.find((w) => w.info.id === ws)?.snap;
  const [sel, setSel] = useState<string | null>(null);
  const [draft, setDraft] = useState<TermProfile | null>(null);
  useEffect(() => { const p = snap?.terminal_profiles.find((x) => x.id === sel); setDraft(p ? structuredClone(p) : null); }, [sel]); // eslint-disable-line react-hooks/exhaustive-deps
  const save = async () => {
    if (!draft) return;
    try {
      const saved = await call<TermProfile>('profile_save', { ws, profile: draft });
      await loadSnapshot(ws); setSel(saved.id); setDraft(saved); toast('Profile saved', 'success', 2000); applyTerminalSettings();
    } catch (e) { toastError(e); }
  };
  return (
    <Modal title="Terminal profiles" width="xwide" onClose={closeDialog} footer={<button onClick={closeDialog}>Close</button>}>
      <div className="inline">
        <select style={{ maxWidth: 260 }} value={ws} onChange={(e) => { setWs(e.target.value); setSel(null); }}>{list.map((w) => <option key={w.info.id} value={w.info.id}>{w.info.name}</option>)}</select>
        <span className="spacer" />
        <button className="primary" disabled={!snap} onClick={() => setDraft({ id: crypto.randomUUID(), name: 'New profile', spec: { ...global } as any, version: 0 })}>New profile</button>
      </div>
      <div className="two">
        <div className="list">
          {snap?.terminal_profiles.length === 0 && !draft && <div className="dim small">No profiles yet. Connections use the app default from Settings → Terminal.</div>}
          {snap?.terminal_profiles.map((p) => <button key={p.id} aria-pressed={sel === p.id} onClick={() => setSel(p.id)}>{p.name}</button>)}
        </div>
        {draft && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
            <Field label="Profile name"><input type="text" value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} /></Field>
            <TerminalEditor value={{ ...global, ...(draft.spec as any) }} onChange={(t) => setDraft({ ...draft, spec: t as any })} />
            <div className="inline">
              {draft.version > 0 && <button className="danger" onClick={async () => { const { confirmDialog } = await import('../actions'); const r = await confirmDialog({ title: 'Delete profile?', body: `${draft.name} will be removed; connections using it fall back to the app default.`, ok: 'Delete', danger: true }); if (!r.ok) return; await call('profile_delete', { ws, id: draft.id }); await loadSnapshot(ws); setSel(null); setDraft(null); }}>Delete</button>}
              <span className="spacer" /><button className="primary" onClick={() => void save()}>Save profile</button>
            </div>
          </div>
        )}
      </div>
    </Modal>
  );
}

// ------------------------------------------------------------------ host keys, audit, about

function HostKeysDialog() {
  const list = wsList().filter((w) => w.snap);
  const [store, setStore] = useState<string>(list[0]?.info.id ?? 'device');
  const [keys, setKeys] = useState<HostKey[]>([]);
  const [q, setQ] = useState('');
  const load = () => call<HostKey[]>('hostkeys_list', { ws: store === 'device' ? null : store }).then(setKeys).catch(toastError);
  useEffect(() => { void load(); }, [store]); // eslint-disable-line react-hooks/exhaustive-deps
  const shown = keys.filter((k) => `${k.host}:${k.port} ${k.key_type} ${k.fingerprint}`.toLowerCase().includes(q.toLowerCase()));
  return (
    <Modal title="Trusted host keys" width="xwide" onClose={closeDialog} footer={<button onClick={closeDialog}>Close</button>}>
      <div className="inline">
        <select style={{ maxWidth: 300 }} value={store} onChange={(e) => setStore(e.target.value)}>
          {list.map((w) => <option key={w.info.id} value={w.info.id}>{w.info.name} (shared with the workspace)</option>)}<option value="device">This device (quick connect)</option>
        </select>
        <input type="text" placeholder="Filter" value={q} onChange={(e) => setQ(e.target.value)} />
      </div>
      {shown.length === 0 ? <div className="dim">No trusted keys yet. Keys are recorded the first time you trust a server.</div> : (
        <table>
          <thead><tr><th>Host</th><th>Type</th><th>Fingerprint</th><th>First seen</th><th /></tr></thead>
          <tbody>{shown.map((k) => (
            <tr key={`${k.host}${k.port}${k.key_type}`}><td>{k.host}:{k.port}</td><td>{k.key_type}</td><td className="mono small" style={{ wordBreak: 'break-all' }}>{k.fingerprint}</td>
              <td className="small">{k.first_seen.slice(0, 10)} by {k.trusted_by}</td>
              <td><button className="danger sm" onClick={async () => { await call('hostkey_delete', { ws: store === 'device' ? null : store, host: k.host, port: k.port, keyType: k.key_type }); void load(); }}>Remove</button></td></tr>
          ))}</tbody>
        </table>
      )}
    </Modal>
  );
}

function AuditDialog({ ws: initial }: { ws?: string }) {
  const list = wsList().filter((w) => w.snap);
  const [ws, setWs] = useState(initial ?? list[0]?.info.id ?? '');
  const [rows, setRows] = useState<AuditEntry[]>([]);
  const [action, setAction] = useState('');
  const [q, setQ] = useState('');
  useEffect(() => { if (ws) void call<AuditEntry[]>('audit_list', { ws, limit: 5000 }).then(setRows).catch(toastError); }, [ws]);
  const snap = snapOf(ws);
  const nameOf = (r: AuditEntry) => snap?.connections.find((c) => c.id === r.target_id)?.name ?? snap?.folders.find((f) => f.id === r.target_id)?.name ?? snap?.tunnels.find((t) => t.id === r.target_id)?.name ?? r.target_id.slice(0, 8);
  const actions = useMemo(() => [...new Set(rows.map((r) => r.action))].sort(), [rows]);
  const shown = rows.filter((r) => (!action || r.action === action) && `${r.actor} ${r.action} ${nameOf(r)} ${JSON.stringify(r.detail)}`.toLowerCase().includes(q.toLowerCase())).slice(0, 1000);
  return (
    <Modal title="Audit log" width="xwide" onClose={closeDialog}
      footer={<><button onClick={async () => { const p = await pickSavePath('okremote-audit.csv', 'CSV', 'csv'); if (!p) return; try { const n = await call('audit_export_csv', { ws, path: p }); toast(`Exported ${n} entries`, 'success'); } catch (e) { toastError(e); } }}>Export CSV…</button><button onClick={closeDialog}>Close</button></>}>
      <div className="inline">
        <select style={{ maxWidth: 220 }} value={ws} onChange={(e) => setWs(e.target.value)}>{list.map((w) => <option key={w.info.id} value={w.info.id}>{w.info.name}</option>)}</select>
        <select style={{ maxWidth: 200 }} value={action} onChange={(e) => setAction(e.target.value)}><option value="">All actions</option>{actions.map((a) => <option key={a}>{a}</option>)}</select>
        <input type="text" placeholder="Filter" value={q} onChange={(e) => setQ(e.target.value)} />
      </div>
      <div className="dim small">Secret fields are recorded as changed, never as values.</div>
      <table>
        <thead><tr><th>When (UTC)</th><th>Who</th><th>Action</th><th>Item</th></tr></thead>
        <tbody>{shown.map((r) => <tr key={r.id}><td className="mono small">{r.at.replace('T', ' ').replace('Z', '')}</td><td>{r.actor}</td><td>{r.action}</td><td>{nameOf(r)}</td></tr>)}</tbody>
      </table>
    </Modal>
  );
}

function AboutDialog() {
  const info = useStore((s) => s.info);
  return (
    <Modal title="About okRemote" onClose={closeDialog} footer={<button onClick={closeDialog}>Close</button>}>
      <b>okRemote {info?.version} ({info?.platform})</b>
      <div>Codename build. Connection manager for SSH, RDP, VNC and web sessions.</div>
      <div className="codebox selectable">{info?.config_dir}</div>
      {info?.portable && <span className="badge">Portable mode</span>}
    </Modal>
  );
}

// ------------------------------------------------------------------ prompts raised by the core

export function Prompts() {
  const p = useStore((s) => s.prompts[0]);
  if (!p) return null;
  return <PromptView key={p.id} p={p} />;
}

function PromptView({ p }: { p: any }) {
  const [v, setV] = useState<any>(p.kind === 'credentials' ? { user: p.user || p.default_user || '', password: '', domain: p.domain || '' } : '');
  const [answers, setAnswers] = useState<string[]>((p.prompts ?? []).map(() => ''));
  const cancel = () => respondPrompt(p.id, null);
  if (p.kind === 'hostkey_changed') {
    return (
      <Modal title="Host key changed: connection blocked" width="wide" onClose={cancel}
        footer={<><button onClick={cancel}>Close</button><button className="primary" onClick={() => { respondPrompt(p.id, 'ack'); openDialog('hostkeys', {}); }}>Open Host keys</button></>}>
        <div className="danger-text">The key offered by {p.host}:{p.port} is different from the one you trusted. Someone could be intercepting the connection, or the server was reinstalled.</div>
        <Field label="Trusted fingerprint"><div className="codebox selectable">{p.old}</div></Field>
        <Field label="Fingerprint offered now"><div className="codebox selectable">{p.new}</div></Field>
        <div className="small">If you have verified the change, remove the old key under Host keys and connect again.</div>
      </Modal>
    );
  }
  if (p.kind === 'hostkey_unknown') {
    return (
      <Modal title="Unknown host key" width="wide" onClose={cancel}
        footer={<><button onClick={cancel}>Cancel</button><button onClick={() => respondPrompt(p.id, 'once')}>Connect once</button><button className="primary" onClick={() => respondPrompt(p.id, 'trust')}>Trust and connect</button></>}>
        <div>This is the first connection to {p.host}:{p.port} ({p.label}). Check the fingerprint with the server's administrator before trusting it.</div>
        <Field label={p.key_type}><div className="codebox selectable">{p.fingerprint}</div></Field>
        {p.other_keys?.length > 0 && <div className="warn-text">This host is already known with other key types: {p.other_keys.join(', ')}.</div>}
        <div className="dim small">A trusted key is saved {p.shared ? 'in the workspace (shared)' : 'on this device'}.</div>
      </Modal>
    );
  }
  if (p.kind === 'credentials') {
    const need = (p.need_user && !v.user) || (p.need_password && !v.password);
    return (
      <Modal title={p.need_password ? 'Sign in' : 'User name'} onClose={cancel}
        footer={<><button onClick={cancel}>Cancel</button><button className="primary" disabled={need} onClick={() => respondPrompt(p.id, v)}>Connect</button></>}>
        <div className="dim2">No saved credentials for {p.name} ({p.host}). They are used for this connection only and are not saved.</div>
        <Field label="User name"><input type="text" autoFocus value={v.user} onChange={(e) => setV({ ...v, user: e.target.value })} /></Field>
        {p.protocol === 'rdp' && <Field label="Domain (optional)"><input type="text" value={v.domain} onChange={(e) => setV({ ...v, domain: e.target.value })} /></Field>}
        {p.protocol !== 'ssh' && <Field label="Password"><input type="password" value={v.password} onChange={(e) => setV({ ...v, password: e.target.value })} onKeyDown={(e) => e.key === 'Enter' && !need && respondPrompt(p.id, v)} /></Field>}
      </Modal>
    );
  }
  if (p.kind === 'password' || p.kind === 'key_passphrase') {
    return (
      <Modal title={p.kind === 'password' ? 'Password' : 'Private key passphrase'} subtitle={p.kind === 'password' ? `${p.user}@${p.host} · ${p.label}` : `Key: ${p.key || 'stored key'}`} onClose={cancel}
        footer={<><button onClick={cancel}>Cancel</button><button className="primary" onClick={() => respondPrompt(p.id, v)}>OK</button></>}>
        <input type="password" autoFocus value={v} onChange={(e) => setV(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && respondPrompt(p.id, v)} />
      </Modal>
    );
  }
  if (p.kind === 'keyboard_interactive') {
    return (
      <Modal title={p.name || 'Authentication'} subtitle={`${p.host} · ${p.label}`} onClose={cancel}
        footer={<><button onClick={cancel}>Cancel</button><button className="primary" onClick={() => respondPrompt(p.id, answers)}>Continue</button></>}>
        {p.instructions && <pre className="pre">{p.instructions}</pre>}
        {p.prompts.map((pr: any, i: number) => (
          <Field key={i} label={pr.prompt}><input type={pr.echo ? 'text' : 'password'} autoFocus={i === 0} value={answers[i]} onChange={(e) => setAnswers(answers.map((a, j) => (j === i ? e.target.value : a)))} onKeyDown={(e) => e.key === 'Enter' && respondPrompt(p.id, answers)} /></Field>
        ))}
      </Modal>
    );
  }
  return null;
}

