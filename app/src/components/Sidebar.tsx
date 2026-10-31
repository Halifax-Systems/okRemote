import { useEffect, useMemo, useRef, useState } from 'react';
import { useStore, getState, setState } from '../store';
import {
  closeWorkspace, copyItems, copySecretField, deleteItems, duplicateItem, editItem, folderPath, inheritedColor, moveItems,
  newConnection, newFolder, openConnection, openDialog, persistExpanded, rowKey, togglePin, toast,
} from '../actions';
import { Icon, protoColor, hint } from '../ui';
import type { Conn, Folder, Protocol } from '../types';
import { call } from '../api';

type Row =
  | { t: 'ws'; key: string; ws: string; depth: number }
  | { t: 'folder'; key: string; ws: string; depth: number; f: Folder; open: boolean; hasKids: boolean }
  | { t: 'conn'; key: string; ws: string; depth: number; c: Conn; right?: string };

const PROTOS: [Protocol, string][] = [['ssh', 'SSH'], ['rdp', 'RDP'], ['vnc', 'VNC'], ['https', 'HTTPS'], ['http', 'HTTP'], ['external', 'External tool']];
const byOrder = (a: { sort: number; name: string }, b: { sort: number; name: string }) => a.sort - b.sort || a.name.localeCompare(b.name);

function useRows(): { rows: Row[]; search: boolean } {
  const workspaces = useStore((s) => s.workspaces);
  const wsOrder = useStore((s) => s.wsOrder);
  const expanded = useStore((s) => s.expanded);
  const filter = useStore((s) => s.filter);
  const chips = useStore((s) => s.chips);
  const pinned = useStore((s) => s.settings.pinned_workspace);
  return useMemo(() => {
    const search = filter.trim() !== '' || chips.length > 0;
    const order = [...wsOrder].sort((a, b) => {
      const pa = workspaces[a]?.info.path === pinned ? 0 : 1, pb = workspaces[b]?.info.path === pinned ? 0 : 1;
      return pa - pb || (workspaces[a]?.info.name ?? '').localeCompare(workspaces[b]?.info.name ?? '');
    });
    const rows: Row[] = [];
    if (search) {
      const terms = filter.toLowerCase().split(/\s+/).filter(Boolean);
      const protoChips = chips.filter((c) => c.startsWith('p:')).map((c) => c.slice(2));
      const tagChips = chips.filter((c) => c.startsWith('t:')).map((c) => c.slice(2));
      const wsChips = chips.filter((c) => c.startsWith('w:')).map((c) => c.slice(2));
      const hits: Row[] = [];
      for (const ws of order) {
        const snap = workspaces[ws]?.snap;
        if (!snap || (wsChips.length && !wsChips.includes(ws))) continue;
        for (const c of snap.connections) {
          const path = folderPath(snap, c.folder_id);
          const text = [c.name, c.host, c.tags.join(' '), c.notes, path].join('\n').toLowerCase();
          if (!terms.every((t) => text.includes(t))) continue;
          if (protoChips.length && !protoChips.includes(c.protocol)) continue;
          if (!tagChips.every((t) => c.tags.includes(t))) continue;
          hits.push({ t: 'conn', key: rowKey(ws, 'conn', c.id), ws, depth: 0, c, right: `${path ? path + ' · ' : ''}${c.host}` });
        }
      }
      const first = terms[0];
      if (first) hits.sort((a, b) => Number((b as any).c.name.toLowerCase().startsWith(first)) - Number((a as any).c.name.toLowerCase().startsWith(first)));
      return { rows: hits.slice(0, 500), search };
    }
    for (const ws of order) {
      const k = rowKey(ws, 'ws', ws);
      rows.push({ t: 'ws', key: k, ws, depth: 0 });
      const snap = workspaces[ws]?.snap;
      if (!snap || expanded[k] === false) continue;
      const walk = (parent: string | null, depth: number) => {
        for (const f of snap.folders.filter((x) => x.parent_id === parent).sort(byOrder)) {
          const fk = rowKey(ws, 'folder', f.id);
          const kids = snap.folders.some((x) => x.parent_id === f.id) || snap.connections.some((x) => x.folder_id === f.id);
          const open = !!expanded[fk];
          rows.push({ t: 'folder', key: fk, ws, depth, f, open, hasKids: kids });
          if (open) walk(f.id, depth + 1);
        }
        for (const c of snap.connections.filter((x) => x.folder_id === parent).sort(byOrder)) rows.push({ t: 'conn', key: rowKey(ws, 'conn', c.id), ws, depth, c });
      };
      walk(null, 1);
    }
    return { rows, search };
  }, [workspaces, wsOrder, expanded, filter, chips, pinned]);
}

