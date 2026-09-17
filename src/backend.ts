import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { MirrorTransform, SavedParameterSnapshot } from './types';

export function isTauriRuntime() {
  return '__TAURI_INTERNALS__' in window;
}

export async function loadParameterExcel(path: string): Promise<SavedParameterSnapshot> {
  return invoke<SavedParameterSnapshot>('load_parameter_excel', { path });
}

export async function loadSavedParameterSnapshot(): Promise<SavedParameterSnapshot | null> {
  return invoke<SavedParameterSnapshot | null>('load_saved_parameter_snapshot');
}

export type SplitRequest = {
  inputFiles: string[];
  outputDir: string;
  namingTemplate: string;
  readBufferMb: number;
  writeBufferMb: number;
  parallel: boolean;
  workerCount: number;
  overlapMm: number;
  transforms: MirrorTransform[];
};

export type SplitResult = {
  inputCount: number;
  outputCount: number;
  outputDir: string;
  elapsedMs: number;
};

export type SplitProgressEvent = {
  percent: number;
  completed: number;
  total: number;
  currentFile: string;
  currentFilePercent: number;
  message: string;
};

export async function splitCliFiles(request: SplitRequest): Promise<SplitResult> {
  return invoke<SplitResult>('split_cli_files', { request });
}

export async function onSplitProgress(handler: (event: SplitProgressEvent) => void): Promise<UnlistenFn> {
  return listen<SplitProgressEvent>('split-progress', (event) => handler(event.payload));
}

export type MergeRequest = {
  inputFiles: string[];
  outputFile: string;
  readBufferMb: number;
  writeBufferMb: number;
  parallel: boolean;
  workerCount: number;
};

export type MergeResult = {
  inputCount: number;
  layerCount: number;
  outputFile: string;
  elapsedMs: number;
};

export type MergeProgressEvent = {
  percent: number;
  completed: number;
  total: number;
  currentFile: string;
  message: string;
};

export async function mergeCliFiles(request: MergeRequest): Promise<MergeResult> {
  return invoke<MergeResult>('merge_cli_files', { request });
}

export async function onMergeProgress(handler: (event: MergeProgressEvent) => void): Promise<UnlistenFn> {
  return listen<MergeProgressEvent>('merge-progress', (event) => handler(event.payload));
}


export type RuntimeSettingsRequest = {
  logLevel: 'error' | 'warn' | 'info' | 'debug';
  logRetentionDays: number;
  autoClearCache: boolean;
};

export type StorageInfo = {
  logDir: string;
  cacheDir: string;
  logBytes: number;
  cacheBytes: number;
};

export async function applyRuntimeSettings(settings: RuntimeSettingsRequest): Promise<void> {
  return invoke<void>('apply_runtime_settings', { settings });
}

export async function getStorageInfo(): Promise<StorageInfo> {
  return invoke<StorageInfo>('get_storage_info');
}

export async function clearCache(): Promise<number> {
  return invoke<number>('clear_cache');
}

export async function clearLogs(): Promise<number> {
  return invoke<number>('clear_logs');
}


export async function installUiImage(kind: 'logo' | 'splash', path: string): Promise<string> {
  return invoke<string>('install_ui_image', { kind, path });
}
