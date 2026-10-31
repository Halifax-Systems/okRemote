import { useState } from 'react';
import { useStore, setState } from '../store';
import { call } from '../api';
import { lockAll, openDialog, openQuick, insertionPoint, loadSnapshot, editItem, toast, toastError, updateSettings, rowKey } from '../actions';
import { Icon, hint } from '../ui';

export function TopBar() {
  const [text, setText] = useState('');
  const collapsed = useStore((s) => s.settings.sidebar_collapsed);
  const anyUnlocked = useStore((s) => s.wsOrder.some((id) => s.workspaces[id]?.snap));
  const blank = text.trim() === '';
  const connect = async () => { if (blank) return; if (await openQuick(text)) setText(''); };
  const save = async () => {
    const at = insertionPoint();
    if (!at) return toast('Open a workspace to save connections', 'error');
    try {
      const t = await call<any>('quick_parse', { input: text });
      const web = t.protocol === 'http' || t.protocol === 'https';
      const conn = await call<any>('conn_save', { ws: at.ws, conn: {
        id: crypto.randomUUID(), folder_id: at.folder, name: t.name, protocol: t.protocol, host: web ? t.url : t.host, port: web ? null : t.port,
        options: {}, tags: [], notes: '', icon: null, has_secret: false, sort: 9999, version: 0 } });
      if (t.user) await call('secret_patch', { ws: at.ws, owner: 'connection', id: conn.id, patch: { username: t.user } });
      await loadSnapshot(at.ws);
      setText('');
      const key = rowKey(at.ws, 'conn', conn.id);
      setState({ selection: [key], focus: key, anchor: key });
      editItem(at.ws, 'conn', conn.id);
    } catch (e) { toastError(e); }
  };
  const menu = (e: React.MouseEvent) => {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    setState({ menu: { x: r.left, y: r.bottom + 2, items: [
      { label: 'Open or create a vault…', run: () => openDialog('vaults') },
      { label: 'Import…', run: () => openDialog('import', {}) },
      { sep: true },
      { label: 'Tunnels…', run: () => openDialog('tunnels', {}) },
      { label: 'Terminal profiles…', run: () => openDialog('profiles', {}) },
      { label: 'Host keys…', run: () => openDialog('hostkeys', {}) },
      { label: 'Audit log…', run: () => openDialog('audit', {}) },
      { sep: true },
      { label: 'Settings…', run: () => openDialog('settings') },
      { label: 'About okRemote', run: () => openDialog('about') },
    ] } });
  };
  return (
    <div className="topbar">
      <button className="icon" title={`Toggle sidebar (${hint('Mod+B')})`} aria-label={collapsed ? 'Show sidebar' : 'Hide sidebar'} onClick={() => updateSettings({ sidebar_collapsed: !collapsed })}><Icon name="sidebar" /></button>
      <div className="quick">
        <Icon name="terminal" size={15} />
        <input id="quick-input" type="text" placeholder="Quick connect: user@host:port, rdp://host, https://…" value={text} spellCheck={false}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') void connect(); if (e.key === 'Escape') (e.target as HTMLInputElement).blur(); }} />
        <button className="primary sm" disabled={blank} onClick={() => void connect()}>Connect</button>
        <button className="sm" disabled={blank} onClick={() => void save()}>Save</button>
      </div>
      <span className="spacer" />
      <button className="sm" onClick={() => setState({ palette: true })}><Icon name="search" size={14} /> <span className="dim small">{hint('Mod+K')}</span></button>
      <button className="icon" aria-label="Lock all workspaces" title="Lock all workspaces" disabled={!anyUnlocked} onClick={() => void lockAll()}><Icon name="lock" /></button>
      <button className="icon" aria-label="Menu" onClick={menu}><Icon name="menu" /></button>
    </div>
  );
}

export function StatusBar() {
  const workspaces = useStore((s) => s.workspaces);
  const order = useStore((s) => s.wsOrder);
  const tab = useStore((s) => s.tabs);
  const active = useStore((s) => { const find = (n: any): any => (n.kind === 'leaf' ? (n.id === s.activePane ? n : null) : find(n.a) ?? find(n.b)); return find(s.root)?.active ?? null; });
  const t = active ? tab[active] : null;
  const tun = useStore((x) => x.tunnels) ?? { forwards: [], transports: [] };
  const bad = tun.transports.find((x: any) => x.health === 'failed' || x.health === 'reconnecting');
  const locked = order.filter((id) => workspaces[id]?.info.locked).length;
  const allLocked = order.length > 0 && locked === order.length;
  const title = order.map((id) => `${workspaces[id].info.name}: ${workspaces[id].info.locked ? 'locked' : 'unlocked'}`).join('\n');
  return (
    <div className="statusbar">
      <span title={title} style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
        <Icon name={allLocked ? 'lock' : 'vault'} size={14} />
        {order.length} workspace(s){locked > 0 && `, ${locked} locked`}
      </span>
      <button onClick={() => openDialog('tunnels', {})}><Icon name="tunnel" size={14} />{(bad || tun.forwards.length > 0) && <span className={`dot ${bad ? bad.health : 'up'}`} />} {tun.forwards.length} forward(s){bad && ` · ${bad.label} ${bad.health}`}</button>
      <span className="spacer" />
      {t && (
        <span>
          {t.protocol.toUpperCase()} · {t.fixedCols && t.fixedRows ? `Fixed ${t.cols}x${t.rows}` : `Auto-fit ${t.cols}x${t.rows}`}
          {t.zoom !== 0 && ` · zoom ${t.zoom > 0 ? '+' : ''}${t.zoom}`}{t.route && ` · via ${t.route}`}{t.logFile && ' · logging'}
        </span>
      )}
    </div>
  );
}

