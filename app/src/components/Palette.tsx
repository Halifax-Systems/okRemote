import { useEffect, useMemo, useRef, useState } from 'react';
import { useStore, setState } from '../store';
import {
  bindingFor, closeActiveTab, currentThemeIsDark, folderPath, newConnection, newFolder, openConnection, openDialog, restoreTabs, runAction,
  splitPane, updateSettings,
} from '../actions';
import { Icon, hint, protoColor } from '../ui';

interface Opt { icon: string; label: string; detail?: string; hint?: string; score: number; run: () => void; color?: string }

const score = (text: string, q: string) => {
  const t = text.toLowerCase();
  if (!q) return 1;
  if (t.startsWith(q)) return 3;
  if (t.includes(q)) return 2;
  let i = 0;
  for (const ch of t) if (ch === q[i]) i++;
  return i === q.length ? 1 : 0;
};

export function Palette() {
  const workspaces = useStore((s) => s.workspaces);
  const [q, setQ] = useState('');
  const [hl, setHl] = useState(0);
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => ref.current?.focus(), []);
  const close = () => setState({ palette: false });
  const after = (f: () => void) => () => { close(); setTimeout(f, 0); };
  const b = (a: string) => { const k = bindingFor(a); return k ? hint(k) : undefined; };

  const commands: Omit<Opt, 'score'>[] = useMemo(() => [
    { icon: 'plus', label: 'New connection', hint: b('newConnection'), run: after(() => newConnection()) },
    { icon: 'folder', label: 'New folder', hint: b('newFolder'), run: after(() => void newFolder()) },
    { icon: 'terminal', label: 'Quick connect', hint: b('quickConnect'), run: after(() => runAction('quickConnect')) },
    { icon: 'search', label: 'Filter the tree', hint: b('filterTree'), run: after(() => runAction('filterTree')) },
    { icon: 'lock', label: 'Lock workspaces', hint: b('lockAll'), run: after(() => runAction('lockAll')) },
    { icon: 'vault', label: 'Open or create a vault', run: after(() => openDialog('vaults')) },
    { icon: 'importIcon', label: 'Import connections', run: after(() => openDialog('import', {})) },
    { icon: 'tunnel', label: 'Tunnels', run: after(() => openDialog('tunnels', {})) },
    { icon: 'terminal', label: 'Terminal profiles', run: after(() => openDialog('profiles', {})) },
    { icon: 'key', label: 'Host keys', run: after(() => openDialog('hostkeys', {})) },
    { icon: 'history', label: 'Audit log', run: after(() => openDialog('audit', {})) },
    { icon: 'gear', label: 'Settings', run: after(() => openDialog('settings')) },
    { icon: 'split', label: 'Split pane right', hint: b('splitRight'), run: after(() => splitPane('row')) },
    { icon: 'splitDown', label: 'Split pane down', hint: b('splitDown'), run: after(() => splitPane('column')) },
    { icon: 'close', label: 'Close tab', hint: b('closeTab'), run: after(closeActiveTab) },
    { icon: 'sidebar', label: 'Toggle sidebar', hint: b('toggleSidebar'), run: after(() => runAction('toggleSidebar')) },
    { icon: 'history', label: 'Restore tabs from last session', run: after(() => void restoreTabs()) },
    { icon: 'gear', label: 'Switch light / dark theme', run: after(() => updateSettings({ theme: currentThemeIsDark() ? 'light' : 'dark' })) },
  ], []); // eslint-disable-line react-hooks/exhaustive-deps

  const query = q.trim().toLowerCase();
  const options = useMemo<Opt[]>(() => {
    const conns: Opt[] = [];
    for (const [ws, w] of Object.entries(workspaces)) {
      if (!w.snap) continue;
      for (const c of w.snap.connections) {
        const path = folderPath(w.snap, c.folder_id);
        const extra = `${c.host} ${path} ${c.tags.join(' ')}`;
        const sc = Math.max(2 * score(c.name, query), score(extra, query));
        if (sc > 0) conns.push({ icon: c.protocol, color: protoColor[c.protocol], label: c.name, detail: `${c.protocol.toUpperCase()} · ${c.host}${path ? ` · ${path}` : ''}`, score: sc, run: after(() => void openConnection(ws, c.id)) });
      }
    }
    const cmds = commands.map((c) => ({ ...c, score: score(c.label, query) })).filter((c) => c.score > 0);
    if (query) return [...conns.sort((a, b) => b.score - a.score).slice(0, 40), ...cmds.sort((a, b) => b.score - a.score)];
    return [...cmds, ...conns.slice(0, 30)];
  }, [workspaces, query, commands]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => setHl(0), [q]);
  useEffect(() => { document.querySelector('.palette li.hl')?.scrollIntoView({ block: 'nearest' }); }, [hl]);

  return (
    <div className="backdrop" style={{ background: 'transparent' }} onMouseDown={(e) => { if (e.target === e.currentTarget) close(); }}>
      <div className="palette" onMouseDown={(e) => e.stopPropagation()}>
        <input ref={ref} type="text" role="combobox" aria-expanded aria-controls="palette-list" placeholder="Type a connection or command…" value={q}
          onChange={(e) => setQ(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Escape') close();
            else if (e.key === 'ArrowDown') { e.preventDefault(); setHl((h) => Math.min(options.length - 1, h + 1)); }
            else if (e.key === 'ArrowUp') { e.preventDefault(); setHl((h) => Math.max(0, h - 1)); }
            else if (e.key === 'Enter') { e.preventDefault(); options[hl]?.run(); }
          }} />
        <ul id="palette-list" role="listbox">
          {options.length === 0 && <li style={{ cursor: 'default' }} className="dim">Nothing matches.</li>}
          {options.map((o, i) => (
            <li key={`${o.label}${i}`} role="option" aria-selected={i === hl} className={i === hl ? 'hl' : ''} onMouseEnter={() => setHl(i)} onClick={o.run}>
              <Icon name={o.icon} style={{ color: o.color }} />
              <div className="l">{o.label}{o.detail && <div className="d">{o.detail}</div>}</div>
              {o.hint && <kbd>{o.hint}</kbd>}
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
