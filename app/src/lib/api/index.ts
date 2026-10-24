import { tauriApi } from './tauri'
import type { Api } from './types'

export * from './types'

const inTauri = '__TAURI_INTERNALS__' in window

/**
 * The backend. Inside the app it is Rust; in a plain browser during
 * development (`npm run dev`) it is an in-memory mock, so the UI can be
 * worked on without the desktop shell.
 */
export const api: Api =
  inTauri || !import.meta.env.DEV ? tauriApi : (await import('./mock')).createMockApi()
