import { messages, type Locale } from './i18n';
export type Settings = { method: 'RHF' | 'RKS'; basis: string; xc: string | null; max_cycle: number; conv_tol: number; grid_level: number | null };
export const defaultSettings: Settings = {method:'RHF',basis:'sto-3g',xc:null,max_cycle:50,conv_tol:1e-9,grid_level:null};
export function conditionLabel(settings: Settings) { return `${settings.xc ?? settings.method} / ${settings.basis.toUpperCase()}`; }
export function CalculationControls({settings,locale,disabled,onChange}:{settings:Settings;locale:Locale;disabled:boolean;onChange:(settings:Settings)=>void}) {
 const t=messages[locale];
 return <div className="calculation-settings"><div className="heading"><h3>{t.nextCalculation}</h3><button type="button" className="info" aria-label={t.settingsHelp}>ⓘ<span role="tooltip">{t.settingsHelp}</span></button></div>
  <div className="settings-row"><label>{t.method}<select aria-label={t.method} value={settings.xc ?? settings.method} disabled={disabled} onChange={event=>{const method=event.target.value;onChange({...settings,method:method==='RHF'?'RHF':'RKS',xc:method==='RHF'?null:method,grid_level:method==='RHF'?null:settings.grid_level??3});}}><option value="RHF">RHF</option><option value="PBE">DFT · PBE</option><option value="PBE0">DFT · PBE0</option><option value="B3LYP">DFT · B3LYP</option></select></label>
  <label>{t.basis}<select aria-label={t.basis} value={settings.basis} disabled={disabled} onChange={event=>onChange({...settings,basis:event.target.value})}>{['sto-3g','6-31g','def2-svp'].map(basis=><option key={basis} value={basis}>{basis.toUpperCase()}</option>)}</select></label></div>
  <details><summary>{t.scfSettings}</summary><div className="settings-row"><label>{t.maxCycle}<select aria-label={t.maxCycle} disabled={disabled} value={settings.max_cycle} onChange={e=>onChange({...settings,max_cycle:Number(e.target.value)})}>{[...new Set([10,25,50,settings.max_cycle])].sort((a,b)=>a-b).map(value=><option key={value} value={value}>{value}</option>)}</select></label>
  <label>{t.convergence}<select aria-label={t.convergence} disabled={disabled} value={settings.conv_tol} onChange={e=>onChange({...settings,conv_tol:Number(e.target.value)})}>{[...new Set([1e-6,1e-8,1e-9,1e-10,1e-12,settings.conv_tol])].sort((a,b)=>b-a).map(value=><option key={value} value={value}>{value.toExponential(0)} Eh</option>)}</select></label>
  {settings.method==='RKS'?<label>{t.dftGrid}<select aria-label={t.dftGrid} disabled={disabled} value={settings.grid_level??3} onChange={e=>onChange({...settings,grid_level:Number(e.target.value)})}><option value="3">3</option><option value="5">5</option></select></label>:null}</div></details>
 </div>;
}
