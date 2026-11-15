import { messages, type Locale } from './i18n';
export type Settings = { method: 'RHF' | 'RKS' | 'UHF' | 'UKS' | 'GFN2-xTB'; basis: string; xc: string | null; max_cycle: number; conv_tol: number; grid_level: number | null };
export const defaultSettings: Settings = {method:'RHF',basis:'sto-3g',xc:null,max_cycle:50,conv_tol:1e-9,grid_level:null};
export function conditionLabel(settings: Settings) { if (settings.method === 'GFN2-xTB') return settings.method; return `${settings.xc ? `${settings.method} · ${settings.xc}` : settings.method} / ${settings.basis.toUpperCase()}`; }
const methods = [
  { method: 'GFN2-xTB', xc: null },
  { method: 'RHF', xc: null }, { method: 'UHF', xc: null },
  ...(['RKS', 'UKS'] as const).flatMap(method => ['PBE', 'PBE0', 'B3LYP'].map(xc => ({ method, xc }))),
] satisfies Pick<Settings, 'method' | 'xc'>[];
const methodKey = ({ method, xc }: Pick<Settings, 'method' | 'xc'>) => xc ? `${method}/${xc}` : method;

export function CalculationControls({settings,locale,spin,disabled,onChange}:{settings:Settings;locale:Locale;spin:number;disabled:boolean;onChange:(settings:Settings)=>void}) {
 const t=messages[locale];
 const screening = settings.method === 'GFN2-xTB';
 return <div className="calculation-settings"><div className="heading"><h3>{t.nextCalculation}</h3><button type="button" className="info" aria-label={t.settingsHelp}>ⓘ<span role="tooltip">{t.settingsHelp}</span></button></div>
  <div className="settings-row"><label>{t.method}<select aria-label={t.method} value={methodKey(settings)} disabled={disabled} onChange={event=>{
    const selected = methods.find(option => methodKey(option) === event.target.value);
    if (!selected) return;
    if (selected.method === 'GFN2-xTB') {
      onChange({ method: 'GFN2-xTB', basis: 'gfn2', xc: null, grid_level: null, max_cycle: 250, conv_tol: 0 });
    } else {
      onChange({...screening ? defaultSettings : settings,...selected,grid_level:selected.xc ? settings.grid_level ?? 3 : null});
    }
  }}>{methods.map(option => <option key={methodKey(option)} value={methodKey(option)} disabled={spin > 0 && (option.method.startsWith('R') || option.method === 'GFN2-xTB')}>{option.xc ? `${option.method} · ${option.xc}` : option.method}</option>)}</select></label>
  {screening ? <button type="button" className="info" aria-label={t.screeningHelp}>ⓘ<span role="tooltip">{t.screeningHelp}</span></button> : <label>{t.basis}<select aria-label={t.basis} value={settings.basis} disabled={disabled} onChange={event=>onChange({...settings,basis:event.target.value})}>{['sto-3g','6-31g','def2-svp'].map(basis=><option key={basis} value={basis}>{basis.toUpperCase()}</option>)}</select></label>}</div>
  {!screening ? <details><summary>{t.scfSettings}</summary><div className="settings-row"><label>{t.maxCycle}<select aria-label={t.maxCycle} disabled={disabled} value={settings.max_cycle} onChange={e=>onChange({...settings,max_cycle:Number(e.target.value)})}>{[...new Set([10,25,50,settings.max_cycle])].sort((a,b)=>a-b).map(value=><option key={value} value={value}>{value}</option>)}</select></label>
  <label>{t.convergence}<select aria-label={t.convergence} disabled={disabled} value={settings.conv_tol} onChange={e=>onChange({...settings,conv_tol:Number(e.target.value)})}>{[...new Set([1e-6,1e-8,1e-9,1e-10,1e-12,settings.conv_tol])].sort((a,b)=>b-a).map(value=><option key={value} value={value}>{value.toExponential(0)} Eh</option>)}</select></label>
  {settings.xc?<label>{t.dftGrid}<select aria-label={t.dftGrid} disabled={disabled} value={settings.grid_level??3} onChange={e=>onChange({...settings,grid_level:Number(e.target.value)})}><option value="3">3</option><option value="5">5</option></select></label>:null}</div></details> : null}
 </div>;
}
