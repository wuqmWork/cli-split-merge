import { useEffect, useState, type ReactNode } from 'react';
import { X, FolderOpen, Trash2, Gauge, Image, Info, SlidersHorizontal, Database, RefreshCw } from 'lucide-react';
import { getVersion } from '@tauri-apps/api/app';
import { convertFileSrc } from '@tauri-apps/api/core';
import type { SettingsState } from './types';
import { clearCache, clearLogs, getStorageInfo, installUiImage, isTauriRuntime, type StorageInfo } from './backend';
import { formatBytes } from './components';
import { pickImageFile } from './fileDialog';

type Tab = 'general' | 'appearance' | 'performance' | 'logs' | 'about';
const tabs: { id: Tab; label: string; icon: typeof SlidersHorizontal }[] = [
  { id: 'general', label: '常规', icon: SlidersHorizontal },
  { id: 'appearance', label: '外观', icon: Image },
  { id: 'performance', label: '性能', icon: Gauge },
  { id: 'logs', label: '日志与缓存', icon: Database },
  { id: 'about', label: '关于', icon: Info },
];

export default function SettingsModal({ settings, onChange, onClose }: { settings: SettingsState; onChange: (next: SettingsState) => void; onClose: () => void }) {
  const [tab, setTab] = useState<Tab>('general');
  const [version, setVersion] = useState('—');
  const [storage, setStorage] = useState<StorageInfo | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => { getVersion().then(setVersion).catch(() => setVersion('—')); }, []);
  useEffect(() => { if (tab === 'logs') refreshStorage(); }, [tab]);
  const patch = (value: Partial<SettingsState>) => onChange({ ...settings, ...value });
  async function refreshStorage() {
    if (!isTauriRuntime()) return;
    try { setStorage(await getStorageInfo()); } catch { setStorage(null); }
  }
  async function clear(kind: 'cache' | 'logs') {
    if (!isTauriRuntime() || busy) return;
    setBusy(true);
    try {
      if (kind === 'cache') await clearCache(); else await clearLogs();
      await refreshStorage();
    } catch (e) { window.alert(`清理失败：${String(e)}`); }
    finally { setBusy(false); }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/20 p-8 backdrop-blur-[2px] dark:bg-black/50" onMouseDown={onClose}>
      <div className="flex h-[680px] w-[980px] overflow-hidden rounded-2xl border border-slate-200 bg-white shadow-2xl dark:border-slate-700 dark:bg-slate-900" onMouseDown={(e) => e.stopPropagation()}>
        <aside className="w-52 border-r border-slate-200 bg-slate-50/80 p-4 dark:border-slate-700 dark:bg-slate-950/70">
          <div className="mb-5 px-2 text-lg font-semibold text-slate-900 dark:text-slate-100">设置</div>
          <div className="space-y-1">{tabs.map((item) => { const Icon=item.icon; return <button key={item.id} className={`settings-tab ${tab===item.id?'active':''}`} onClick={()=>setTab(item.id)}><Icon size={18} strokeWidth={1.9}/>{item.label}</button>; })}</div>
        </aside>
        <div className="flex min-w-0 flex-1 flex-col">
          <header className="flex h-16 shrink-0 items-center justify-between border-b border-slate-200 px-6 dark:border-slate-700"><h2 className="text-xl font-semibold text-slate-900 dark:text-slate-100">{tabs.find((i)=>i.id===tab)?.label}</h2><button className="icon-button" onClick={onClose}><X size={20}/></button></header>
          <div className="flex-1 overflow-y-auto p-7">
            {tab==='general' && <div className="settings-stack">
              <SettingRow title="分割文件命名规则" description="用于生成各振镜输出 CLI 文件名。"><input className="settings-input w-[360px]" value={settings.splitNamingTemplate} onChange={(e)=>patch({splitNamingTemplate:e.target.value})}/></SettingRow>
              <SettingRow title="记住上次输出目录" description="重新启动程序后自动恢复最近使用的输出目录。"><Toggle checked={settings.rememberLastOutput} onChange={(v)=>patch({rememberLastOutput:v})}/></SettingRow>
            </div>}
            {tab==='appearance' && <div className="settings-stack">
              <SettingRow title="主题" description="立即应用到整个程序界面。"><select className="settings-input w-48" value={settings.appearance} onChange={(e)=>patch({appearance:e.target.value as SettingsState['appearance']})}><option value="system">跟随系统</option><option value="light">浅色</option><option value="dark">深色</option></select></SettingRow>
              <SettingRow title="程序 Logo" description={settings.logoPath || "未设置，将使用默认图标。"}><div className="flex items-center gap-3">{settings.logoPath && <img src={convertFileSrc(settings.logoPath)} className="h-10 w-10 rounded-lg object-contain"/>}<button className="secondary-button" onClick={async()=>{const path=await pickImageFile(); if(path) { const saved=await installUiImage('logo', path); patch({logoPath:saved}); }}}><FolderOpen size={17}/>选择图片</button>{settings.logoPath && <button className="icon-button" onClick={()=>patch({logoPath:undefined})} title="恢复默认"><X size={17}/></button>}</div></SettingRow>
              <SettingRow title="启动画面" description={settings.splashPath || "未设置，不显示自定义启动画面。"}><div className="flex items-center gap-3">{settings.splashPath && <img src={convertFileSrc(settings.splashPath)} className="h-10 w-16 rounded-lg object-cover"/>}<button className="secondary-button" onClick={async()=>{const path=await pickImageFile(); if(path) { const saved=await installUiImage('splash', path); patch({splashPath:saved}); }}}><FolderOpen size={17}/>选择图片</button>{settings.splashPath && <button className="icon-button" onClick={()=>patch({splashPath:undefined})} title="清除"><X size={17}/></button>}</div></SettingRow>
            </div>}
            {tab==='performance' && <div className="settings-stack">
              <SettingRow title="并行处理" description="多个输入 CLI 可并行分割；合并时并行建立层索引。"><Toggle checked={settings.parallel} onChange={(v)=>patch({parallel:v})}/></SettingRow>
              <SettingRow title="线程数" description="0 表示自动使用可用 CPU 线程；SSD 已满载时建议 2～4。"><NumberWithUnit value={settings.workerCount} unit="线程" min={0} max={128} onChange={(v)=>patch({workerCount:v})}/></SettingRow>
              <SettingRow title="读缓冲" description="CLI 输入读取缓冲大小。"><NumberWithUnit value={settings.readBufferMb} unit="MB" min={1} max={1024} onChange={(v)=>patch({readBufferMb:v})}/></SettingRow>
              <SettingRow title="写缓冲" description="单路 CLI 输出写入缓冲大小。"><NumberWithUnit value={settings.writeBufferMb} unit="MB" min={1} max={256} onChange={(v)=>patch({writeBufferMb:v})}/></SettingRow>
            </div>}
            {tab==='logs' && <div className="settings-stack">
              <SettingRow title="日志级别" description="控制 Rust 后端写入的运行日志详细程度。"><select className="settings-input w-40" value={settings.logLevel} onChange={(e)=>patch({logLevel:e.target.value as SettingsState['logLevel']})}><option value="error">错误</option><option value="warn">警告</option><option value="info">信息</option><option value="debug">调试</option></select></SettingRow>
              <SettingRow title="日志保留天数" description="超过保留期的日志会在设置应用时自动清理。"><NumberWithUnit value={settings.logRetentionDays} unit="天" min={1} max={3650} onChange={(v)=>patch({logRetentionDays:v})}/></SettingRow>
              <SettingRow title="日志占用" description={storage?.logDir || '程序数据目录 / logs'}><div className="flex items-center gap-2"><span className="text-sm text-slate-600 dark:text-slate-300">{storage?formatBytes(storage.logBytes):'—'}</span><button className="secondary-button h-10 px-3" disabled={busy} onClick={()=>clear('logs')}><Trash2 size={16}/>清理日志</button></div></SettingRow>
              <SettingRow title="缓存占用" description={storage?.cacheDir || '程序数据目录 / cache'}><div className="flex items-center gap-2"><span className="text-sm text-slate-600 dark:text-slate-300">{storage?formatBytes(storage.cacheBytes):'—'}</span><button className="secondary-button h-10 px-3" disabled={busy} onClick={()=>clear('cache')}><Trash2 size={16}/>清理缓存</button><button className="icon-button" onClick={refreshStorage} title="刷新"><RefreshCw size={17}/></button></div></SettingRow>
              <SettingRow title="退出时清理缓存" description="退出程序时自动删除程序缓存目录。"><Toggle checked={settings.autoClearCache} onChange={(v)=>patch({autoClearCache:v})}/></SettingRow>
            </div>}
            {tab==='about' && <div className="flex h-full flex-col items-center justify-center text-center">{settings.logoPath ? <img src={convertFileSrc(settings.logoPath)} className="mb-5 h-16 w-16 rounded-2xl object-contain" alt="程序 Logo" /> : <div className="mb-5 flex h-16 w-16 items-center justify-center rounded-2xl bg-blue-50 text-blue-600 dark:bg-blue-950/50 dark:text-blue-400"><Info size={34} strokeWidth={1.8}/></div>}<div className="text-2xl font-semibold text-slate-900 dark:text-slate-100">CLI Split & Merge</div><div className="mt-2 text-sm text-slate-500 dark:text-slate-400">版本 v{version}</div><div className="mt-8 grid grid-cols-2 gap-x-12 gap-y-3 text-left text-sm text-slate-600 dark:text-slate-400"><span>桌面框架</span><span className="font-medium text-slate-800 dark:text-slate-200">Tauri 2</span><span>前端</span><span className="font-medium text-slate-800 dark:text-slate-200">React + TypeScript + Tailwind CSS</span><span>图标</span><span className="font-medium text-slate-800 dark:text-slate-200">Lucide React</span><span>核心处理</span><span className="font-medium text-slate-800 dark:text-slate-200">Rust</span></div></div>}
          </div>
        </div>
      </div>
    </div>
  );
}
function SettingRow({title,description,children}:{title:string;description:string;children:ReactNode}) { return <div className="flex items-center justify-between gap-8 rounded-xl border border-slate-200 bg-white p-5 dark:border-slate-700 dark:bg-slate-900"><div><div className="font-medium text-slate-900 dark:text-slate-100">{title}</div><div className="mt-1 max-w-[430px] text-sm text-slate-500 dark:text-slate-400">{description}</div></div><div className="shrink-0">{children}</div></div>; }
function Toggle({checked,onChange}:{checked:boolean;onChange:(v:boolean)=>void}) { return <button className={`relative h-7 w-12 rounded-full transition ${checked?'bg-blue-600':'bg-slate-300 dark:bg-slate-600'}`} onClick={()=>onChange(!checked)}><span className={`absolute top-1 h-5 w-5 rounded-full bg-white shadow transition ${checked?'left-6':'left-1'}`}/></button>; }
function NumberWithUnit({value,unit,onChange,min=0,max=9999}:{value:number;unit:string;onChange:(v:number)=>void;min?:number;max?:number}) { return <div className="flex items-center"><input type="number" min={min} max={max} className="settings-input w-28 rounded-r-none" value={value} onChange={(e)=>onChange(Math.min(max,Math.max(min,Number(e.target.value)||0)))}/><span className="flex h-10 items-center rounded-r-lg border border-l-0 border-slate-200 bg-slate-50 px-3 text-sm text-slate-500 dark:border-slate-700 dark:bg-slate-800 dark:text-slate-400">{unit}</span></div>; }
