import { useSyncExternalStore } from 'react';
import type { Settings, Snapshot, Tab, PaneNode, PromptMsg, Toast, WsInfo } from './types';

export interface Editor { ws: string; kind: 'conn' | 'folder'; id: string; blank?: any }
export interface DialogState { name: string; props?: any }
export interface MenuState { x: number; y: number; items: MenuItem[] }
export interface MenuItem { label?: string; hint?: string; disabled?: boolean; danger?: boolean; sep?: boolean; run?: () => void }

export interface State {
  ready: boolean;
  info: { product: string; version: string; platform: string; portable: boolean; config_dir: string; logs_dir: string } | null;
  settings: Settings;
  workspaces: Record<string, { info: WsInfo; snap: Snapshot | null }>;
  wsOrder: string[];
  tabs: Record<string, Tab>;
  root: PaneNode;
  activePane: string;
  selection: string[];
  focus: string | null;
  anchor: string | null;
  filter: string;
  chips: string[];
  chipsOpen: boolean;
  expanded: Record<string, boolean>;
  editor: Editor | null;
  dialog: DialogState | null;
  palette: boolean;
  prompts: PromptMsg[];
  toasts: Toast[];
  menu: MenuState | null;
  tunnels: any;
  firstRun: boolean;
  searchOpen: Record<string, boolean>;
}

const defaultSettings = {
  theme: 'system', accent: '#9fcdba', density: 'comfortable', ui_scale: 1, startup_mode: 'last', pinned_workspace: null,
  recent_workspaces: [], last_open_workspaces: [], auto_lock_minutes: 15, lock_on_sleep: true, confirm_tab_close: true,
  backup_retention: 10, clipboard_clear_seconds: 30,
  terminal: {
    fontFamily: 'Menlo', fontSize: 13, fontWeight: 'normal', lineHeight: 1.15, scheme: 'OkRemote Dark', cursorStyle: 'block',
    cursorBlink: true, scrollback: 10000, opacity: 1, bell: 'visual', copyOnSelect: false, rightClickPaste: true,
    confirmMultilinePaste: true, highlightRules: [],
  },
  custom_schemes: [], keybindings: {}, sidebar_width: 290, sidebar_collapsed: false, recents: [], last_tabs: [],
  restore_tabs: 'ask', expanded: [],
} as Settings;

export const rootLeaf = (): PaneNode => ({ kind: 'leaf', id: 'pane-1', tabs: [], active: null });

let state: State = {
  ready: false, info: null, settings: defaultSettings, workspaces: {}, wsOrder: [], tabs: {}, root: rootLeaf(),
  activePane: 'pane-1', selection: [], focus: null, anchor: null, filter: '', chips: [], chipsOpen: false, expanded: {},
  editor: null, dialog: null, palette: false, prompts: [], toasts: [], menu: null, tunnels: null, firstRun: false, searchOpen: {},
};

const listeners = new Set<() => void>();
export const getState = () => state;
export function setState(patch: Partial<State> | ((s: State) => Partial<State>)) {
  const p = typeof patch === 'function' ? patch(state) : patch;
  state = { ...state, ...p };
  listeners.forEach((l) => l());
}
export function subscribe(l: () => void) { listeners.add(l); return () => { listeners.delete(l); }; }

/** Re-renders only when the selected slice changes (reference equality). */
export function useStore<T>(selector: (s: State) => T): T {
  return useSyncExternalStore(subscribe, () => selector(state));
}
