import { useEffect, useRef, useState } from 'react';
import { useStore, setState } from '../store';
import {
  activateTab, closeTab, editItem, findLeaf, getController, getView, moveTab, newConnection, openConnection, openDialog, reconnectTab,
  restoreTabs, saveFontSize, setActivePane, setRatio, snapOf, splitPane, updateSettings, zoomTab,
} from '../actions';
import { Dot, Icon, hint, protoColor } from '../ui';
import type { PaneNode, Tab } from '../types';

export function Sessions() {
  const root = useStore((s) => s.root);
  return <div className="sessions"><PaneView node={root} /></div>;
}

function PaneView({ node }: { node: PaneNode }) {
  if (node.kind === 'leaf') return <Leaf id={node.id} />;
  return <SplitView node={node} />;
}

function SplitView({ node }: { node: Extract<PaneNode, { kind: 'split' }> }) {
  const ref = useRef<HTMLDivElement>(null);
  const drag = (e: React.MouseEvent) => {
    e.preventDefault();
    const el = ref.current!;
    const move = (ev: MouseEvent) => {
      const r = el.getBoundingClientRect();
      setRatio(node.id, node.dir === 'row' ? (ev.clientX - r.left) / r.width : (ev.clientY - r.top) / r.height);
    };
    const up = () => { window.removeEventListener('mousemove', move); window.removeEventListener('mouseup', up); };
    window.addEventListener('mousemove', move);
    window.addEventListener('mouseup', up);
  };
  return (
    <div ref={ref} className={`split ${node.dir}`}>
      <div style={{ flex: `${node.ratio} 1 0`, display: 'flex', minWidth: 0, minHeight: 0 }}><PaneView node={node.a} /></div>
      <div className="divider" onMouseDown={drag} />
      <div style={{ flex: `${1 - node.ratio} 1 0`, display: 'flex', minWidth: 0, minHeight: 0 }}><PaneView node={node.b} /></div>
    </div>
  );
}

function Leaf({ id }: { id: string }) {
  const leaf = useStore((s) => findLeaf(s.root, id));
  const tabs = useStore((s) => s.tabs);
  const active = useStore((s) => s.activePane === id);
  const [dropIdx, setDropIdx] = useState<number | null>(null);
  if (!leaf) return null;
  const list = leaf.tabs.map((t) => tabs[t]).filter(Boolean);
  const showMenu = (e: React.MouseEvent, t: Tab) => {
    e.preventDefault();
    const quick = !t.target.conn_id;
    setState({ menu: { x: e.clientX, y: e.clientY, items: [
      { label: 'Reconnect', disabled: t.state === 'connecting', run: () => reconnectTab(t.id) },
      { label: 'Duplicate', disabled: quick, run: () => void openConnection(t.target.ws!, t.target.conn_id!, id) },
      { sep: true },
      { label: 'Zoom in', hint: hint('Mod+='), run: () => zoomTab(t.id, 1) },
      { label: 'Zoom out', hint: hint('Mod+-'), run: () => zoomTab(t.id, -1) },
      { label: 'Reset zoom', hint: hint('Mod+0'), run: () => zoomTab(t.id, 'reset') },
      { label: 'Save font size for this connection', disabled: quick, run: () => void saveFontSize(t.id) },
      { sep: true },
      { label: 'Edit connection', disabled: quick, run: () => editItem(t.target.ws!, 'conn', t.target.conn_id!) },
      { sep: true },
      { label: 'Close', run: () => void closeTab(t.id) },
    ] } });
  };
  return (
    <div className={`pane ${active ? 'active' : ''}`} onMouseDown={() => setActivePane(id)}>
      <div className="tabbar"
        onDragOver={(e) => { if (e.dataTransfer.types.includes('application/x-okremote-tab')) { e.preventDefault(); if (e.target === e.currentTarget) setDropIdx(list.length); } }}
        onDragLeave={() => setDropIdx(null)}
        onDrop={(e) => {
          const t = e.dataTransfer.getData('application/x-okremote-tab');
          if (t) { e.preventDefault(); moveTab(t, id, dropIdx); }
          setDropIdx(null);
        }}>
        {list.map((t, i) => (
          <div key={t.id} className={`tab ${leaf.active === t.id ? 'active' : ''} ${dropIdx === i ? 'dropmark' : ''}`} style={{ borderTopColor: t.color ?? 'transparent' }}
            title={`${t.title}${t.route ? ` — via ${t.route}` : ''}${t.logFile ? `\nLogging to ${t.logFile}` : ''}`}
            draggable onDragStart={(e) => e.dataTransfer.setData('application/x-okremote-tab', t.id)}
            onDragOver={(e) => { if (e.dataTransfer.types.includes('application/x-okremote-tab')) { e.preventDefault(); e.stopPropagation(); setDropIdx(i); } }}
            onClick={() => activateTab(id, t.id)}
            onAuxClick={(e) => { if (e.button === 1) void closeTab(t.id); }}
            onContextMenu={(e) => showMenu(e, t)}>
            <Dot state={t.state} />
            <Icon name={t.protocol} size={13} style={{ color: protoColor[t.protocol] }} />
            <span className="title">{t.title}</span>
            {t.route && <span title={`Through ${t.route}`}><Icon name="tunnel" size={12} /></span>}
            <button className="icon" aria-label={`Close ${t.title}`} onClick={(e) => { e.stopPropagation(); void closeTab(t.id); }}><Icon name="close" size={12} /></button>
          </div>
        ))}
        <span className="spacer" />
        {list.length > 0 && (<>
          <button className="icon" title={`Split right (${hint('Mod+D')})`} aria-label="Split right" onClick={() => { setActivePane(id); splitPane('row'); }}><Icon name="split" /></button>
          <button className="icon" title={`Split down (${hint('Mod+Shift+D')})`} aria-label="Split down" onClick={() => { setActivePane(id); splitPane('column'); }}><Icon name="splitDown" /></button>
        </>)}
      </div>
      <div className="panebody">
        {list.length === 0 && <Welcome paneId={id} />}
        {list.map((t) => <TermHost key={t.id} tab={t} visible={leaf.active === t.id} />)}
      </div>
    </div>
  );
}

