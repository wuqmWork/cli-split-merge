export type Page = 'split' | 'merge';

export type SelectedCliFile = {
  name: string;
  path: string;
  size: number;
};

export type MirrorTransform = {
  category: string;
  transformName: string;
  source: string;
  target: string;
  pointCount: number;
  a: number;
  b: number;
  tx: number;
  ty: number;
  rotationDeg: number;
  matrix: [[number, number, number], [number, number, number], [number, number, number]];
};

export type SavedParameterSnapshot = {
  sourcePath: string;
  sourceName: string;
  loadedAt: string;
  schemaVersion: number;
  sheetName: string;
  transformCount: number;
  transforms: MirrorTransform[];
};

export type TaskProgress = {
  running: boolean;
  percent: number;
  completed: number;
  total: number;
  currentFile: string;
  elapsedSeconds: number;
};

export type SettingsState = {
  rememberLastOutput: boolean;
  splitNamingTemplate: string;
  appearance: 'system' | 'light' | 'dark';
  parallel: boolean;
  workerCount: number;
  readBufferMb: number;
  writeBufferMb: number;
  logLevel: 'error' | 'warn' | 'info' | 'debug';
  logRetentionDays: number;
  autoClearCache: boolean;
  logoPath?: string;
  splashPath?: string;
};
