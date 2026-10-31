import { writeText, readText } from '@tauri-apps/plugin-clipboard-manager';
import { open as pickOpen, save as pickSave } from '@tauri-apps/plugin-dialog';
import { call, Channel, errText, isApiError, listen } from './api';
import { getState, setState, rootLeaf, type State } from './store';
import { TermController } from './terminal';
import { RdpController, isNormalClose } from './rdp';
import type { Conn, Folder, PaneNode, PromptMsg, Settings, Snapshot, Tab, Target, WsInfo } from './types';

// ------------------------------------------------------------------ toasts, dialogs

let toastSeq = 1;
export function toast(text: string, kind: 'info' | 'success' | 'error' = 'info', ms = 5000) {
  const id = toastSeq++;
  setState((s) => ({ toasts: [...s.toasts, { id, kind, text }] }));
  window.setTimeout(() => dismissToast(id), ms);
}
export const dismissToast = (id: number) => setState((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
export const toastError = (e: unknown, ms = 8000) => toast(errText(e), 'error', ms);

export function openDialog(name: string, props?: any) { setState({ dialog: { name, props }, menu: null }); }
export function closeDialog() { setState({ dialog: null }); }

export function confirmDialog(o: { title: string; body?: string; ok?: string; danger?: boolean; checkbox?: string }): Promise<{ ok: boolean; checked: boolean }> {
  return new Promise((resolve) => openDialog('confirm', { ...o, resolve }));
}
export function inputDialog(o: { title: string; label: string; initial?: string }): Promise<string | null> {
  return new Promise((resolve) => openDialog('input', { ...o, resolve }));
}

// ------------------------------------------------------------------ settings

let saveTimer: number | undefined;
export function updateSettings(patch: Partial<Settings> | ((s: Settings) => Partial<Settings>)) {
  setState((s) => {
    const p = typeof patch === 'function' ? patch(s.settings) : patch;
    return { settings: { ...s.settings, ...p } };
  });
  window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => { void call('settings_save', { settings: getState().settings }).catch(() => {}); }, 400);
  applyTheme();
  if ((patch as any).terminal || typeof patch === 'function') applyTerminalSettings();
}

export function applyTheme() {
  const { settings } = getState();
  const dark = settings.theme === 'dark' || (settings.theme === 'system' && matchMedia('(prefers-color-scheme: dark)').matches);
  const root = document.documentElement;
  root.dataset.theme = dark ? 'dark' : 'light';
  root.dataset.density = settings.density;
  root.style.fontSize = `${13 * Math.min(2, Math.max(0.8, settings.ui_scale))}px`;
  const a = settings.accent;
  root.style.setProperty('--accent', a);
  const m = /^#?([0-9a-f]{6})$/i.exec(a);
  if (m) {
    const n = parseInt(m[1], 16);
    const lum = (0.299 * (n >> 16) + 0.587 * ((n >> 8) & 255) + 0.114 * (n & 255)) / 255;
    root.style.setProperty('--on-accent', lum > 0.6 ? '#1d2a25' : '#ffffff');
  }
}

export function currentThemeIsDark() { return document.documentElement.dataset.theme === 'dark'; }

// ------------------------------------------------------------------ workspaces

export function snapOf(ws: string): Snapshot | null { return getState().workspaces[ws]?.snap ?? null; }

export async function refreshWorkspaces() {
  const infos = await call<WsInfo[]>('ws_list');
  const prev = getState().workspaces;
  const next: State['workspaces'] = {};
  for (const info of infos) next[info.id] = { info, snap: info.locked ? null : prev[info.id]?.snap ?? null };
  setState({ workspaces: next, wsOrder: infos.map((i) => i.id) });
  await Promise.all(infos.filter((i) => !i.locked).map((i) => loadSnapshot(i.id)));
  setState((s) => ({ firstRun: s.wsOrder.length === 0 }));
}

export async function loadSnapshot(ws: string) {
  try {
    const snap = await call<Snapshot>('ws_snapshot', { id: ws });
    setState((s) => (s.workspaces[ws] ? { workspaces: { ...s.workspaces, [ws]: { ...s.workspaces[ws], snap } } } : {}));
  } catch (e) {
    if (isApiError(e) && e.code === 'locked') await refreshWorkspaces(); else toastError(e);
  }
}

export async function openWorkspacePath(path: string, passphrase?: string, remember?: boolean): Promise<WsInfo> {
  const info = await call<WsInfo>('ws_open', { path, passphrase: passphrase ?? null, remember: remember ?? null });
  await refreshWorkspaces();
  const s = getState().settings;
  updateSettings({ recent_workspaces: [path, ...s.recent_workspaces.filter((p) => p !== path)].slice(0, 12) });
  saveOpenWorkspaces();
  if (!info.locked) await onUnlocked(info.id);
  return info;
}

export function saveOpenWorkspaces() {
  const paths = getState().wsOrder.map((id) => getState().workspaces[id]?.info.path).filter(Boolean) as string[];
  updateSettings({ last_open_workspaces: paths.slice(0, 12) });
}

export async function unlockWorkspace(id: string, passphrase: string, remember: boolean) {
  await call('ws_unlock', { id, passphrase, remember });
  await refreshWorkspaces();
  await onUnlocked(id);
}

async function onUnlocked(id: string) {
  const snap = snapOf(id);
  if (!snap) return;
  const open = Object.values(getState().tabs);
  for (const c of snap.connections) {
    if (c.options.auto_open === true && !open.some((t) => t.target.ws === id && t.target.conn_id === c.id)) void openConnection(id, c.id);
  }
}

export async function lockAll(announce = true) {
  await call('ws_lock_all');
  setState({ editor: null });
  await refreshWorkspaces();
  if (announce) toast('All workspaces locked');
}

export async function closeWorkspace(id: string) {
  await call('ws_close', { id });
  await refreshWorkspaces();
  saveOpenWorkspaces();
}

export async function createWorkspace(path: string, name: string, passphrase: string, remember: boolean) {
  const info = await call<WsInfo>('ws_create', { path, name, passphrase, remember });
  await refreshWorkspaces();
  updateSettings({ recent_workspaces: [path, ...getState().settings.recent_workspaces.filter((p) => p !== path)].slice(0, 12) });
  saveOpenWorkspaces();
  toast(`Created ${info.name}`, 'success');
  return info;
}

export async function chooseVaultFile() {
  const p = await pickOpen({ multiple: false, filters: [{ name: 'okRemote vault', extensions: ['okvault'] }] });
  if (typeof p === 'string') await openVaultFromPath(p);
}

export async function openVaultFromPath(path: string) {
  try {
    const info = await openWorkspacePath(path);
    if (info.locked) openDialog('unlock', { ws: info.id });
    else closeDialog();
  } catch (e) { toast(`Could not open ${path}: ${errText(e)}`, 'error', 8000); }
}

export async function pickSavePath(defaultPath: string, name: string, ext: string) {
  const p = await pickSave({ defaultPath, filters: [{ name, extensions: [ext] }] });
  return p ?? null;
}

// ------------------------------------------------------------------ tree helpers

export const rowKey = (ws: string, kind: string, id: string) => `${ws}:${kind}:${id}`;

export function insertionPoint(): { ws: string; folder: string | null } | null {
  const s = getState();
  const unlocked = s.wsOrder.filter((id) => s.workspaces[id]?.snap);
  if (!unlocked.length) return null;
  if (s.focus) {
    const [ws, kind, id] = s.focus.split(':');
    const snap = snapOf(ws);
    if (snap && kind === 'folder') return { ws, folder: id };
    if (snap && kind === 'conn') return { ws, folder: snap.connections.find((c) => c.id === id)?.folder_id ?? null };
  }
  return { ws: unlocked[0], folder: null };
}

export function folderPath(snap: Snapshot, folderId: string | null): string {
  const names: string[] = [];
  const seen = new Set<string>();
  let cur = folderId;
  while (cur && !seen.has(cur)) {
    seen.add(cur);
    const f = snap.folders.find((x) => x.id === cur);
    if (!f) break;
    names.unshift(f.name);
    cur = f.parent_id;
  }
  return names.join('/');
}

export function inheritedColor(snap: Snapshot, folderId: string | null): string | null {
  let cur = folderId;
  const seen = new Set<string>();
  while (cur && !seen.has(cur)) {
    seen.add(cur);
    const f = snap.folders.find((x) => x.id === cur);
    if (!f) break;
    if (f.color) return f.color;
    cur = f.parent_id;
  }
  return null;
}

export function resolveOption(snap: Snapshot, c: { folder_id: string | null; options: Record<string, unknown> }, key: string): unknown {
  if (key in c.options && c.options[key] !== null) return c.options[key];
  let cur = c.folder_id;
  const seen = new Set<string>();
  while (cur && !seen.has(cur)) {
    seen.add(cur);
    const f = snap.folders.find((x) => x.id === cur);
    if (!f) break;
    if (key in f.defaults && f.defaults[key] !== null) return f.defaults[key];
    cur = f.parent_id;
  }
  return undefined;
}

// ------------------------------------------------------------------ record actions

export async function newFolder() {
  const at = insertionPoint();
  if (!at) return toast('Open or unlock a workspace first', 'error');
  const name = await inputDialog({ title: 'New folder', label: 'Folder name', initial: 'New folder' });
  if (!name?.trim()) return;
  try {
    const f = await call<Folder>('folder_save', { ws: at.ws, folder: { id: crypto.randomUUID(), parent_id: at.folder, name: name.trim(), color: null, icon: null, defaults: {}, has_secret: false, sort: 9999, version: 0 } });
    await loadSnapshot(at.ws);
    const key = rowKey(at.ws, 'folder', f.id);
    setState((s) => ({ expanded: at.folder ? { ...s.expanded, [rowKey(at.ws, 'folder', at.folder!)]: true } : s.expanded, selection: [key], focus: key, anchor: key }));
    persistExpanded();
  } catch (e) { toastError(e); }
}

export function newConnection(folderOverride?: { ws: string; folder: string | null }) {
  const at = folderOverride ?? insertionPoint();
  if (!at) return toast('Open or unlock a workspace first', 'error');
  const blank: Conn = { id: crypto.randomUUID(), folder_id: at.folder, name: '', protocol: 'ssh', host: '', port: null, options: {}, tags: [], notes: '', icon: null, has_secret: false, sort: 9999, version: 0 };
  setState({ editor: { ws: at.ws, kind: 'conn', id: blank.id, blank }, menu: null });
}

export function editItem(ws: string, kind: 'conn' | 'folder', id: string) { setState({ editor: { ws, kind, id }, menu: null }); }

export async function deleteItems(items: { ws: string; kind: 'conn' | 'folder'; id: string }[]) {
  if (!items.length) return;
  const names = items.map((i) => {
    const snap = snapOf(i.ws);
    return i.kind === 'conn' ? snap?.connections.find((c) => c.id === i.id)?.name : snap?.folders.find((f) => f.id === i.id)?.name;
  }).filter(Boolean) as string[];
  const shown = names.slice(0, 8).join('\n') + (names.length > 8 ? `\n…and ${names.length - 8} more` : '');
  const hasFolder = items.some((i) => i.kind === 'folder');
  const r = await confirmDialog({
    title: items.length === 1 ? 'Delete item?' : `Delete ${items.length} items?`,
    body: shown + (hasFolder ? '\n\nDeleting a folder deletes everything inside it.' : ''), ok: 'Delete', danger: true,
  });
  if (!r.ok) return;
  try {
    for (const i of items) await call(i.kind === 'conn' ? 'conn_delete' : 'folder_delete', { ws: i.ws, id: i.id }).catch((e) => { if (!(isApiError(e) && e.code === 'not_found')) throw e; });
    for (const ws of new Set(items.map((i) => i.ws))) await loadSnapshot(ws);
    setState({ selection: [], focus: null, editor: null });
  } catch (e) { toastError(e); }
}

export async function duplicateItem(ws: string, kind: 'conn' | 'folder', id: string) {
  try {
    await call(kind === 'conn' ? 'conn_duplicate' : 'folder_duplicate', { ws, id });
    await loadSnapshot(ws);
  } catch (e) { toastError(e); }
}

export async function togglePin(ws: string, c: Conn) {
  try {
    const options = { ...c.options };
    if (options.pinned === true) delete options.pinned; else options.pinned = true;
    await call('conn_save', { ws, conn: { ...c, options } });
    await loadSnapshot(ws);
  } catch (e) { toastError(e); }
}

export async function copySecretField(ws: string, owner: 'connection' | 'folder', id: string, field: string) {
  try {
    const secs = await call<number>('secret_copy', { ws, owner, id, field });
    toast(secs ? `Copied; the clipboard clears in ${secs} s` : 'Copied', 'success', 3000);
  } catch (e) { toastError(e); }
}

export async function moveItems(ws: string, items: { kind: string; id: string }[], parent: string | null, before: { kind: string; id: string } | null) {
  try {
    await call('items_move', { ws, items, parent, before });
    await loadSnapshot(ws);
  } catch (e) { toastError(e); }
}

export async function copyItems(ws: string, items: { kind: string; id: string }[], parent: string | null) {
  try {
    for (const i of items) await call(i.kind === 'connection' ? 'conn_duplicate' : 'folder_duplicate', { ws, id: i.id, target: parent ?? '' });
    await loadSnapshot(ws);
  } catch (e) { toastError(e); }
}

export function persistExpanded() {
  const keys = Object.entries(getState().expanded).map(([k, v]) => (k.split(':')[1] === 'ws' ? (v ? null : `${k}:closed`) : v ? k : null)).filter(Boolean) as string[];
  updateSettings({ expanded: keys });
}

export function loadExpanded() {
  const map: Record<string, boolean> = {};
  for (const k of getState().settings.expanded) {
    if (k.endsWith(':closed')) map[k.slice(0, -7)] = false; else map[k] = true;
  }
  setState({ expanded: map });
}

// ------------------------------------------------------------------ tunnels

export async function refreshTunnels() {
  try { setState({ tunnels: await call('tunnels_status') }); } catch { /* shell not ready */ }
}

export async function startTunnelProfile(ws: string, id: string, exposes: boolean) {
  if (exposes) {
    const r = await confirmDialog({ title: 'Expose a forward to the network?', body: 'At least one forward listens on an address other than loopback; other machines may be able to use it.', ok: 'Start anyway', danger: true });
    if (!r.ok) return;
  }
  try {
    const addrs = await call<string[]>('tunnel_start', { ws, id, allowExposed: exposes });
    toast(addrs.length ? `Listening on ${addrs.join(', ')}` : 'Jump chain connected', 'success', 5000);
  } catch (e) { toastError(e); }
}

// ------------------------------------------------------------------ prompts

export function respondPrompt(id: string, value: unknown) {
  setState((s) => ({ prompts: s.prompts.filter((p) => p.id !== id) }));
  void call('prompt_respond', { id, value });
}

// ------------------------------------------------------------------ tabs & panes

const controllers = new Map<string, TermController>();
let tabSeq = 1, paneSeq = 2;
const rdpViews = new Map<string, RdpController>();
export const getController = (tabId: string) => controllers.get(tabId);
export const getView = (tabId: string): { mount(p: HTMLElement): void; refit(): void; focus(): void } | undefined => controllers.get(tabId) ?? rdpViews.get(tabId);

function mapLeaf(n: PaneNode, f: (l: Extract<PaneNode, { kind: 'leaf' }>) => PaneNode): PaneNode {
  if (n.kind === 'leaf') return f(n);
  return { ...n, a: mapLeaf(n.a, f), b: mapLeaf(n.b, f) };
}
export function findLeaf(n: PaneNode, id: string): Extract<PaneNode, { kind: 'leaf' }> | null {
  if (n.kind === 'leaf') return n.id === id ? n : null;
  return findLeaf(n.a, id) ?? findLeaf(n.b, id);
}
function firstLeaf(n: PaneNode): Extract<PaneNode, { kind: 'leaf' }> { return n.kind === 'leaf' ? n : firstLeaf(n.a); }

/** Removes empty leaves, collapsing splits into the surviving sibling; a lone root stays. */
function prune(n: PaneNode, isRoot = true): PaneNode | null {
  if (n.kind === 'leaf') return n.tabs.length === 0 && !isRoot ? null : n;
  const a = prune(n.a, false), b = prune(n.b, false);
  if (a && b) return { ...n, a, b };
  return a ?? b ?? (isRoot ? rootLeaf() : null);
}

function setRoot(root: PaneNode) {
  const pruned = prune(root) ?? rootLeaf();
  setState((s) => ({ root: pruned, activePane: findLeaf(pruned, s.activePane) ? s.activePane : firstLeaf(pruned).id }));
}

export function setActivePane(id: string) { setState({ activePane: id }); }
export function activateTab(paneId: string, tabId: string) {
  setRoot(mapLeaf(getState().root, (l) => (l.id === paneId ? { ...l, active: tabId } : l)));
  setState({ activePane: paneId });
  window.setTimeout(() => { controllers.get(tabId)?.refit(); controllers.get(tabId)?.focus(); }, 0);
}

export function splitPane(dir: 'row' | 'column') {
  const { activePane, root } = getState();
  const leaf = findLeaf(root, activePane);
  if (!leaf) return;
  const newId = `pane-${paneSeq++}`;
  const moving = leaf.tabs.length > 1 ? leaf.active : null;
  const keep = { ...leaf, tabs: moving ? leaf.tabs.filter((t) => t !== moving) : leaf.tabs, active: moving ? leaf.tabs.find((t) => t !== moving) ?? null : leaf.active };
  const split = (n: PaneNode): PaneNode => {
    if (n.kind === 'leaf') return n.id === leaf.id
      ? { kind: 'split', id: `split-${paneSeq++}`, dir, ratio: 0.5, a: keep, b: { kind: 'leaf', id: newId, tabs: moving ? [moving] : [], active: moving } }
      : n;
    return { ...n, a: split(n.a), b: split(n.b) };
  };
  setRoot(split(root));
  setState({ activePane: newId });
  window.setTimeout(() => Object.values(getState().tabs).forEach((t) => controllers.get(t.id)?.refit()), 30);
}

export function setRatio(splitId: string, ratio: number) {
  const f = (n: PaneNode): PaneNode => (n.kind === 'leaf' ? n : n.id === splitId ? { ...n, ratio: Math.min(0.85, Math.max(0.15, ratio)) } : { ...n, a: f(n.a), b: f(n.b) });
  setState((s) => ({ root: f(s.root) }));
}

export function moveTab(tabId: string, toPane: string, index: number | null) {
  const { root } = getState();
  const without = mapLeaf(root, (l) => (l.tabs.includes(tabId) ? { ...l, tabs: l.tabs.filter((t) => t !== tabId), active: l.active === tabId ? l.tabs.find((t) => t !== tabId) ?? null : l.active } : l));
  const placed = mapLeaf(without, (l) => {
    if (l.id !== toPane) return l;
    const tabs = [...l.tabs];
    tabs.splice(index ?? tabs.length, 0, tabId);
    return { ...l, tabs, active: tabId };
  });
  setRoot(placed);
  setState({ activePane: toPane });
  window.setTimeout(() => controllers.get(tabId)?.refit(), 30);
}

export function cycleTab(delta: number) {
  const leaf = findLeaf(getState().root, getState().activePane);
  if (!leaf || leaf.tabs.length < 2) return;
  const i = leaf.tabs.indexOf(leaf.active ?? '');
  activateTab(leaf.id, leaf.tabs[(i + delta + leaf.tabs.length) % leaf.tabs.length]);
}

export function saveLastTabs() {
  const tabs = Object.values(getState().tabs).map((t) => ({ ws: t.target.ws, id: t.target.conn_id, quick: t.target.quick }));
  updateSettings({ last_tabs: tabs });
}

function recordRecent(ws: string, id: string) {
  const r = getState().settings.recents.filter((x) => !(x.ws === ws && x.id === id));
  updateSettings({ recents: [{ ws, id, at: Date.now() }, ...r].slice(0, 25) });
}

export async function openConnection(ws: string, id: string, paneId?: string) {
  const snap = snapOf(ws);
  const c = snap?.connections.find((x) => x.id === id);
  if (!snap || !c) return;
  recordRecent(ws, id);
  if (c.protocol === 'ssh') {
    openTab({ ws, conn_id: id, quick: null }, c.name, 'ssh', inheritedColor(snap, c.folder_id), paneId);
    return;
  }
  if (c.protocol === 'rdp' && !String(resolveOption(snap, c, 'rdp_client_path') ?? '').trim()) {
    openTab({ ws, conn_id: id, quick: null }, c.name, 'rdp', inheritedColor(snap, c.folder_id), paneId);
    return;
  }
  try {
    const r = await call<{ client: string; message: string }>('launch_external', { target: { ws, conn_id: id, quick: null } });
    toast(r.message, 'success', 6000);
  } catch (e) { toastError(e); }
}

export async function openQuick(text: string, paneId?: string) {
  try {
    const t = await call<any>('quick_parse', { input: text });
    if (t.protocol === 'ssh') { openTab({ ws: null, conn_id: null, quick: text }, t.name, 'ssh', null, paneId); return true; }
    if (t.protocol === 'http' || t.protocol === 'https') { await import('@tauri-apps/plugin-opener').then((m) => m.openUrl(t.url)); return true; }
    toast(`${t.protocol.toUpperCase()} quick connect needs a saved connection in this version`, 'info');
    return true;
  } catch (e) { toastError(e); return false; }
}

function openTab(target: Target, title: string, protocol: Tab['protocol'], color: string | null, paneId?: string) {
  const id = `tab-${tabSeq++}`;
  const tab: Tab = { id, title, protocol, target, sessionId: null, state: 'connecting', error: null, color, route: null, zoom: 0,
    termProfile: null, osc52: false, fixedCols: null, fixedRows: null, logFile: null, cols: 0, rows: 0 };
  const pane = paneId ?? getState().activePane;
  setState((s) => ({ tabs: { ...s.tabs, [id]: tab } }));
  setRoot(mapLeaf(getState().root, (l) => (l.id === pane ? { ...l, tabs: [...l.tabs, id], active: id } : l)));
  setState({ activePane: pane });
  saveLastTabs();
  requestAnimationFrame(() => void startSession(id));
}

function patchTab(id: string, p: Partial<Tab>) {
  setState((s) => (s.tabs[id] ? { tabs: { ...s.tabs, [id]: { ...s.tabs[id], ...p } } } : {}));
}

function ensureController(tabId: string): TermController {
  let c = controllers.get(tabId);
  if (c) return c;
  const s = () => getState().settings;
  c = new TermController({
    onInput: (d) => { const sid = getState().tabs[tabId]?.sessionId; if (sid) void call('session_write', { id: sid, data: Array.from(new TextEncoder().encode(d)) }); },
    onResize: (cols, rows) => {
      patchTab(tabId, { cols, rows });
      const sid = getState().tabs[tabId]?.sessionId;
      if (sid) void call('session_resize', { id: sid, cols, rows });
    },
    copyText: (t) => void writeText(t),
    readText: () => readText().catch(() => ''),
    confirmPaste: async (t) => {
      const lines = t.replace(/[\r\n]+$/, '').split(/\r\n|\r|\n/).length;
      const r = await confirmDialog({ title: 'Paste multiple lines?', body: `You are pasting ${lines} lines. Each line may run as a command.\n\n${t.slice(0, 600)}${t.length > 600 ? '…' : ''}`, ok: 'Paste' });
      return r.ok;
    },
    appKey: (e) => handleShortcut(e, true),
    toggleSearch: () => setState((st) => ({ searchOpen: { ...st.searchOpen, [tabId]: !st.searchOpen[tabId] } })),
    zoom: (d) => zoomTab(tabId, d),
  }, s().terminal, s);
  controllers.set(tabId, c);
  return c;
}

export function zoomTab(tabId: string, d: number | 'reset') {
  const t = getState().tabs[tabId];
  if (!t) return;
  const zoom = d === 'reset' ? 0 : Math.min(24, Math.max(-8, t.zoom + d));
  patchTab(tabId, { zoom });
  controllers.get(tabId)?.apply(effectiveTerm(t), zoom);
}

function effectiveTerm(t: Tab) {
  const base = getState().settings.terminal;
  const snap = t.target.ws ? snapOf(t.target.ws) : null;
  const prof = t.termProfile && snap?.terminal_profiles.find((p) => p.id === t.termProfile);
  const c = t.target.ws && t.target.conn_id && snap ? snap.connections.find((x) => x.id === t.target.conn_id) : null;
  const fontSize = c && snap ? (resolveOption(snap, c, 'terminal_font_size') as number | undefined) : undefined;
  return { ...base, ...(prof ? (prof.spec as object) : {}), ...(fontSize ? { fontSize } : {}) };
}

export function applyTerminalSettings() {
  for (const t of Object.values(getState().tabs)) controllers.get(t.id)?.apply(effectiveTerm(t), t.zoom);
}

async function startRdp(tabId: string) {
  const tab = getState().tabs[tabId];
  if (!tab) return;
  rdpViews.get(tabId)?.dispose();
  const view = new RdpController(
    (reason) => {
      patchTab(tabId, { state: 'closed', sessionId: null, error: isNormalClose(reason) ? null : reason });
    },
    (w, h) => patchTab(tabId, { cols: w, rows: h }),
  );
  rdpViews.set(tabId, view);
  patchTab(tabId, { state: 'connecting', error: null });
  const host = document.querySelector<HTMLElement>(`[data-tab="${tabId}"]`);
  if (host) view.mount(host);
  const { width, height } = view.size();
  const channel = new Channel<ArrayBuffer>();
  channel.onmessage = (b) => view.handle(b);
  try {
    const r = await call<any>('rdp_open', { target: tab.target, width, height, binary: channel });
    if (!getState().tabs[tabId]) { void call('rdp_close', { id: r.id }); return; }
    view.start(r.id, r.width, r.height);
    patchTab(tabId, { sessionId: r.id, state: 'connected', route: r.route, color: r.color ?? tab.color });
  } catch (e) {
    const err = isApiError(e) ? e : { code: 'error', message: String(e) };
    if (err.code === 'external') {
      closeTab(tabId, true);
      try { const x = await call<{ message: string }>('launch_external', { target: tab.target }); toast(x.message, 'success', 6000); } catch (e2) { toastError(e2); }
    } else if (err.code === 'cancelled') {
      closeTab(tabId, true);
      toast(err.message, 'info');
    } else {
      patchTab(tabId, { state: 'error', error: err.message });
      toast(`${tab.title}: ${err.message}`, 'error', 9000);
    }
  }
}

export async function startSession(tabId: string) {
  const tab = getState().tabs[tabId];
  if (!tab) return;
  if (tab.protocol === 'rdp') return startRdp(tabId);
  const ctrl = ensureController(tabId);
  patchTab(tabId, { state: 'connecting', error: null });
  const { cols, rows } = ctrl.fitNow();
  const channel = new Channel<ArrayBuffer>();
  channel.onmessage = (buf) => ctrl.write(new Uint8Array(buf));
  try {
    const r = await call<any>('session_open', { target: tab.target, cols: Math.max(cols, 20), rows: Math.max(rows, 5), binary: channel });
    if (!getState().tabs[tabId]) { void call('session_close', { id: r.id }); return; }
    ctrl.setOsc52(!!r.osc52);
    if (r.fixed_cols && r.fixed_rows) ctrl.setFixed(r.fixed_cols, r.fixed_rows);
    patchTab(tabId, { sessionId: r.id, state: 'connected', route: r.route, termProfile: r.terminal_profile, osc52: !!r.osc52,
      fixedCols: r.fixed_cols, fixedRows: r.fixed_rows, logFile: r.log_file, color: r.color ?? tab.color });
    ctrl.apply(effectiveTerm(getState().tabs[tabId]), getState().tabs[tabId].zoom);
    ctrl.announce(true);
    ctrl.focus();
  } catch (e) {
    const err = isApiError(e) ? e : { code: 'error', message: String(e) };
    if (err.code === 'cancelled' && err.message !== 'Sign-in was cancelled.' && /was not opened/.test(err.message)) {
      closeTab(tabId, true);
      toast(err.message, 'info');
    } else if (err.code === 'cancelled') {
      patchTab(tabId, { state: 'closed', error: 'Sign-in was cancelled.' });
    } else {
      patchTab(tabId, { state: 'error', error: err.message });
      toast(`${tab.title}: ${err.message}`, 'error', 9000);
    }
  }
}

export function reconnectTab(tabId: string) {
  const t = getState().tabs[tabId];
  if (!t || t.state === 'connecting') return;
  if (t.protocol === 'rdp') {
    if (t.sessionId) void call('rdp_close', { id: t.sessionId });
    patchTab(tabId, { sessionId: null });
    void startSession(tabId);
    return;
  }
  const ctrl = controllers.get(tabId);
  if (t.state === 'connected' && t.sessionId) {
    void call('session_close', { id: t.sessionId });
    ctrl?.write('\r\n\x1b[2m[session closed]\x1b[0m\r\n');
  }
  const d = new Date();
  ctrl?.write(`\r\n\x1b[2m[reconnected at ${d.toLocaleTimeString([], { hour12: false })}]\x1b[0m\r\n`);
  patchTab(tabId, { sessionId: null });
  void startSession(tabId);
}

export async function closeTab(tabId: string, force = false) {
  const t = getState().tabs[tabId];
  if (!t) return;
  if (!force && t.state === 'connected' && getState().settings.confirm_tab_close) {
    const r = await confirmDialog({ title: 'Close this session?', body: `${t.title} is still connected.`, ok: 'Close', checkbox: "Don't ask again" });
    if (!r.ok) return;
    if (r.checked) updateSettings({ confirm_tab_close: false });
  }
  if (t.sessionId) void call(t.protocol === 'rdp' ? 'rdp_close' : 'session_close', { id: t.sessionId });
  rdpViews.get(tabId)?.dispose();
  rdpViews.delete(tabId);
  controllers.get(tabId)?.dispose();
  controllers.delete(tabId);
  setRoot(mapLeaf(getState().root, (l) => {
    if (!l.tabs.includes(tabId)) return l;
    const i = l.tabs.indexOf(tabId);
    const tabs = l.tabs.filter((x) => x !== tabId);
    return { ...l, tabs, active: l.active === tabId ? tabs[Math.min(i, tabs.length - 1)] ?? null : l.active };
  }));
  setState((s) => { const tabs = { ...s.tabs }; delete tabs[tabId]; return { tabs }; });
  saveLastTabs();
}

export function closeActiveTab() {
  const leaf = findLeaf(getState().root, getState().activePane);
  if (leaf?.active) void closeTab(leaf.active);
}

export function onSessionState(p: { id: string; state: string; exit_status: number | null }) {
  const tab = Object.values(getState().tabs).find((t) => t.sessionId === p.id);
  if (!tab) return;
  controllers.get(tab.id)?.write('\r\n\x1b[2m[session closed]\x1b[0m\r\n');
  patchTab(tab.id, { state: 'closed', sessionId: null });
}

export async function saveFontSize(tabId: string) {
  const t = getState().tabs[tabId];
  if (!t?.target.ws || !t.target.conn_id) return;
  const snap = snapOf(t.target.ws);
  const c = snap?.connections.find((x) => x.id === t.target.conn_id);
  if (!c) return;
  const options = { ...c.options };
  if (t.zoom === 0) delete options.terminal_font_size; else options.terminal_font_size = Math.max(6, effectiveTerm({ ...t, zoom: 0 }).fontSize + t.zoom);
  try { await call('conn_save', { ws: t.target.ws, conn: { ...c, options } }); await loadSnapshot(t.target.ws); toast('Font size saved', 'success', 2500); } catch (e) { toastError(e); }
}

export async function restoreTabs() {
  const tabs = getState().settings.last_tabs;
  for (const t of tabs) {
    if (t.quick) await openQuick(t.quick);
    else if (t.ws && t.id) await openConnection(t.ws, t.id);
  }
}

// ------------------------------------------------------------------ keyboard

export const DEFAULT_KEYS: Record<string, { combo: string; label: string }> = {
  palette: { combo: 'Mod+K', label: 'Open the command palette' },
  quickConnect: { combo: 'Mod+L', label: 'Quick connect' },
  filterTree: { combo: 'Mod+F', label: 'Filter the tree' },
  newConnection: { combo: 'Mod+N', label: 'New connection' },
  newFolder: { combo: 'Mod+Shift+N', label: 'New folder' },
  editSelected: { combo: 'F2', label: 'Edit the focused item' },
  closeTab: { combo: 'Mod+W', label: 'Close the active tab' },
  nextTab: { combo: 'Ctrl+Tab', label: 'Next tab' },
  prevTab: { combo: 'Ctrl+Shift+Tab', label: 'Previous tab' },
  splitRight: { combo: 'Mod+D', label: 'Split pane side by side' },
  splitDown: { combo: 'Mod+Shift+D', label: 'Split pane top and bottom' },
  lockAll: { combo: 'Mod+Shift+L', label: 'Lock all workspaces' },
  toggleSidebar: { combo: 'Mod+B', label: 'Show or hide the sidebar' },
  zoomIn: { combo: 'Mod+=', label: 'Terminal zoom in' },
  zoomOut: { combo: 'Mod+-', label: 'Terminal zoom out' },
  zoomReset: { combo: 'Mod+0', label: 'Terminal zoom reset' },
};

export const bindingFor = (action: string) => getState().settings.keybindings[action] ?? DEFAULT_KEYS[action]?.combo;

export function comboOf(e: KeyboardEvent): string | null {
  if (['Control', 'Shift', 'Alt', 'Meta'].includes(e.key)) return null;
  const mac = navigator.platform.toLowerCase().includes('mac');
  const parts: string[] = [];
  if (mac ? e.metaKey : e.ctrlKey) parts.push('Mod');
  else if (mac && e.ctrlKey) parts.push('Ctrl');
  if (e.altKey) parts.push('Alt');
  if (e.shiftKey) parts.push('Shift');
  if (!mac && e.metaKey) parts.push('Meta');
  let k = e.key === '+' ? '=' : e.key === '_' ? '-' : e.key;
  k = k.length === 1 ? k.toUpperCase() : k;
  parts.push(k);
  return parts.join('+');
}

function actionFor(combo: string): string | null {
  const all = new Set([...Object.keys(DEFAULT_KEYS)]);
  for (const a of all) if (bindingFor(a)?.toLowerCase() === combo.toLowerCase()) return a;
  return null;
}

export function runAction(a: string) {
  const s = getState();
  switch (a) {
    case 'palette': setState({ palette: true }); break;
    case 'quickConnect': document.getElementById('quick-input')?.focus(); break;
    case 'filterTree':
      if (s.settings.sidebar_collapsed) updateSettings({ sidebar_collapsed: false });
      setTimeout(() => document.getElementById('filter-input')?.focus(), 0); break;
    case 'newConnection': newConnection(); break;
    case 'newFolder': void newFolder(); break;
    case 'editSelected': {
      if (!s.focus) break;
      const [ws, kind, id] = s.focus.split(':');
      if (kind === 'conn' || kind === 'folder') editItem(ws, kind, id);
      break;
    }
    case 'closeTab': closeActiveTab(); break;
    case 'nextTab': cycleTab(1); break;
    case 'prevTab': cycleTab(-1); break;
    case 'splitRight': splitPane('row'); break;
    case 'splitDown': splitPane('column'); break;
    case 'lockAll': void lockAll(); break;
    case 'toggleSidebar': updateSettings({ sidebar_collapsed: !s.settings.sidebar_collapsed }); break;
  }
}

/** Returns true when the key was handled. */
export function handleShortcut(e: KeyboardEvent, fromTerminal = false): boolean {
  const s = getState();
  if (s.dialog || s.palette) return false;
  const combo = e.ctrlKey && e.key === 'Tab' ? (e.shiftKey ? 'Ctrl+Shift+Tab' : 'Ctrl+Tab') : comboOf(e);
  if (!combo) return false;
  const a = actionFor(combo);
  if (!a) return false;
  const typing = (e.target as HTMLElement | null)?.closest?.('input, textarea, select');
  if (a === 'editSelected' && typing) return false;
  if (['zoomIn', 'zoomOut', 'zoomReset'].includes(a)) return false;
  if (a === 'filterTree' && fromTerminal) return false;
  e.preventDefault();
  e.stopPropagation();
  runAction(a);
  return true;
}

// ------------------------------------------------------------------ auto-lock

export function startAutoLock() {
  let last = Date.now(), lastTick = Date.now();
  const bump = () => { last = Date.now(); };
  for (const ev of ['keydown', 'mousedown', 'mousemove', 'wheel']) window.addEventListener(ev, bump, true);
  window.setInterval(() => {
    const now = Date.now();
    const gap = now - lastTick;
    lastTick = now;
    const s = getState();
    if (!s.wsOrder.some((id) => s.workspaces[id]?.snap)) return;
    if (gap > 60_000) { if (s.settings.lock_on_sleep) void lockAll(); last = now; return; }
    const mins = s.settings.auto_lock_minutes;
    if (mins > 0 && now - last > mins * 60_000) { void lockAll(); last = now; }
  }, 15_000);
}

// ------------------------------------------------------------------ boot

let booted = false;
export async function boot() {
  if (booted) return;
  booted = true;
  try {
    const info = await call<any>('app_info');
    const settings = await call<Settings>('settings_get');
    setState({ info, settings });
    loadExpanded();
    applyTheme();
    matchMedia('(prefers-color-scheme: dark)').addEventListener('change', applyTheme);
    await listen<PromptMsg>('prompt', (e) => setState((s) => ({ prompts: [...s.prompts, e.payload] })));
    await listen<any>('session-state', (e) => onSessionState(e.payload));
    await listen<string[]>('open-workspaces', (e) => { void call('startup_requests'); e.payload.forEach((p) => void openVaultFromPath(p)); });
    await listen<any>('tunnels-changed', () => void refreshTunnels());
    await listen<any>('toast', (e) => toast(e.payload.text, e.payload.kind ?? 'info', 8000));
    startAutoLock();

    const requested = await call<string[]>('startup_requests');
    const pinned = settings.pinned_workspace;
    const paths: string[] = [...requested];
    if (settings.startup_mode !== 'chooser' && pinned && !paths.includes(pinned)) paths.push(pinned);
    if (settings.startup_mode === 'last') for (const p of settings.last_open_workspaces) if (!paths.includes(p)) paths.push(p);
    for (const p of paths) { try { await openWorkspacePath(p); } catch (e) { toast(`Could not open ${p}: ${errText(e)}`, 'error', 8000); } }
    await refreshWorkspaces();
    if (settings.restore_tabs === 'always') await restoreTabs();
  } catch (e) {
    setState({ firstRun: true });
    toast(`Startup problem: ${errText(e)}`, 'error', 10000);
  } finally {
    setState({ ready: true });
  }
}