function TermHost({ tab, visible }: { tab: Tab; visible: boolean }) {
  const host = useRef<HTMLDivElement>(null);
  const search = useStore((s) => !!s.searchOpen[tab.id]);
  useEffect(() => {
    const c = getView(tab.id);
    if (host.current && c) c.mount(host.current);
  });
  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const ro = new ResizeObserver(() => getView(tab.id)?.refit());
    ro.observe(el);
    return () => ro.disconnect();
  }, [tab.id]);
  useEffect(() => { if (visible) { getView(tab.id)?.refit(); getView(tab.id)?.focus(); } }, [visible, tab.id]);
  const dead = tab.state === 'closed' || tab.state === 'error';
  return (
    <div style={{ position: 'absolute', inset: 0, visibility: visible ? 'visible' : 'hidden' }}>
      <div ref={host} className="term" data-tab={tab.id} />
      {tab.state === 'connecting' && <div className="center-note">Connecting…</div>}
      {search && tab.protocol === 'ssh' && <SearchBar tabId={tab.id} />}
      {dead && (
        <div className="overlay">
          <div className="box">
            <b>{tab.state === 'error' ? 'Connection failed' : 'Disconnected'}</b>
            {tab.error && <div className="danger-text">{tab.error}</div>}
            <div className="inline">
              <button className="primary" autoFocus onClick={() => reconnectTab(tab.id)}><Icon name="refresh" size={14} /> Reconnect</button>
              <button onClick={() => void closeTab(tab.id, true)}>Close tab</button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function SearchBar({ tabId }: { tabId: string }) {
  const [q, setQ] = useState('');
  const [re, setRe] = useState(false);
  const [cs, setCs] = useState(false);
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => ref.current?.focus(), []);
  const find = (dir: 1 | -1, text = q) => {
    const c = getController(tabId);
    if (!c || !text) return;
    if (re) { try { new RegExp(text); } catch { return; } }
    const opts = { regex: re, caseSensitive: cs, decorations: { matchBackground: '#7c6f2a', activeMatchBackground: '#c9a227', matchOverviewRuler: '#c9a227', activeMatchColorOverviewRuler: '#c9a227' } };
    if (dir === 1) c.search.findNext(text, opts); else c.search.findPrevious(text, opts);
  };
  const close = () => { setState((s) => ({ searchOpen: { ...s.searchOpen, [tabId]: false } })); getController(tabId)?.search.clearDecorations(); getController(tabId)?.focus(); };
  return (
    <div className="searchbar" onKeyDown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); close(); } }}>
      <input ref={ref} type="text" placeholder="Find in terminal" value={q}
        onChange={(e) => { setQ(e.target.value); find(1, e.target.value); }}
        onKeyDown={(e) => { if (e.key === 'Enter') find(e.shiftKey ? -1 : 1); }} />
      <button className="chip sm" aria-pressed={re} onClick={() => setRe(!re)}>.*</button>
      <button className="chip sm" aria-pressed={cs} onClick={() => setCs(!cs)}>Aa</button>
      <button className="icon" aria-label="Previous match" onClick={() => find(-1)} style={{ transform: 'rotate(-90deg)' }}><Icon name="chevron" size={14} /></button>
      <button className="icon" aria-label="Next match" onClick={() => find(1)} style={{ transform: 'rotate(90deg)' }}><Icon name="chevron" size={14} /></button>
      <button className="icon" aria-label="Close search" onClick={close}><Icon name="close" size={14} /></button>
    </div>
  );
}

