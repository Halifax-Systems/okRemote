import { invoke, Channel } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { ApiError } from './types';

export async function call<T = any>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e: any) {
    if (e && typeof e === 'object' && 'code' in e) throw e as ApiError;
    throw { code: 'error', message: String(e?.message ?? e) } as ApiError;
  }
}

export { Channel, listen };
export const isApiError = (e: unknown): e is ApiError => !!e && typeof e === 'object' && 'code' in (e as any);
export const errText = (e: unknown) => (isApiError(e) ? e.message : String((e as any)?.message ?? e));
