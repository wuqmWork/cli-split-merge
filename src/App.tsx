import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';
import {
  BarChart3,
  CheckCircle2,
  Clock3,
  Combine,
  FilePlus2,
  FileSpreadsheet,
  FileText,
  FolderOpen,
  Layers3,
  Play,
  Scissors,
  Settings,
  Trash2,
} from 'lucide-react';
import { FileCard, Panel, ProgressBar, StepBadge, formatElapsed } from './components';
import SettingsModal from './SettingsModal';
import { pickCliFiles, pickExcelFile, pickFolder } from './fileDialog';
import { applyRuntimeSettings, isTauriRuntime, loadParameterExcel, loadSavedParameterSnapshot, mergeCliFiles, onMergeProgress, onSplitProgress, splitCliFiles } from './backend';
import type { Page, SavedParameterSnapshot, SelectedCliFile, SettingsState, TaskProgress } from './types';

const SETTINGS_STORAGE_KEY = 'cli-split-merge.settings.v1';
const OUTPUT_STORAGE_KEY = 'cli-split-merge.output-dir.v1';
const OVERLAP_STORAGE_KEY = 'cli-split-merge.overlap-mm.v1';

const defaultSettings: SettingsState = {
  rememberLastOutput: true,
  splitNamingTemplate: '{index:03}_{source_stem}_{mirror}.cli',
  appearance: 'light',
  parallel: true,
  workerCount: 0,
  readBufferMb: 16, // merge 仍使用；split 的 mmap 路径忽略此值
  writeBufferMb: 2, // v14.1: 2 MB 写缓冲更好地摊薄 syscall 开销
  logLevel: 'info',
  logRetentionDays: 14,
  autoClearCache: false,
};

function createInitialProgress(): TaskProgress {
  return { running: false, percent: 0, completed: 0, total: 0, currentFile: '', elapsedSeconds: 0 };
}