export function Sidebar() {
  const width = useStore((s) => s.settings.sidebar_width);
  return (
    <div className="sidebar" style={{ width }}>
      <FilterBox />
      <Favorites />
      <Tree />
      <div className="sidebar-actions">
        <button className="sm" onClick={() => newConnection()}><Icon name="plus" size={13} /> Connection</button>
        <button className="sm" onClick={() => void newFolder()}><Icon name="folder" size={13} /> Folder</button>
        <button className="sm" onClick={() => openDialog('vaults')}><Icon name="vault" size={13} /> Vault</button>
      </div>
    </div>
  );
}

function FilterBox() {
  const filter = useStore((s) => s.filter);
  const chips = useStore((s) => s.chips);
  const open = useStore((s) => s.chipsOpen);
  const workspaces = useStore((s) => s.workspaces);
  const tags = useMemo(() => {
    const set = new Set<string>();
    Object.values(workspaces).forEach((w) => w.snap?.connections.forEach((c) => c.tags.forEach((t) => set.add(t))));
    return [...set].sort().slice(0, 40);
  }, [workspaces]);
  const unlocked = Object.values(workspaces).filter((w) => w.snap);
  const toggle = (c: string) => setState((s) => ({ chips: s.chips.includes(c) ? s.chips.filter((x) => x !== c) : [...s.chips, c] }));
  const active = filter.trim() !== '' || chips.length > 0;
  return (
    <>
      <div className="filterbox">
        <Icon name="search" size={14} />
        <input id="filter-input" type="text" placeholder="Filter connections" value={filter}
          onChange={(e) => setState({ filter: e.target.value })}
          onKeyDown={(e) => {
            if (e.key === 'Escape') { setState({ filter: '' }); (e.target as HTMLInputElement).blur(); }
            if (e.key === 'ArrowDown') { e.preventDefault(); document.getElementById('tree')?.focus(); }
          }} />
        <button className={`icon ${open || active ? 'on' : ''}`} aria-label="Filters" title="Filters" onClick={() => setState({ chipsOpen: !open })}><Icon name="menu" size={14} /></button>
      </div>
      {open && (
        <div className="chiprow">
          {PROTOS.map(([p, l]) => <button key={p} className="chip sm" aria-pressed={chips.includes(`p:${p}`)} onClick={() => toggle(`p:${p}`)}>{l}</button>)}
          {tags.map((t) => <button key={t} className="chip sm" aria-pressed={chips.includes(`t:${t}`)} onClick={() => toggle(`t:${t}`)}>#{t}</button>)}
          {unlocked.length > 1 && unlocked.map((w) => <button key={w.info.id} className="chip sm" aria-pressed={chips.includes(`w:${w.info.id}`)} onClick={() => toggle(`w:${w.info.id}`)}>{w.info.name}</button>)}
          {active && <button className="chip sm" onClick={() => setState({ filter: '', chips: [] })}>Clear</button>}
        </div>
      )}
    </>
  );
}

function Favorites() {
  const workspaces = useStore((s) => s.workspaces);
  const favs = useMemo(() => Object.entries(workspaces).flatMap(([ws, w]) => (w.snap?.connections ?? []).filter((c) => c.options.pinned === true).map((c) => ({ ws, c }))), [workspaces]);
  if (!favs.length) return null;
  return (
    <div className="favs">
      {favs.map(({ ws, c }) => (
        <button key={c.id} title={`Connect to ${c.name}`} onClick={() => void openConnection(ws, c.id)}>
          <Icon name={c.protocol} size={14} style={{ color: protoColor[c.protocol] }} /> {c.name}
        </button>
      ))}
    </div>
  );
}

function Tree() {
  const { rows, search } = useRows();
  const selection = useStore((s) => s.selection);
  const focus = useStore((s) => s.focus);
  const workspaces = useStore((s) => s.workspaces);
  const rowH = useStore((s) => (s.settings.density === 'compact' ? 22 : 28));
  const ref = useRef<HTMLDivElement>(null);
  const [scroll, setScroll] = useState({ top: 0, h: 600 });
  const [drop, setDrop] = useState<{ key: string; pos: 'before' | 'into' | 'after' } | null>(null);
  const fontPx = parseFloat(document.documentElement.style.fontSize || '13') / 13;
  const rh = rowH * fontPx;

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const on = () => setScroll({ top: el.scrollTop, h: el.clientHeight });
    on();
    el.addEventListener('scroll', on);
    const ro = new ResizeObserver(on);
    ro.observe(el);
    return () => { el.removeEventListener('scroll', on); ro.disconnect(); };
  }, []);

  const first = Math.max(0, Math.floor(scroll.top / rh) - 12);
  const last = Math.min(rows.length, Math.ceil((scroll.top + scroll.h) / rh) + 12);
  const idx = (key: string | null) => (key ? rows.findIndex((r) => r.key === key) : -1);

  const select = (r: Row, _e: React.MouseEvent | null, additive = false, range = false) => {
    if (r.t === 'ws') { setState({ selection: [], focus: null }); return; }
    setState((s) => {
      if (range && s.anchor) {
        const a = idx(s.anchor), b = idx(r.key);
        if (a >= 0 && b >= 0) {
          const [lo, hi] = a < b ? [a, b] : [b, a];
          const sel = rows.slice(lo, hi + 1).filter((x) => x.t !== 'ws' && x.ws === r.ws).map((x) => x.key);
          return { selection: sel, focus: r.key };
        }
      }
      if (additive) return { selection: s.selection.includes(r.key) ? s.selection.filter((k) => k !== r.key) : [...s.selection, r.key], focus: r.key, anchor: r.key };
      return { selection: [r.key], focus: r.key, anchor: r.key };
    });
  };

  const toggleExpand = (r: Row) => {
    if (r.t === 'conn') return;
    setState((s) => ({ expanded: { ...s.expanded, [r.key]: r.t === 'ws' ? s.expanded[r.key] === false : !s.expanded[r.key] } }));
    persistExpanded();
  };

  const itemsOf = (keys: string[]) => keys.map((k) => { const [ws, kind, id] = k.split(':'); return { ws, kind: kind as 'conn' | 'folder', id }; }).filter((i) => i.kind === 'conn' || i.kind === 'folder');

  const openRow = (r: Row) => {
    if (r.t === 'conn') void openConnection(r.ws, r.c.id);
    else if (r.t === 'ws' && workspaces[r.ws]?.info.locked) openDialog('unlock', { ws: r.ws });
    else toggleExpand(r);
  };

  const connectFolder = (ws: string, f: Folder) => {
    const kids = workspaces[ws]?.snap?.connections.filter((c) => c.folder_id === f.id) ?? [];
    if (kids.length > 20) return toast('That folder has more than 20 connections; select the ones you need', 'error');
    kids.forEach((c) => void openConnection(ws, c.id));
  };

  const move = (key: string, dir: -1 | 1) => {
    const [ws, kind, id] = key.split(':');
    const snap = workspaces[ws]?.snap;
    if (!snap) return;
    const parent = kind === 'conn' ? snap.connections.find((c) => c.id === id)?.folder_id ?? null : snap.folders.find((f) => f.id === id)?.parent_id ?? null;
    const sibs = (kind === 'conn' ? snap.connections.filter((c) => c.folder_id === parent) : snap.folders.filter((f) => f.parent_id === parent)).sort(byOrder);
    const i = sibs.findIndex((x) => x.id === id), j = i + dir;
    if (j < 0 || j >= sibs.length) return;
    const k = kind === 'conn' ? 'connection' : 'folder';
    // dir -1: before sibs[j]; dir 1: before sibs[j+1] (or end)
    const before = dir === -1 ? sibs[j] : sibs[j + 1];
    void moveItems(ws, [{ kind: k, id }], parent, before ? { kind: k, id: before.id } : null);
  };

  const menuFor = (r: Row, x: number, y: number) => {
    if (r.t !== 'ws' && !selection.includes(r.key)) select(r, null);
    const sel = itemsOf(getState().selection);
    let items: any[] = [];
    if (r.t === 'ws') {
      const locked = workspaces[r.ws]?.info.locked;
      items = [
        locked ? { label: 'Unlock…', run: () => openDialog('unlock', { ws: r.ws }) } : { label: 'Lock', run: () => void call('ws_lock', { id: r.ws }).then(() => import('../actions').then((m) => m.refreshWorkspaces())) },
        { label: 'New connection', disabled: locked, run: () => newConnection({ ws: r.ws, folder: null }) },
        { label: 'New folder', disabled: locked, run: () => { setState({ focus: null }); void newFolder(); } },
        { sep: true },
        { label: 'Tunnel profiles…', disabled: locked, run: () => openDialog('tunnels', { ws: r.ws }) },
        { label: 'Terminal profiles…', disabled: locked, run: () => openDialog('profiles', { ws: r.ws }) },
        { label: 'Import into this workspace…', disabled: locked, run: () => openDialog('import', { ws: r.ws }) },
        { label: 'Audit log…', disabled: locked, run: () => openDialog('audit', { ws: r.ws }) },
        { sep: true },
        { label: 'Close workspace', run: () => void closeWorkspace(r.ws) },
      ];
    } else {
      const multi = sel.length > 1;
      const conns = sel.filter((s) => s.kind === 'conn');
      const common: any[] = [
        { label: 'Edit', hint: 'F2', run: () => editItem(r.ws, r.t === 'conn' ? 'conn' : 'folder', r.t === 'conn' ? r.c.id : (r as any).f.id) },
        { label: 'Duplicate', run: () => sel.forEach((s) => void duplicateItem(s.ws, s.kind, s.id)) },
        ...(conns.length > 1 ? [{ label: `Bulk edit ${conns.length} connections…`, run: () => openDialog('bulk', { ws: r.ws, ids: conns.map((c) => c.id) }) }] : []),
        { label: 'Move up', hint: hint('Alt+Up'), run: () => move(r.key, -1) },
        { label: 'Move down', hint: hint('Alt+Down'), run: () => move(r.key, 1) },
        { sep: true },
        { label: multi ? `Delete ${sel.length} items` : 'Delete', hint: 'Del', danger: true, run: () => void deleteItems(sel) },
      ];
      if (r.t === 'conn') {
        items = [
          { label: multi ? `Connect to ${conns.length}` : 'Connect', hint: 'Enter', run: () => conns.forEach((c) => void openConnection(c.ws, c.id)) },
          { label: r.c.options.pinned === true ? 'Unpin from favorites' : 'Pin to favorites', run: () => void togglePin(r.ws, r.c) },
          { sep: true }, ...common, { sep: true },
          { label: 'Copy user name', run: () => void copySecretField(r.ws, 'connection', r.c.id, 'username') },
          { label: 'Copy password', run: () => void copySecretField(r.ws, 'connection', r.c.id, 'password') },
          { label: 'Copy host', run: () => { void navigator.clipboard.writeText(r.c.host); } },
        ];
        // Delete must stay last
        const del = common[common.length - 1];
        items = items.filter((i) => i !== del);
        items.push({ sep: true }, del);
      } else {
        items = [
          { label: 'New connection here', run: () => newConnection({ ws: r.ws, folder: r.f.id }) },
          { label: 'New subfolder', run: () => { setState({ focus: r.key }); void newFolder(); } },
          { label: 'Connect to all in folder', run: () => connectFolder(r.ws, r.f) },
          { sep: true }, ...common,
        ];
      }
    }
    setState({ menu: { x, y, items } });
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (!rows.length) return;
    let i = idx(focus);
    const go = (n: number, extend = false) => {
      n = Math.max(0, Math.min(rows.length - 1, n));
      const r = rows[n];
      if (r.t === 'ws') { setState({ focus: r.key }); }
      else select(r, null, false, extend);
      if (extend) setState({ focus: r.key });
      ref.current?.scrollTo({ top: Math.max(0, n * rh - scroll.h / 2) });
    };
    const cur = rows[i];
    switch (e.key) {
      case 'ArrowDown': e.preventDefault(); go(i < 0 ? 0 : i + 1, e.shiftKey); break;
      case 'ArrowUp':
        e.preventDefault();
        if (i <= 0 && search) document.getElementById('filter-input')?.focus(); else if (e.altKey && cur && cur.t !== 'ws') move(cur.key, -1); else go(i - 1, e.shiftKey);
        break;
      case 'ArrowRight': e.preventDefault(); if (cur && cur.t !== 'conn' && !(cur as any).open && !(cur.t === 'ws' && getState().expanded[cur.key] !== false)) toggleExpand(cur); else go(i + 1); break;
      case 'ArrowLeft': e.preventDefault(); if (cur && cur.t === 'folder' && cur.open) toggleExpand(cur); break;
      case 'Home': e.preventDefault(); go(0); break;
      case 'End': e.preventDefault(); go(rows.length - 1); break;
      case 'Enter': if (cur) { e.preventDefault(); openRow(cur); } break;
      case 'Delete': e.preventDefault(); void deleteItems(itemsOf(getState().selection)); break;
      case 'F2': if (cur && cur.t !== 'ws') { e.preventDefault(); editItem(cur.ws, cur.t === 'conn' ? 'conn' : 'folder', cur.t === 'conn' ? cur.c.id : cur.f.id); } break;
    }
    if (e.altKey && e.key === 'ArrowDown' && cur && cur.t !== 'ws') { move(cur.key, 1); }
  };

  const onDragStart = (e: React.DragEvent, r: Row) => {
    if (r.t === 'ws') return e.preventDefault();
    const keys = selection.includes(r.key) ? selection : [r.key];
    e.dataTransfer.setData('application/x-okremote-items', JSON.stringify(itemsOf(keys)));
    e.dataTransfer.effectAllowed = 'copyMove';
  };

  const onDragOver = (e: React.DragEvent, r: Row) => {
    if (!e.dataTransfer.types.includes('application/x-okremote-items')) return;
    e.preventDefault();
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const y = (e.clientY - rect.top) / rect.height;
    const pos = r.t === 'ws' ? 'into' : r.t === 'folder' ? (y < 0.25 ? 'before' : y > 0.75 ? 'after' : 'into') : y < 0.5 ? 'before' : 'after';
    setDrop({ key: r.key, pos });
  };

  const onDrop = (e: React.DragEvent, r: Row) => {
    e.preventDefault();
    const d = drop;
    setDrop(null);
    const raw = e.dataTransfer.getData('application/x-okremote-items');
    if (!raw || !d) return;
    const items: { ws: string; kind: 'conn' | 'folder'; id: string }[] = JSON.parse(raw);
    if (items.some((i) => i.ws !== r.ws)) return toast('Moving between workspaces is not supported yet; export and import instead', 'error');
    const snap = workspaces[r.ws]?.snap;
    if (!snap) return;
    let parent: string | null;
    let before: { kind: string; id: string } | null = null;
    if (r.t === 'ws' || (r.t === 'folder' && d.pos === 'into')) parent = r.t === 'ws' ? null : r.f.id;
    else if (r.t === 'folder') {
      parent = r.f.parent_id;
      if (d.pos === 'before') before = { kind: 'folder', id: r.f.id };
      else {
        const next = snap.folders.filter((f) => f.parent_id === parent).sort(byOrder).find((f) => byOrder(f, r.f) > 0);
        before = next ? { kind: 'folder', id: next.id } : null;
      }
    } else {
      parent = r.c.folder_id;
      const sibs = snap.connections.filter((c) => c.folder_id === parent).sort(byOrder);
      if (d.pos === 'before') before = { kind: 'connection', id: r.c.id };
      else { const n = sibs[sibs.findIndex((c) => c.id === r.c.id) + 1]; before = n ? { kind: 'connection', id: n.id } : null; }
    }
    if (items.some((i) => i.kind === 'folder' && i.id === parent)) return toast('A folder cannot be moved into itself', 'error');
    const mapped = items.map((i) => ({ kind: i.kind === 'conn' ? 'connection' : 'folder', id: i.id }));
    if (e.ctrlKey || e.altKey) void copyItems(r.ws, mapped, parent); else void moveItems(r.ws, mapped, parent, before);
    if (parent) { setState((s) => ({ expanded: { ...s.expanded, [rowKey(r.ws, 'folder', parent!)]: true } })); persistExpanded(); }
  };

  const empty = rows.length === 0
    ? search ? 'No connections match.' : Object.keys(workspaces).length === 0 ? 'No workspace is open.' : null : null;
  const onlyEmptyWs = !search && rows.length > 0 && rows.every((r) => r.t === 'ws') && Object.values(workspaces).some((w) => w.snap);

  return (
    <div id="tree" className="tree" ref={ref} tabIndex={0} onKeyDown={onKeyDown}
      onFocus={() => { if (!getState().focus && rows.length) { const r = rows.find((x) => x.t !== 'ws'); if (r) select(r, null); } }}
      onDragLeave={() => setDrop(null)}>
      {empty && <div className="empty-text">{empty}</div>}
      {onlyEmptyWs && <div className="empty-text" style={{ marginTop: rh }}>Empty. Press {hint('Mod+N')} to add a connection.</div>}
      <div style={{ height: rows.length * rh, position: 'relative' }}>
        {rows.slice(first, last).map((r, k) => {
          const i = first + k;
          const ws = workspaces[r.ws];
          const sel = selection.includes(r.key);
          const cls = ['row', sel ? 'sel' : '', focus === r.key ? 'focus' : '', drop?.key === r.key ? `drop-${drop.pos}` : ''].join(' ');
          const snap = ws?.snap;
          return (
            <div key={r.key} className={cls} style={{ top: i * rh, paddingLeft: search ? 8 : 8 + r.depth * 6 * fontPx }}
              draggable={r.t !== 'ws'} onDragStart={(e) => onDragStart(e, r)} onDragOver={(e) => onDragOver(e, r)} onDrop={(e) => onDrop(e, r)}
              onClick={(e) => { if (r.t === 'ws') { select(r, e); } else select(r, e, e.ctrlKey || e.metaKey, e.shiftKey); }}
              onDoubleClick={() => openRow(r)}
              onContextMenu={(e) => { e.preventDefault(); menuFor(r, e.clientX, e.clientY); }}>
              {!search && (
                <span className={`twisty ${(r.t === 'ws' ? getState().expanded[r.key] !== false : (r as any).open) ? 'open' : ''} ${r.t === 'conn' || (r.t === 'folder' && !r.hasKids) || (r.t === 'ws' && ws?.info.locked) ? 'hidden' : ''}`}
                  onClick={(e) => { e.stopPropagation(); toggleExpand(r); }}><Icon name="chevron" size={14} /></span>
              )}
              {r.t === 'ws' && (<>
                <Icon name={ws?.info.locked ? 'lock' : 'vault'} /><b>{ws?.info.name}</b>
                {ws?.info.read_only && <span className="badge">read-only</span>}
                {ws?.info.locked && <button className="link" onClick={(e) => { e.stopPropagation(); openDialog('unlock', { ws: r.ws }); }}>Unlock</button>}
              </>)}
              {r.t === 'folder' && (<>
                <Icon name={r.open ? 'folderOpen' : 'folder'} />
                {snap && inheritedColor(snap, r.f.id) && <span className="cchip" style={{ background: inheritedColor(snap, r.f.id)! }} />}
                <span className="name">{r.f.name}</span>
              </>)}
              {r.t === 'conn' && (<>
                <Icon name={r.c.protocol} style={{ color: protoColor[r.c.protocol] }} />
                {snap && inheritedColor(snap, r.c.folder_id) && <span className="cchip" style={{ background: inheritedColor(snap, r.c.folder_id)! }} />}
                <span className="name" style={{ maxWidth: '70%' }}>{r.c.name}</span>
                {r.c.options.pinned === true && <Icon name="star" size={12} style={{ color: '#eab308' }} />}
                <span className="host">{r.right ?? r.c.host}</span>
              </>)}
            </div>
          );
        })}
      </div>
    </div>
  );
}

