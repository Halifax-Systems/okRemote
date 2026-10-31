import { useEffect, useMemo, useState } from 'react';
import { useStore, setState } from '../store';
import { call, isApiError } from '../api';
import { closeDialog, confirmDialog, copySecretField, loadSnapshot, openConnection, openDialog, rowKey, toast, toastError } from '../actions';
import { Field, Icon } from '../ui';
import type { Conn, Folder, Opts, Protocol, Snapshot } from '../types';

type Kind = 'text' | 'number' | 'select' | 'bool' | 'float';
interface OptField {
  key: string; label: string; section: string; kind: Kind; protos?: Protocol[]; options?: [string, string][];
  placeholder?: string; hint?: string; warn?: string; def?: string;
}
const SSH: Protocol[] = ['ssh'], RDP: Protocol[] = ['rdp'], VNC: Protocol[] = ['vnc'], WEB: Protocol[] = ['http', 'https'], EXT: Protocol[] = ['external'];

const FIELDS: OptField[] = [
  { key: 'auth_method', label: 'Authentication', section: 'Credentials', kind: 'select', protos: SSH, def: 'auto', options: [['auto', 'Automatic (key, agent, password)'], ['password', 'Password'], ['key', 'Private key'], ['agent', 'SSH agent'], ['keyboard_interactive', 'Keyboard-interactive']] },
  { key: 'key_file', label: 'Private key file', section: 'Credentials', kind: 'text', protos: SSH, placeholder: '~/.ssh/id_ed25519' },
  { key: 'tunnel_profile', label: 'Tunnel profile', section: 'Tunnel', kind: 'select', hint: 'The connection routes through this jump chain; other protocols get a local forward on an ephemeral port.' },
  { key: 'rdp_scaling', label: 'Scaling', section: 'Display', kind: 'select', protos: RDP, def: 'dynamic', options: [['dynamic', 'Dynamic resolution'], ['smart', 'Smart size (scale to fit)'], ['fixed', 'Fixed resolution'], ['scroll', 'Fixed, with scroll']] },
  { key: 'rdp_width', label: 'Width', section: 'Display', kind: 'number', protos: RDP },
  { key: 'rdp_height', label: 'Height', section: 'Display', kind: 'number', protos: RDP },
  { key: 'rdp_scale_factor', label: 'Desktop scale (%)', section: 'Display', kind: 'select', protos: RDP, def: '100', options: ['100', '125', '150', '175', '200', '250', '300', '400', '500'].map((v) => [v, v]) },
  { key: 'rdp_color_depth', label: 'Color depth', section: 'Display', kind: 'select', protos: RDP, def: '32', options: ['32', '24', '16', '15', '8'].map((v) => [v, `${v} bit`]) },
  { key: 'rdp_fullscreen', label: 'Full screen', section: 'Display', kind: 'bool', protos: RDP },
  { key: 'rdp_clipboard', label: 'Clipboard redirection', section: 'Display', kind: 'bool', protos: RDP, def: 'true' },
  { key: 'rdp_drives', label: 'Drive redirection', section: 'Display', kind: 'bool', protos: RDP },
  { key: 'rdp_printers', label: 'Printer redirection', section: 'Display', kind: 'bool', protos: RDP },
  { key: 'vnc_view_only', label: 'View only', section: 'Display', kind: 'bool', protos: VNC },
  { key: 'vnc_remote_resize', label: 'Ask the server to resize', section: 'Display', kind: 'bool', protos: VNC },
  { key: 'http_open_in', label: 'Open in', section: 'Display', kind: 'select', protos: WEB, def: 'embedded', hint: 'Tunneled web connections always use an okRemote window.', options: [['embedded', 'okRemote window'], ['system', 'System browser']] },
  { key: 'http_zoom', label: 'Zoom', section: 'Display', kind: 'select', protos: WEB, def: '1', options: [['0.75', '75%'], ['0.9', '90%'], ['1', '100%'], ['1.1', '110%'], ['1.25', '125%'], ['1.5', '150%'], ['2', '200%']] },
  { key: 'http_path', label: 'Path', section: 'Display', kind: 'text', protos: WEB, placeholder: '/admin' },
  { key: 'terminal_profile', label: 'Terminal profile', section: 'Terminal', kind: 'select', protos: SSH, hint: 'Overrides the app default from Settings → Terminal.' },
  { key: 'term_type', label: 'TERM', section: 'Terminal', kind: 'text', protos: SSH, placeholder: 'xterm-256color' },
  { key: 'fixed_cols', label: 'Fixed columns', section: 'Terminal', kind: 'number', protos: SSH, hint: 'Leave empty to fit the pane.' },
  { key: 'fixed_rows', label: 'Fixed rows', section: 'Terminal', kind: 'number', protos: SSH, hint: 'Leave empty to fit the pane.' },
  { key: 'session_log', label: 'Log session to a file', section: 'Terminal', kind: 'bool', protos: SSH, warn: 'The log can contain secrets typed into the session, such as passwords at sudo prompts.' },
  { key: 'session_log_dir', label: 'Log folder', section: 'Terminal', kind: 'text', protos: SSH, placeholder: "Default: the app's logs folder" },
  { key: 'session_log_timestamps', label: 'Timestamp each line', section: 'Terminal', kind: 'bool', protos: SSH },
  { key: 'osc52', label: 'Allow remote programs to set the clipboard (OSC 52)', section: 'Terminal', kind: 'bool', protos: SSH, warn: 'A compromised server could overwrite your clipboard.' },
  { key: 'keepalive_seconds', label: 'Keepalive (seconds, 0 = off)', section: 'Advanced', kind: 'number', protos: SSH, def: '30' },
  { key: 'allow_sha1_rsa', label: 'Allow ssh-rsa (SHA-1) host keys', section: 'Advanced', kind: 'bool', protos: SSH, warn: 'SHA-1 signatures are weak. Enable only for old servers.' },
  { key: 'rdp_nla', label: 'Network Level Authentication', section: 'Advanced', kind: 'bool', protos: RDP, def: 'true' },
  { key: 'rdp_admin', label: 'Admin / console session', section: 'Advanced', kind: 'bool', protos: RDP },
  { key: 'rdp_client_path', label: 'RDP client', section: 'Advanced', kind: 'text', protos: RDP, placeholder: 'Default: mstsc / FreeRDP / Windows App' },
  { key: 'vnc_client_path', label: 'VNC viewer', section: 'Advanced', kind: 'text', protos: VNC, placeholder: 'Default: TigerVNC or Screen Sharing' },
  { key: 'ext_command', label: 'Command line', section: 'Advanced', kind: 'text', protos: EXT, placeholder: 'putty.exe -telnet %HOST% %PORT%', hint: 'Variables: %NAME% %HOST% %PORT% %USER% %DOMAIN%. The password is passed through $OKREMOTE_PASSWORD or stdin, never on the command line.' },
  { key: 'ext_password_via', label: 'Pass the password through', section: 'Advanced', kind: 'select', protos: EXT, def: 'env', options: [['env', 'Environment variable'], ['stdin', 'Standard input'], ['none', "Don't pass it"]] },
  { key: 'ext_working_dir', label: 'Working folder', section: 'Advanced', kind: 'text', protos: EXT },
];