export default function App() {
  const [page, setPage] = useState<Page>('split');
  const [splitFiles, setSplitFiles] = useState<SelectedCliFile[]>([]);
  const [mergeFiles, setMergeFiles] = useState<SelectedCliFile[]>([]);
  const [parameter, setParameter] = useState<SavedParameterSnapshot | null>(null);
  const [splitOutput, setSplitOutput] = useState(() => localStorage.getItem(OUTPUT_STORAGE_KEY) || 'D:\\output');
  const [mergeOutput, setMergeOutput] = useState('D:\\output\\merged.cli');
  const [overlapMm, setOverlapMm] = useState(() => {
    const raw = localStorage.getItem(OVERLAP_STORAGE_KEY);
    if (raw === null) return 2.0;
    const saved = Number(raw);
    return Number.isFinite(saved) ? Math.min(20, Math.max(0, saved)) : 2.0;
  });
  const [progress, setProgress] = useState<TaskProgress>(() => createInitialProgress());
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsState, setSettingsState] = useState<SettingsState>(() => ({ ...defaultSettings, ...(loadJson(SETTINGS_STORAGE_KEY) ?? {}) }));
  const timerRef = useRef<number | null>(null);
  const [showSplash, setShowSplash] = useState(Boolean(settingsState.splashPath));

  useEffect(() => {
    localStorage.setItem(OVERLAP_STORAGE_KEY, String(overlapMm));
  }, [overlapMm]);

  useEffect(() => {
    localStorage.setItem(SETTINGS_STORAGE_KEY, JSON.stringify(settingsState));
    if (isTauriRuntime()) {
      applyRuntimeSettings({
        logLevel: settingsState.logLevel,
        logRetentionDays: settingsState.logRetentionDays,
        autoClearCache: settingsState.autoClearCache,
      }).catch((error) => console.error('应用运行设置失败', error));
    }
  }, [settingsState]);

  useEffect(() => {
    const media = window.matchMedia('(prefers-color-scheme: dark)');
    const applyTheme = () => {
      const dark = settingsState.appearance === 'dark' || (settingsState.appearance === 'system' && media.matches);
      document.documentElement.classList.toggle('dark', dark);
      document.documentElement.style.colorScheme = dark ? 'dark' : 'light';
    };
    applyTheme();
    media.addEventListener('change', applyTheme);
    return () => media.removeEventListener('change', applyTheme);
  }, [settingsState.appearance]);

  useEffect(() => {
    if (!settingsState.splashPath) { setShowSplash(false); return; }
    setShowSplash(true);
    const timer = window.setTimeout(() => setShowSplash(false), 1200);
    return () => window.clearTimeout(timer);
  }, [settingsState.splashPath]);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    loadSavedParameterSnapshot()
      .then((snapshot) => {
        if (snapshot) {
          setParameter(snapshot);
        }
      })
      .catch((error) => console.error('恢复参数快照失败', error));
  }, []);

  useEffect(() => () => { if (timerRef.current) window.clearInterval(timerRef.current); }, []);


  async function chooseCliFiles() {
    if (isTauriRuntime()) {
      const tauriFiles = await pickCliFiles();
      if (tauriFiles === null) return; // 用户取消时保留已有选择
      const unique = dedupeFiles(tauriFiles);
      page === 'split' ? setSplitFiles(unique) : setMergeFiles(unique);
      return;
    }
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = '.cli';
    input.multiple = true;
    input.onchange = () => {
      const files = dedupeFiles(Array.from(input.files ?? []).map((file) => ({ name: file.name, path: file.name, size: file.size })));
      page === 'split' ? setSplitFiles(files) : setMergeFiles(files);
    };
    input.click();
  }

  async function chooseParameter() {
    const tauriPath = await pickExcelFile();
    if (tauriPath && isTauriRuntime()) {
      try {
        const snapshot = await loadParameterExcel(tauriPath);
        setParameter(snapshot);
      } catch (error) {
        window.alert(`参数文件加载失败：${String(error)}`);
      }
      return;
    }

    // 纯浏览器预览模式无法读取 Excel 内容，仅保留界面演示能力。
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = '.xlsx,.xls';
    input.onchange = () => {
      const file = input.files?.[0];
      if (!file) return;
      window.alert('浏览器预览模式不会解析 Excel。请在 Tauri 桌面程序中选择参数文件。');
    };
    input.click();
  }

  async function chooseSplitOutput() {
    const folder = await pickFolder();
    if (!folder) return;
    setSplitOutput(folder);
    if (settingsState.rememberLastOutput) localStorage.setItem(OUTPUT_STORAGE_KEY, folder);
  }

  async function chooseMergeOutputFolder() {
    const folder = await pickFolder();
    if (!folder) return;
    const filename = mergeOutput.split(/[\\/]/).pop() || 'merged.cli';
    setMergeOutput(joinPath(folder, filename));
  }

  async function startTask() {
    if (!isTauriRuntime()) {
      window.alert(page === 'split' ? 'CLI 分割需要在 Tauri 桌面程序中运行。' : 'CLI 合并需要在 Tauri 桌面程序中运行。');
      return;
    }

    if (page === 'merge') {
      if (mergeFiles.length < 2) {
        window.alert('CLI 合并至少需要选择两个文件。');
        return;
      }
      if (!mergeOutput.trim()) {
        window.alert('请设置输出文件路径。');
        return;
      }
      if (progress.running) return;

      if (timerRef.current) window.clearInterval(timerRef.current);
      const startedAt = Date.now();
      setProgress({ running: true, percent: 0, completed: 0, total: mergeFiles.length, currentFile: mergeFiles[0].name, elapsedSeconds: 0 });
      timerRef.current = window.setInterval(() => {
        setProgress((prev) => ({ ...prev, elapsedSeconds: Math.floor((Date.now() - startedAt) / 1000) }));
      }, 250);

      let unlisten: (() => void) | null = null;
      try {
        unlisten = await onMergeProgress((event) => {
          setProgress((prev) => ({
            ...prev,
            running: true,
            percent: Math.max(prev.percent, Math.round(event.percent * 10) / 10),
            completed: Math.max(prev.completed, event.completed),
            total: event.total,
            currentFile: event.currentFile,
          }));
        });

        const result = await mergeCliFiles({
          inputFiles: mergeFiles.map((file) => file.path),
          outputFile: mergeOutput,
          readBufferMb: settingsState.readBufferMb,
          writeBufferMb: settingsState.writeBufferMb,
          parallel: settingsState.parallel,
          workerCount: settingsState.workerCount,
        });

        // 后端会在缺失扩展名时自动补 .cli；同步回 UI。
        setMergeOutput(result.outputFile);
        setProgress((prev) => ({
          ...prev,
          running: false,
          percent: 100,
          completed: mergeFiles.length,
          total: mergeFiles.length,
          currentFile: result.outputFile.split(/[\\/]/).pop() || 'merged.cli',
          elapsedSeconds: Math.floor((Date.now() - startedAt) / 1000),
        }));
      } catch (error) {
        setProgress((prev) => ({ ...prev, running: false }));
        window.alert(`CLI 合并失败：${String(error)}`);
      } finally {
        unlisten?.();
        if (timerRef.current) {
          window.clearInterval(timerRef.current);
          timerRef.current = null;
        }
      }
      return;
    }
    if (splitFiles.length === 0) {
      window.alert('请先选择至少一个 CLI 文件。');
      return;
    }
    if (!parameter) {
      window.alert('请先选择分割参数 Excel 文件。');
      return;
    }
    if (!splitOutput.trim()) {
      window.alert('请选择输出文件夹。');
      return;
    }
    if (progress.running) return;

    if (timerRef.current) window.clearInterval(timerRef.current);
    const startedAt = Date.now();
    setProgress({ running: true, percent: 0, completed: 0, total: splitFiles.length, currentFile: splitFiles[0].name, elapsedSeconds: 0 });
    timerRef.current = window.setInterval(() => {
      setProgress((prev) => ({ ...prev, elapsedSeconds: Math.floor((Date.now() - startedAt) / 1000) }));
    }, 250);

    let unlisten: (() => void) | null = null;
    try {
      unlisten = await onSplitProgress((event) => {
        setProgress((prev) => ({
          ...prev,
          running: true,
          // 并行模式下事件可能乱序到达，进度只进不退（与 merge 侧防护一致）
          percent: Math.max(prev.percent, Math.round(event.percent * 10) / 10),
          completed: Math.max(prev.completed, event.completed),
          total: event.total,
          currentFile: event.currentFile,
        }));
      });

      const result = await splitCliFiles({
        inputFiles: splitFiles.map((file) => file.path),
        outputDir: splitOutput,
        namingTemplate: settingsState.splitNamingTemplate,
        readBufferMb: settingsState.readBufferMb,
        writeBufferMb: settingsState.writeBufferMb,
        parallel: settingsState.parallel,
        workerCount: settingsState.workerCount,
        overlapMm,
        transforms: parameter.transforms,
      });

      const failedCount = result.failedFiles?.length ?? 0;
      setProgress((prev) => ({
        ...prev,
        running: false,
        percent: 100,
        completed: splitFiles.length - failedCount,
        total: splitFiles.length,
        currentFile: splitFiles[splitFiles.length - 1]?.name ?? '',
        elapsedSeconds: Math.floor((Date.now() - startedAt) / 1000),
      }));

      // 跳过模式：个别坏文件不会中止任务，结束后统一汇总告知。
      if (failedCount > 0) {
        window.alert(
          `分割完成，但有 ${failedCount} 个文件处理失败已跳过：\n` +
            result.failedFiles!.map((name) => `• ${name}`).join('\n') +
            '\n\n这些文件可能不完整（缺少 $$GEOMETRYEND），请重新导出或合并后重试。'
        );
      }
    } catch (error) {
      setProgress((prev) => ({ ...prev, running: false }));
      window.alert(`CLI 分割失败：${String(error)}`);
    } finally {
      unlisten?.();
      if (timerRef.current) {
        window.clearInterval(timerRef.current);
        timerRef.current = null;
      }
    }
  }

  const parameterLoadedAt = useMemo(() => parameter ? formatLoadedTime(parameter.loadedAt) : '', [parameter]);
  const outputForHeader = page === 'split' ? splitOutput : mergeOutput;

  return (
    <div className="min-h-screen bg-[#f8fbff] text-slate-900 dark:bg-slate-950 dark:text-slate-100">
      <div className="flex min-h-screen">
        <aside className="fixed inset-y-0 left-0 z-20 w-[290px] border-r border-slate-200 bg-[#f7faff] px-4 py-5 dark:border-slate-800 dark:bg-slate-950">
          <div className="mb-8 flex items-center gap-3 px-2">
            {settingsState.logoPath ? <img src={convertFileSrc(settingsState.logoPath)} className="h-[38px] w-[38px] rounded-lg object-contain" alt="程序 Logo" /> : <Layers3 size={38} strokeWidth={1.9} className="text-slate-900 dark:text-slate-100" />}
            <div className="text-[22px] font-semibold tracking-tight">CLI Split & Merge</div>
          </div>
          <nav className="space-y-2">
            <NavItem active={page === 'split'} icon={<Scissors size={24} strokeWidth={1.9} />} label="CLI 分割" onClick={() => setPage('split')} />
            <NavItem active={page === 'merge'} icon={<Combine size={24} strokeWidth={1.9} />} label="CLI 合并" onClick={() => setPage('merge')} />
          </nav>
        </aside>

        <main className="ml-[290px] min-h-screen flex-1">
          <header className="flex h-[62px] items-center justify-end border-b border-slate-200 bg-white/70 px-7 backdrop-blur-sm dark:border-slate-800 dark:bg-slate-900/70">
            <button className="icon-button" onClick={() => setSettingsOpen(true)} title="设置"><Settings size={22} strokeWidth={1.9} /></button>
          </header>

          <div className="mx-auto max-w-[1240px] px-7 pb-8 pt-5">
            {page === 'split' ? (
              <SplitPage
                files={splitFiles}
                setFiles={setSplitFiles}
                chooseFiles={chooseCliFiles}
                parameter={parameter}
                parameterLoadedAt={parameterLoadedAt}
                chooseParameter={chooseParameter}
                output={splitOutput}
                overlapMm={overlapMm}
                setOverlapMm={setOverlapMm}
                chooseOutput={chooseSplitOutput}
                setOutput={setSplitOutput}
                startTask={startTask}
                progress={progress}
              />
            ) : (
              <MergePage
                files={mergeFiles}
                setFiles={setMergeFiles}
                chooseFiles={chooseCliFiles}
                output={mergeOutput}
                setOutput={setMergeOutput}
                chooseOutputFolder={chooseMergeOutputFolder}
                startTask={startTask}
                progress={progress}
              />
            )}
            <div className="sr-only">当前输出：{outputForHeader}</div>
          </div>
        </main>
      </div>
      {showSplash && settingsState.splashPath && (
        <div className="fixed inset-0 z-[80] flex items-center justify-center bg-white dark:bg-slate-950">
          <img src={convertFileSrc(settingsState.splashPath)} className="max-h-[70vh] max-w-[70vw] object-contain" alt="启动画面" onError={() => setShowSplash(false)} />
        </div>
      )}
      {settingsOpen && <SettingsModal settings={settingsState} onChange={setSettingsState} onClose={() => setSettingsOpen(false)} />}
    </div>
  );
}

