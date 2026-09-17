import { open } from '@tauri-apps/plugin-dialog';
import type { SelectedCliFile } from './types';

function fileNameFromPath(path: string) {
  return path.split(/[\\/]/).pop() ?? path;
}

function isTauriRuntime() {
  return '__TAURI_INTERNALS__' in window;
}

export async function pickCliFiles(): Promise<SelectedCliFile[] | null> {
  if (!isTauriRuntime()) return null;
  const selected = await open({
    multiple: true,
    directory: false,
    filters: [{ name: 'CLI 文件', extensions: ['cli'] }],
  });
  if (!selected) return null;
  const paths = Array.isArray(selected) ? selected : [selected];
  return paths.map((path) => ({ name: fileNameFromPath(path), path, size: 0 }));
}

export async function pickExcelFile(): Promise<string | null> {
  if (!isTauriRuntime()) return null;
  const selected = await open({
    multiple: false,
    directory: false,
    filters: [{ name: 'Excel 参数文件', extensions: ['xlsx', 'xls', 'xlsm', 'xlsb'] }],
  });
  return typeof selected === 'string' ? selected : null;
}

export async function pickFolder(): Promise<string | null> {
  if (!isTauriRuntime()) return null;
  const selected = await open({ multiple: false, directory: true });
  return typeof selected === 'string' ? selected : null;
}


export async function pickImageFile(): Promise<string | null> {
  if (!isTauriRuntime()) return null;
  const selected = await open({
    multiple: false,
    directory: false,
    filters: [{ name: '图片文件', extensions: ['png', 'jpg', 'jpeg', 'webp', 'bmp'] }],
  });
  return typeof selected === 'string' ? selected : null;
}