const SWATCHES = ['#ef4444', '#f97316', '#eab308', '#22c55e', '#14b8a6', '#3b82f6', '#8b5cf6', '#ec4899', '#64748b'];
const SECTIONS = ['General', 'Credentials', 'Tunnel', 'Display', 'Terminal', 'Advanced'];
type Draft = (Conn | Folder) & { __kind?: string };

export function Editor() {
  const editor = useStore((s) => s.editor);
  const snap = useStore((s) => (s.editor ? s.workspaces[s.editor.ws]?.snap ?? null : null));
  if (!editor || !snap) return null;
  return <EditorInner key={`${editor.ws}:${editor.id}`} />;
}

function EditorInner() {
  const editor = useStore((s) => s.editor)!;
  const info = useStore((s) => s.workspaces[editor.ws]?.info);
  const snap = useStore((s) => s.workspaces[editor.ws]?.snap)!;
  const isFolder = editor.kind === 'folder';
  const original = useMemo<Draft | null>(() => {
    if (editor.blank) return editor.blank;
    return (isFolder ? snap.folders.find((f) => f.id === editor.id) : snap.connections.find((c) => c.id === editor.id)) ?? null;
  }, []); // eslint-disable-line react-hooks/exhaustive-deps
  const [draft, setDraft] = useState<Draft | null>(original ? structuredClone(original) : null);
  const [section, setSection] = useState('General');
  const [creds, setCreds] = useState<Record<string, string | null>>({});
  const [summary, setSummary] = useState<any>(null);
  const [revealed, setRevealed] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);
  const isNew = !!editor.blank;
  const readOnly = !!info?.read_only;

  useEffect(() => {
    if (isNew || !original) return;
    call('secret_summary', { ws: editor.ws, owner: isFolder ? 'folder' : 'connection', id: editor.id }).then(setSummary).catch(() => {});
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  if (!original || !draft) return null;
  const proto: Protocol | 'all' = isFolder ? 'all' : (draft as Conn).protocol;
  const optsOf = (d: Draft): Opts => (isFolder ? (d as Folder).defaults : (d as Conn).options);
  const setOpts = (o: Opts) => setDraft(isFolder ? { ...(draft as Folder), defaults: o } : { ...(draft as Conn), options: o });
  const dirty = JSON.stringify(draft) !== JSON.stringify(original) || Object.keys(creds).length > 0;
  const fieldsFor = (sec: string) => FIELDS.filter((f) => f.section === sec && (proto === 'all' || !f.protos || f.protos.includes(proto)));
  const sections = SECTIONS.filter((s) => s === 'General' || s === 'Credentials' || fieldsFor(s).length > 0);

  const close = async () => {
    if (dirty) {
      const r = await confirmDialog({ title: 'Discard changes?', body: 'Your edits have not been saved.', ok: 'Discard', danger: true });
      if (!r.ok) return;
    }
    setState({ editor: null });
  };

  // inheritance marks for option fields
  const inheritedFrom = (key: string): { from: string | null; value: unknown } => {
    const start = isFolder ? (draft as Folder).parent_id : (draft as Conn).folder_id;
    let cur = start;
    const seen = new Set<string>();
    while (cur && !seen.has(cur)) {
      seen.add(cur);
      const f = snap.folders.find((x) => x.id === cur);
      if (!f) break;
      if (key in f.defaults && f.defaults[key] !== null) return { from: f.name, value: f.defaults[key] };
      cur = f.parent_id;
    }
    return { from: null, value: undefined };
  };

  const folderOptions = [{ id: '', label: 'Top level' }, ...snap.folders.filter((f) => f.id !== (isFolder ? editor.id : '')).map((f) => ({ id: f.id, label: pathOf(snap, f.id) }))];

  const save = async (connect: boolean) => {
    const name = draft.name.trim();
    if (!name) return toast('A name is required', 'error');
    if (!isFolder && (draft as Conn).protocol !== 'external' && !(draft as Conn).host.trim()) return toast('A host is required', 'error');
    setSaving(true);
    try {
      let rec: any = { ...draft, name };
      if (!isFolder) rec.host = (draft as Conn).host.trim();
      let saved: any;
      for (let attempt = 0; attempt < 5; attempt++) {
        try {
          saved = await call(isFolder ? 'folder_save' : 'conn_save', { ws: editor.ws, [isFolder ? 'folder' : 'conn']: rec });
          break;
        } catch (e) {
          if (isApiError(e) && e.code === 'conflict') {
            const merged = await resolveConflict(rec, e.detail);
            if (!merged) { setState({ editor: null }); await loadSnapshot(editor.ws); return; }
            rec = merged;
          } else throw e;
        }
      }
      if (Object.keys(creds).length) await call('secret_patch', { ws: editor.ws, owner: isFolder ? 'folder' : 'connection', id: saved.id, patch: creds });
      await loadSnapshot(editor.ws);
      toast(`Saved ${name}`, 'success', 2500);
      const key = rowKey(editor.ws, isFolder ? 'folder' : 'conn', saved.id);
      setState({ editor: null, selection: [key], focus: key, anchor: key });
      if (connect && !isFolder) void openConnection(editor.ws, saved.id);
    } catch (e) { toastError(e); } finally { setSaving(false); }
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === 'Escape') { e.stopPropagation(); void close(); }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') { e.preventDefault(); void save(false); }
  };

  const del = async () => {
    const { deleteItems } = await import('../actions');
    await deleteItems([{ ws: editor.ws, kind: isFolder ? 'folder' : 'conn', id: editor.id }]);
  };

  const credRow = (field: string, label: string, secret: boolean, rows = 1) => {
    const sm = summary?.[field];
    const pending = field in creds;
    const own = !!sm?.own && !(pending && creds[field] === null);
    const inh = sm && !sm.own && sm.inherited_from;
    const val = pending ? creds[field] ?? '' : revealed[field] ?? (!secret && sm?.own ? sm.value ?? '' : '');
    const placeholder = pending && creds[field] === null ? 'Inherits from the folder' : own && secret && !(field in revealed) ? '•••••••• (stored)'
      : inh ? `Inherited from ${sm.inherited_from}${!secret && sm.value ? `: ${sm.value}` : ''}` : 'Not set';
    const Tag: any = rows > 1 ? 'textarea' : 'input';
    return (
      <Field key={field} label={label} mark={inh && !own && !pending ? <span className="inh">↳ {sm.inherited_from}</span> : null}>
        <div className="inline">
          <Tag type={secret && rows === 1 && !(field in revealed) ? 'password' : 'text'} rows={rows} value={val} placeholder={placeholder} className={rows > 1 ? 'mono' : ''}
            autoComplete="off" onChange={(e: any) => setCreds({ ...creds, [field]: e.target.value })} />
          {secret && !isNew && sm?.own && !pending && (
            <button className="icon" title="Reveal (recorded in the audit log)" aria-label={`Reveal ${label}`}
              onClick={async () => { try { const s = await call('secret_get', { ws: editor.ws, owner: isFolder ? 'folder' : 'connection', id: editor.id }); setRevealed({ ...revealed, [field]: s[field] ?? '' }); } catch (e) { toastError(e); } }}><Icon name="eye" /></button>
          )}
          {!isNew && (field === 'username' || field === 'password') && (
            <button className="icon" title={`Copy ${label.toLowerCase()}`} aria-label={`Copy ${label}`} onClick={() => void copySecretField(editor.ws, isFolder ? 'folder' : 'connection', editor.id, field)}><Icon name="copy" /></button>
          )}
          {(own || pending) && (
            <button className="icon" title="Clear, and inherit from the folder" aria-label={`Clear ${label}`} onClick={() => setCreds({ ...creds, [field]: null })}><Icon name="reset" /></button>
          )}
          {pending && <button className="link small" onClick={() => { const c = { ...creds }; delete c[field]; setCreds(c); }}>Undo</button>}
        </div>
      </Field>
    );
  };

  const o = optsOf(draft);
  const optControl = (f: OptField) => {
    const set = f.key in o && o[f.key] !== null && o[f.key] !== undefined;
    const inh = inheritedFrom(f.key);
    const value = set ? o[f.key] : undefined;
    const update = (v: unknown) => {
      const next = { ...o };
      if (v === undefined || v === '') delete next[f.key]; else next[f.key] = v;
      setOpts(next);
    };
    const mark = set ? (
      <button className="icon" style={{ width: 20, height: 20 }} title="Reset to the inherited value" aria-label={`Reset ${f.label}`} onClick={() => update(undefined)}><Icon name="reset" size={12} /></button>
    ) : <span className="inh">↳ {inh.from ? `from ${inh.from}` : 'default'}</span>;
    const inhText = inh.value !== undefined ? String(inh.value) : f.def ?? '';
    let control: React.ReactNode;
    if (f.kind === 'bool') {
      const eff = set ? value === true : inh.value !== undefined ? inh.value === true : f.def === 'true';
      return (
        <Field key={f.key} hint={f.warn && eff ? <span className="warn-text">{f.warn}</span> : f.hint}>
          <label className="check"><input type="checkbox" checked={eff} onChange={(e) => update(e.target.checked)} /> <span>{f.label} {mark}</span></label>
        </Field>
      );
    }
    if (f.kind === 'select') {
      let options = f.options ?? [];
      let blank = `Inherit (${inhText || 'default'})`;
      if (f.key === 'tunnel_profile') { options = [['none', 'None (connect directly)'], ...snap.tunnels.map((t) => [t.id, t.name] as [string, string])]; blank = `Inherit${inh.value ? ` (${snap.tunnels.find((t) => t.id === inh.value)?.name ?? ''})` : ''}`; }
      if (f.key === 'terminal_profile') { options = snap.terminal_profiles.map((t) => [t.id, t.name] as [string, string]); blank = `Inherit (${snap.terminal_profiles.find((t) => t.id === inh.value)?.name ?? 'app default'})`; }
      control = (
        <select value={set ? String(value) : ''} onChange={(e) => {
          const raw = e.target.value;
          const numeric = ['rdp_scale_factor', 'rdp_color_depth', 'http_zoom'].includes(f.key);
          update(raw === '' ? undefined : numeric ? Number(raw) : raw);
        }}>
          <option value="">{blank}</option>
          {options.map(([v, l]) => <option key={v} value={v}>{l}</option>)}
        </select>
      );
    } else if (f.kind === 'number') {
      control = <input type="number" value={set ? String(value) : ''} placeholder={inhText} onChange={(e) => update(e.target.value === '' ? undefined : Number(e.target.value))} />;
    } else {
      control = <input type="text" value={set ? String(value) : ''} placeholder={inh.from ? inhText : f.placeholder ?? inhText} onChange={(e) => update(e.target.value)} />;
    }
    return <Field key={f.key} label={f.label} mark={mark} hint={f.hint}>{control}{f.key === 'tunnel_profile' && <button className="sm" style={{ alignSelf: 'flex-start' }} onClick={() => openDialog('tunnels', { ws: editor.ws })}>Manage tunnel profiles</button>}{f.key === 'terminal_profile' && <button className="sm" style={{ alignSelf: 'flex-start' }} onClick={() => openDialog('profiles', { ws: editor.ws })}>Manage terminal profiles</button>}</Field>;
  };

  const c = draft as Conn;
  const ph = proto === 'ssh' ? '22' : proto === 'rdp' ? '3389' : proto === 'vnc' ? '5900' : proto === 'http' ? '80' : proto === 'https' ? '443' : '';
  const isWeb = proto === 'http' || proto === 'https';

  return (
    <div className="drawer" onKeyDown={onKey} role="dialog" aria-label="Editor">
      <header>
        <div className="t">
          <b>{isNew ? (isFolder ? 'New folder' : 'New connection') : draft.name || 'Untitled'}</b>
          <span className="dim small">{(isFolder ? pathOf(snap, (draft as Folder).parent_id) : pathOf(snap, c.folder_id)) || 'Top level'}{readOnly ? ' · read-only' : ''}</span>
        </div>
        <button className="icon" aria-label="Close" onClick={() => void close()}><Icon name="close" /></button>
      </header>
      <div className="dtabs" role="tablist">
        {sections.map((s) => <button key={s} role="tab" aria-selected={section === s} onClick={() => setSection(s)}>{s}</button>)}
      </div>
      <div className="dbody">
        {section === 'General' && !isFolder && (<>
          <Field label="Name"><input type="text" autoFocus value={c.name} onChange={(e) => setDraft({ ...c, name: e.target.value })} /></Field>
          <Field label="Protocol">
            <select value={c.protocol} onChange={(e) => setDraft({ ...c, protocol: e.target.value as Protocol })}>
              {[['ssh', 'SSH'], ['rdp', 'RDP'], ['vnc', 'VNC'], ['http', 'HTTP'], ['https', 'HTTPS'], ['external', 'External tool']].map(([v, l]) => <option key={v} value={v}>{l}</option>)}
            </select>
          </Field>
          <Field label={isWeb ? 'Host or URL' : 'Host'}><input type="text" value={c.host} placeholder={isWeb ? 'intranet.example.com or https://…' : '10.0.0.5 or host.example.com'} onChange={(e) => setDraft({ ...c, host: e.target.value })} /></Field>
          <Field label="Port"><input type="number" min={1} max={65535} value={c.port ?? ''} placeholder={ph} onChange={(e) => setDraft({ ...c, port: e.target.value === '' ? null : Number(e.target.value) })} /></Field>
          <Field label="Tags"><input type="text" value={c.tags.join(', ')} onChange={(e) => setDraft({ ...c, tags: e.target.value.split(',').map((t) => t.trim()).filter(Boolean) })} /></Field>
          <Field label="Notes"><textarea className="mono" rows={4} value={c.notes} onChange={(e) => setDraft({ ...c, notes: e.target.value })} /></Field>
          <label className="check"><input type="checkbox" checked={c.options.pinned === true} onChange={(e) => { const n = { ...c.options }; if (e.target.checked) n.pinned = true; else delete n.pinned; setDraft({ ...c, options: n }); }} /> Pin to favorites</label>
          <label className="check"><input type="checkbox" checked={c.options.auto_open === true} onChange={(e) => { const n = { ...c.options }; if (e.target.checked) n.auto_open = true; else delete n.auto_open; setDraft({ ...c, options: n }); }} /> Open automatically when the workspace unlocks</label>
          <Field label="Folder">
            <select value={c.folder_id ?? ''} onChange={(e) => setDraft({ ...c, folder_id: e.target.value || null })}>
              {folderOptions.map((f) => <option key={f.id} value={f.id}>{f.label}</option>)}
            </select>
          </Field>
        </>)}
        {section === 'General' && isFolder && (<>
          <Field label="Folder name"><input type="text" autoFocus value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} /></Field>
          <Field label="Color" hint="Shown on the tabs of sessions from this folder">
            <div className="swatches">
              <button className="sw" aria-pressed={!(draft as Folder).color} title="None" style={{ background: 'var(--bg-3)' }} onClick={() => setDraft({ ...draft, color: null } as Folder)}>∅</button>
              {SWATCHES.map((s) => <button key={s} className="sw" aria-pressed={(draft as Folder).color === s} style={{ background: s }} aria-label={s} onClick={() => setDraft({ ...draft, color: s } as Folder)} />)}
              <input type="color" value={(draft as Folder).color ?? '#3b82f6'} onChange={(e) => setDraft({ ...draft, color: e.target.value } as Folder)} />
            </div>
          </Field>
          <div className="dim small">Settings on the other sections become defaults for everything in the folder.</div>
        </>)}
        {section === 'Credentials' && (<>
          {credRow('username', 'User name', false)}
          {credRow('password', 'Password', true)}
          {(isFolder || proto === 'rdp' || proto === 'external') && credRow('domain', 'Domain', false)}
          {(isFolder || proto === 'ssh') && credRow('private_key', 'Private key (inline, encrypted in the vault)', true, 4)}
          {(isFolder || proto === 'ssh') && credRow('key_passphrase', 'Key passphrase', true)}
          {fieldsFor('Credentials').map(optControl)}
          <div className="dim small">Empty fields inherit from the folder. Values are encrypted in the vault.</div>
        </>)}
        {section !== 'General' && section !== 'Credentials' && fieldsFor(section).map(optControl)}
      </div>
      <footer>
        {!isNew && <button className="danger" onClick={() => void del()}>Delete</button>}
        <span className="spacer" />
        <button onClick={() => void close()}>Cancel</button>
        {!isFolder && <button disabled={saving || readOnly} onClick={() => void save(true)}>Save and connect</button>}
        <button className="primary" disabled={saving || readOnly || (!isNew && !dirty)} onClick={() => void save(false)}>Save</button>
      </footer>
    </div>
  );
}