function SplitPage(props: {
  files: SelectedCliFile[];
  setFiles: (files: SelectedCliFile[]) => void;
  chooseFiles: () => void;
  parameter: SavedParameterSnapshot | null;
  parameterLoadedAt: string;
  chooseParameter: () => void;
  output: string;
  overlapMm: number;
  setOverlapMm: (value: number) => void;
  chooseOutput: () => void;
  setOutput: (value: string) => void;
  startTask: () => void;
  progress: TaskProgress;
}) {
  const [overlapText, setOverlapText] = useState(() => String(props.overlapMm));
  useEffect(() => { setOverlapText(String(props.overlapMm)); }, [props.overlapMm]);

  const commitOverlap = () => {
    const parsed = Number(overlapText);
    const normalized = Number.isFinite(parsed) ? Math.min(20, Math.max(0, parsed)) : props.overlapMm;
    props.setOverlapMm(normalized);
    setOverlapText(String(normalized));
  };

  return (
    <>
      <PageTitle icon={<Scissors size={36} strokeWidth={1.9} />} title="CLI 分割" subtitle="根据参数文件将 CLI 文件分割为多个振镜文件。" />
      <InputFilesPanel step={1} files={props.files} setFiles={props.setFiles} chooseFiles={props.chooseFiles} subtitle="选择一个或多个 CLI 文件" />
      <div className="mt-4 grid grid-cols-[minmax(0,1fr)_360px] gap-4">
        <Panel className="p-5">
          <StepHeading step={2} title="参数文件" subtitle="选择分割所需的参数 Excel 文件" />
          <div className="mt-5 flex gap-3">
            <PathField value={props.parameter?.sourcePath ?? ''} placeholder="请选择参数 Excel 文件" icon={<FileSpreadsheet size={20} strokeWidth={1.9} />} readOnly hideClear />
            <button className="secondary-button whitespace-nowrap" onClick={props.chooseParameter}><FolderOpen size={18} strokeWidth={1.9} />选择文件</button>
          </div>
          {props.parameter && (
            <div className="mt-3 flex items-center gap-2 px-1 text-sm">
              <CheckCircle2 size={18} className="text-green-600" strokeWidth={2} />
              <span className="font-medium text-green-600">已记忆当前参数</span>
              <span className="ml-1 text-slate-400 dark:text-slate-500">加载于 {props.parameterLoadedAt}</span>
            </div>
          )}
        </Panel>
        <Panel className="p-5">
          <StepHeading step={3} title="重叠区域" subtitle="相邻分割区域的公共重叠宽度" />
          <div className="mt-5 flex h-12 items-center overflow-hidden rounded-lg border border-slate-200 bg-white focus-within:border-blue-400 focus-within:ring-2 focus-within:ring-blue-100 dark:border-slate-700 dark:bg-slate-900 dark:focus-within:ring-blue-950">
            <input
              type="number"
              min={0}
              max={20}
              step={0.1}
              value={overlapText}
              onChange={(event) => setOverlapText(event.target.value)}
              onBlur={commitOverlap}
              onKeyDown={(event) => { if (event.key === 'Enter') event.currentTarget.blur(); }}
              className="min-w-0 flex-1 bg-transparent px-4 text-base text-slate-800 outline-none dark:text-slate-200"
              aria-label="重叠区域"
            />
            <span className="border-l border-slate-200 px-4 text-sm font-medium text-slate-500 dark:border-slate-700 dark:text-slate-400">mm</span>
          </div>
          <div className="mt-3 text-sm text-slate-400 dark:text-slate-500">默认 2.0 mm，内部边界两侧各扩展一半</div>
        </Panel>
      </div>
      <Panel className="mt-4 p-5">
        <StepHeading step={4} title="输出文件夹" subtitle="选择分割结果的输出目录" />
        <div className="mt-5 flex gap-3">
          <PathField value={props.output} onChange={props.setOutput} placeholder="请选择输出文件夹" icon={<FolderOpen size={20} strokeWidth={1.9} />} />
          <button className="secondary-button whitespace-nowrap" onClick={props.chooseOutput}><FolderOpen size={18} strokeWidth={1.9} />选择文件夹</button>
          <button className="primary-action whitespace-nowrap disabled:cursor-not-allowed disabled:opacity-60" onClick={props.startTask} disabled={props.progress.running}><Play size={19} fill="currentColor" strokeWidth={1.7} />{props.progress.running ? '正在分割' : '开始分割'}</button>
        </div>
      </Panel>
      <TaskProgressPanel progress={props.progress} />
    </>
  );
}

