import type { ReactNode } from 'react';
import { X } from 'lucide-react';
import type { SelectedCliFile } from './types';

export function StepBadge({ children }: { children: ReactNode }) {
  return (
    <span className="inline-flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-blue-600 text-lg font-semibold text-white shadow-sm">
      {children}
    </span>
  );
}

export function Panel({ children, className = '' }: { children: ReactNode; className?: string }) {
  return <section className={`rounded-xl border border-slate-200 bg-white shadow-panel dark:border-slate-700 dark:bg-slate-900 ${className}`}>{children}</section>;
}

export function FileCard({ file, onRemove }: { file: SelectedCliFile; onRemove: () => void }) {
  const displaySize = file.size > 0 ? formatBytes(file.size) : 'CLI 文件';
  return (
    <div className="group flex min-w-0 items-center gap-3 rounded-lg border border-slate-200 bg-slate-50/70 dark:border-slate-700 dark:bg-slate-800/60 px-4 py-3 transition hover:border-blue-200 hover:bg-blue-50/30 dark:hover:border-blue-800 dark:hover:bg-blue-950/30">
      <div className="flex min-w-0 flex-1 flex-col">
        <span className="truncate text-sm font-medium text-slate-800 dark:text-slate-200" title={file.path}>{file.name}</span>
        <span className="mt-1 truncate text-xs text-slate-500 dark:text-slate-400">{displaySize}</span>
      </div>
      <button className="icon-button h-7 w-7" onClick={onRemove} title="移除此文件" aria-label={`移除 ${file.name}`}>
        <X size={17} strokeWidth={1.9} />
      </button>
    </div>
  );
}

export function ProgressBar({ percent }: { percent: number }) {
  return (
    <div className="flex items-center gap-4">
      <div className="h-3 flex-1 overflow-hidden rounded-full bg-slate-200 dark:bg-slate-700">
        <div className="h-full rounded-full bg-blue-600 transition-all duration-300" style={{ width: `${Math.max(0, Math.min(100, percent))}%` }} />
      </div>
      <span className="w-14 text-right text-xl font-semibold text-blue-600">{Math.round(percent)}%</span>
    </div>
  );
}

export function formatBytes(bytes: number) {
  if (!bytes) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB'];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(unit >= 2 ? 2 : 0)} ${units[unit]}`;
}

export function formatElapsed(seconds: number) {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = Math.floor(seconds % 60);
  return [h, m, s].map((value) => value.toString().padStart(2, '0')).join(':');
}