function pathOf(snap: Snapshot, id: string | null): string {
  const names: string[] = [];
  const seen = new Set<string>();
  let cur = id;
  while (cur && !seen.has(cur)) { seen.add(cur); const f = snap.folders.find((x) => x.id === cur); if (!f) break; names.unshift(f.name); cur = f.parent_id; }
  return names.join('/');
}

function resolveConflict(mine: any, theirs: any): Promise<any | null> {
  return new Promise((resolve) => openDialog('conflict', { mine, theirs, resolve }));
}

export function ConflictDialog({ mine, theirs, resolve }: { mine: any; theirs: any; resolve: (v: any | null) => void }) {
  const keys = Object.keys({ ...mine, ...theirs }).filter((k) => !['id', 'version', 'has_secret'].includes(k) && JSON.stringify(mine[k]) !== JSON.stringify(theirs[k]));
  const [pick, setPick] = useState<Record<string, 'mine' | 'theirs'>>(Object.fromEntries(keys.map((k) => [k, 'mine'])));
  const done = (v: any | null) => { closeDialog(); resolve(v); };
  const fmt = (v: unknown) => (typeof v === 'string' ? v : JSON.stringify(v));
  return (
    <div className="backdrop">
      <div className="modal wide" role="dialog" aria-label="This item was changed elsewhere">
        <header><h2>This item was changed elsewhere</h2></header>
        <div className="body">
          <table>
            <thead><tr><th>Field</th><th>Mine</th><th>Theirs</th></tr></thead>
            <tbody>{keys.map((k) => (
              <tr key={k}><td>{k}</td>
                {(['mine', 'theirs'] as const).map((side) => (
                  <td key={side}><label className="check"><input type="radio" name={k} checked={pick[k] === side} onChange={() => setPick({ ...pick, [k]: side })} /> <span className="mono small">{fmt((side === 'mine' ? mine : theirs)[k])}</span></label></td>
                ))}
              </tr>))}
            </tbody>
          </table>
        </div>
        <footer>
          <button onClick={() => done(null)}>Keep theirs</button>
          <button onClick={() => done({ ...mine, version: theirs.version })}>Keep mine</button>
          <button className="primary" onClick={() => done({ ...theirs, ...Object.fromEntries(keys.filter((k) => pick[k] === 'mine').map((k) => [k, mine[k]])), version: theirs.version })}>Save merged</button>
        </footer>
      </div>
    </div>
  );
}