function MergePage(props: {
  files: SelectedCliFile[];
  setFiles: (files: SelectedCliFile[]) => void;
  chooseFiles: () => void;
  output: string;
  setOutput: (value: string) => void;
  chooseOutputFolder: () => void;
  startTask: () => void;
  progress: TaskProgress;
}) {
  return (
    <>
      <PageTitle icon={<Combine size={36} strokeWidth={1.9} />} title="CLI 合并" subtitle="将多个 CLI 文件按层合并为一个文件。" />
      <InputFilesPanel step={1} files={props.files} setFiles={props.setFiles} chooseFiles={props.chooseFiles} subtitle="选择两个或多个 CLI 文件" />
      <Panel className="mt-4 p-5">
        <StepHeading step={2} title="输出设置" subtitle="选择输出文件夹，并可直接修改生成的 .cli 文件名" />
        <div className="mt-5 flex gap-3">
          <PathField value={props.output} onChange={props.setOutput} placeholder="D:\\output\\merged.cli" icon={<FolderOpen size={20} strokeWidth={1.9} />} />
          <button className="secondary-button whitespace-nowrap" onClick={props.chooseOutputFolder}><FolderOpen size={18} strokeWidth={1.9} />选择文件夹</button>
        </div>
      </Panel>
      <PrimaryAction label={props.progress.running ? "正在合并" : "开始合并"} onClick={props.startTask} disabled={props.progress.running} />
      <TaskProgressPanel progress={props.progress} />
    </>
  );
}