function Welcome({ paneId }: { paneId: string }) {
  const workspaces = useStore((s) => s.workspaces);
  const recents = useStore((s) => s.settings.recents);
  const lastTabs = useStore((s) => s.settings.last_tabs);
  const restore = useStore((s) => s.settings.restore_tabs);
  const anyTab = useStore((s) => Object.keys(s.tabs).length > 0);
  const firstRun = useStore((s) => s.firstRun);
  if (firstRun) return <FirstRun />;
  const favs = Object.entries(workspaces).flatMap(([ws, w]) => (w.snap?.connections ?? []).filter((c) => c.options.pinned === true).map((c) => ({ ws, c })));
  const rec = recents.map((r) => ({ ws: r.ws, c: snapOf(r.ws)?.connections.find((c) => c.id === r.id) })).filter((r) => r.c).slice(0, 10) as { ws: string; c: any }[];
  const item = (ws: string, c: any) => (
    <button key={`${ws}${c.id}`} onClick={() => void openConnection(ws, c.id, paneId)}>
      <Icon name={c.protocol} size={14} style={{ color: protoColor[c.protocol as keyof typeof protoColor] }} /> {c.name}
      <span className="host">{c.protocol.toUpperCase()} · {c.host}</span>
    </button>
  );
  return (
    <div className="welcome">
      <h1>okRemote</h1>
      <div className="dim2">Pick a connection in the tree, or press {hint('Mod+L')} to connect to a host by address.</div>
      {!anyTab && restore === 'ask' && lastTabs.length > 0 && (
        <div className="restore">
          <span>{lastTabs.length} tab{lastTabs.length === 1 ? ' was' : 's were'} open last time.</span>
          <span className="spacer" />
          <button className="primary sm" onClick={() => void restoreTabs()}>Restore tabs</button>
          <button className="sm" onClick={() => updateSettings({ last_tabs: [] })}>Dismiss</button>
        </div>
      )}
      <div className="cols">
        <div><h3>Favorites</h3>{favs.length ? favs.map((f) => item(f.ws, f.c)) : <div className="dim small">Pin a connection from its context menu to see it here.</div>}</div>
        <div><h3>Recent</h3>{rec.length ? rec.map((f) => item(f.ws, f.c)) : <div className="dim small">Connections you open appear here.</div>}</div>
      </div>
      <div className="inline">
        <button onClick={() => newConnection()}><Icon name="plus" size={14} /> New connection</button>
        <button onClick={() => openDialog('import', {})}><Icon name="importIcon" size={14} /> Import</button>
        <button onClick={() => setState({ palette: true })}><Icon name="search" size={14} /> Command palette <span className="dim small">{hint('Mod+K')}</span></button>
      </div>
    </div>
  );
}

export function FirstRun() {
  const btn = (icon: string, title: string, line: string, run?: () => void, primary = false, disabled = false, tip?: string) => (
    <button className={`bigbtn ${primary ? 'primary' : ''}`} disabled={disabled} title={tip} onClick={run}><Icon name={icon} size={22} /><span><b>{title}</b><small>{line}</small></span></button>
  );
  return (
    <div className="firstrun">
      <div className="card">
        <h1 style={{ margin: 0 }}>Welcome to okRemote</h1>
        <div className="dim2">Your connections live in a workspace. Start with a personal vault file: a single encrypted file you can keep in a synced folder or on a USB drive.</div>
        {btn('vault', 'Create a vault file', 'Set a passphrase; the app opens with an empty tree.', () => openDialog('vaults', { mode: 'create' }), true)}
        {btn('folderOpen', 'Open an existing vault', 'A .okvault file from another machine or a backup.', () => openDialog('vaults', { mode: 'open' }))}
        {btn('importIcon', 'Import an existing connection file', 'Create a vault, then import your connection file into it.', () => { openDialog('vaults', { mode: 'create' }); import('../actions').then((m) => m.toast('Create a vault first, then use Import to bring in your existing connection file', 'info', 8000)); })}
        {btn('tunnel', 'Connect to a team database', 'PostgreSQL workspaces arrive in a later release.', undefined, false, true, 'Team databases arrive in a later release')}
      </div>
    </div>
  );
}