function InputFilesPanel({ step, files, setFiles, chooseFiles, subtitle }: { step: number; files: SelectedCliFile[]; setFiles: (files: SelectedCliFile[]) => void; chooseFiles: () => void; subtitle: string }) {
  return (
    <Panel className="mt-4 p-5">
      <div className="flex items-start justify-between gap-6">
        <StepHeading step={step} title="输入文件" subtitle={subtitle} />
        <div className="flex gap-3">
          <button className="primary-small" onClick={chooseFiles}><FilePlus2 size={18} strokeWidth={1.9} />选择文件</button>
          <button className="secondary-button" onClick={() => setFiles([])}><Trash2 size={18} strokeWidth={1.9} />清空</button>
        </div>
      </div>
      <div className="mt-4 text-sm font-medium text-slate-700 dark:text-slate-300">已选择 {files.length} 个文件</div>
      <div className="mt-3 grid grid-cols-4 gap-3">
        {files.map((file, index) => <FileCard key={`${file.path}-${index}`} file={file} onRemove={() => setFiles(files.filter((_, i) => i !== index))} />)}
        {files.length === 0 && <div className="col-span-4 rounded-lg border border-dashed border-slate-300 py-8 text-center text-sm text-slate-400 dark:border-slate-700">尚未选择 CLI 文件</div>}
      </div>
    </Panel>
  );
}

function TaskProgressPanel({ progress }: { progress: TaskProgress }) {
  return (
    <Panel className="mt-5 p-5">
      <div className="mb-3 flex items-center gap-2 text-lg font-semibold"><BarChart3 size={21} strokeWidth={1.9} />任务进度</div>
      <ProgressBar percent={progress.percent} />
      <div className="mt-4 grid grid-cols-3 divide-x divide-slate-200 border-t border-slate-200 pt-4 dark:divide-slate-700 dark:border-slate-700">
        <ProgressStat icon={<CheckCircle2 size={25} strokeWidth={1.9} />} label="已处理" value={`${progress.completed} / ${progress.total}`} />
        <ProgressStat icon={<FileText size={25} strokeWidth={1.9} />} label="当前文件" value={progress.currentFile || '—'} />
        <ProgressStat icon={<Clock3 size={25} strokeWidth={1.9} />} label="运行时间" value={formatElapsed(progress.elapsedSeconds)} />
      </div>
    </Panel>
  );
}

function ProgressStat({ icon, label, value }: { icon: ReactNode; label: string; value: string }) {
  return <div className="flex items-center gap-4 px-5 first:pl-1"><div className="text-slate-800 dark:text-slate-200">{icon}</div><div><div className="text-sm text-slate-500 dark:text-slate-400">{label}</div><div className="mt-1 text-lg font-medium text-slate-900 dark:text-slate-100">{value}</div></div></div>;
}

function PageTitle({ icon, title, subtitle }: { icon: ReactNode; title: string; subtitle: string }) {
  return <div className="flex items-start gap-4 px-3"><div className="mt-1 text-slate-900 dark:text-slate-100">{icon}</div><div><h1 className="text-[32px] font-semibold tracking-tight text-slate-950 dark:text-slate-50">{title}</h1><p className="mt-1 text-[17px] text-slate-500 dark:text-slate-400">{subtitle}</p></div></div>;
}

function StepHeading({ step, title, subtitle }: { step: number; title: string; subtitle: string }) {
  return <div className="flex items-start gap-4"><StepBadge>{step}</StepBadge><div><h2 className="text-xl font-semibold text-slate-950 dark:text-slate-50">{title}</h2><div className="mt-1 text-sm text-slate-500 dark:text-slate-400">{subtitle}</div></div></div>;
}

function NavItem({ active, icon, label, onClick }: { active: boolean; icon: ReactNode; label: string; onClick: () => void }) {
  return <button onClick={onClick} className={`flex w-full items-center gap-4 rounded-lg border-l-[3px] px-5 py-4 text-left text-lg transition ${active ? 'border-blue-600 bg-blue-50 text-blue-600 dark:bg-blue-950/50 dark:text-blue-400' : 'border-transparent text-slate-900 hover:bg-slate-100'}`}>{icon}<span className="font-medium">{label}</span></button>;
}

function PathField({ value, onChange, onClear, placeholder, icon, readOnly = false, hideClear = false }: { value: string; onChange?: (value: string) => void; onClear?: () => void; placeholder: string; icon: ReactNode; readOnly?: boolean; hideClear?: boolean }) {
  return <div className="flex h-12 min-w-0 flex-1 items-center rounded-lg border border-slate-200 bg-white focus-within:border-blue-400 focus-within:ring-2 focus-within:ring-blue-100 dark:border-slate-700 dark:bg-slate-900 dark:focus-within:ring-blue-950"><div className="flex h-full w-12 shrink-0 items-center justify-center border-r border-slate-200 text-slate-700 dark:border-slate-700 dark:text-slate-300">{icon}</div><input className="min-w-0 flex-1 bg-transparent px-3 text-sm text-slate-800 outline-none dark:text-slate-200" value={value} onChange={(e) => onChange?.(e.target.value)} placeholder={placeholder} readOnly={readOnly} />{!hideClear && <button className="icon-button mr-2 h-7 w-7" onClick={() => { if (onClear) onClear(); else onChange?.(''); }} aria-label="清空"><span className="text-xl leading-none text-slate-400">×</span></button>}</div>;
}

function PrimaryAction({ label, onClick, disabled = false }: { label: string; onClick: () => void; disabled?: boolean }) {
  return <div className="my-5 flex justify-center"><button className="primary-action disabled:cursor-not-allowed disabled:opacity-60" onClick={onClick} disabled={disabled}><Play size={19} fill="currentColor" strokeWidth={1.7} />{label}</button></div>;
}

function dedupeFiles(files: SelectedCliFile[]) {
  const seen = new Set<string>();
  return files.filter((file) => {
    const key = file.path.replace(/\\/g, '/').toLocaleLowerCase();
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

function loadJson(key: string) {
  try {
    const raw = localStorage.getItem(key);
    return raw ? JSON.parse(raw) : null;
  } catch {
    return null;
  }
}

function formatLoadedTime(iso: string) {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return '';
  const pad = (value: number) => value.toString().padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function joinPath(folder: string, file: string) {
  const separator = folder.includes('\\') ? '\\' : '/';
  return `${folder.replace(/[\\/]$/, '')}${separator}${file}`;
}
