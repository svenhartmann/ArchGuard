'use strict';
const LANGUAGE_KEY='archguard-language';
let viewerLanguage='en';
try { const saved=localStorage.getItem(LANGUAGE_KEY);if(saved==='de'||saved==='en')viewerLanguage=saved; } catch(error) { /* Storage is optional in standalone reports. */ }
// Only explicitly marked application text is translated. Names, paths,
// evidence, IDs and analysis payloads never pass through a text replacement.
function t(en,de,values={}) {
  return String(viewerLanguage==='de'?de:en).replace(/\{(\w+)\}/g,(match,key)=>Object.prototype.hasOwnProperty.call(values,key)?String(values[key]):match);
}
const originalUiText=new WeakMap();
function applyStaticLanguage() {
  for(const attribute of ['text','title','aria-label','placeholder']){
    const marker=attribute==='text'?'data-i18n-de':'data-i18n-'+attribute+'-de';
    for(const node of document.querySelectorAll('['+marker+']')){
      let originals=originalUiText.get(node);if(!originals){originals={};originalUiText.set(node,originals);}
      if(!Object.prototype.hasOwnProperty.call(originals,attribute))originals[attribute]=attribute==='text'?node.textContent:node.getAttribute(attribute);
      const value=viewerLanguage==='de'?node.getAttribute(marker):originals[attribute];
      if(attribute==='text')node.textContent=value;else node.setAttribute(attribute,value);
    }
  }
  document.documentElement.setAttribute('lang',viewerLanguage);
  for(const lang of ['de','en'])document.getElementById('language-'+lang).setAttribute('aria-pressed',String(viewerLanguage===lang));
}
function setViewerLanguage(lang) {
  if(!['en','de'].includes(lang)||lang===viewerLanguage)return false;
  const containers=['summary','sidebar','package-tree','services-tree','module-list','calls-relations','calls-evidence','calls-suggestions','performance-panel','review-insights'];
  const state=containers.map(id=>{const node=document.getElementById(id);return {id,top:node.scrollTop,left:node.scrollLeft,open:Array.from(node.querySelectorAll('details'),detail=>detail.open)};});
  const moveTarget=document.getElementById('sidebar').querySelector('select[data-manual-move-target]')?.value;
  viewerLanguage=lang;
  try {localStorage.setItem(LANGUAGE_KEY,lang);} catch(error) { /* Optional persistence. */ }
  hideMetricTooltip();hideContextTooltip();feedback('');applyStaticLanguage();
  matrixRevision=-1;renderAll(false);buildLegend();syncServiceLanguage();syncPanelSizes();renderWorkspaceDescription();
  const moveControl=document.getElementById('sidebar').querySelector('select[data-manual-move-target]');
  if(moveControl&&Array.from(moveControl.children).some(option=>option.value===moveTarget))moveControl.value=moveTarget;
  for(const entry of state){
    const node=document.getElementById(entry.id);Array.from(node.querySelectorAll('details')).forEach((detail,index)=>{if(index<entry.open.length)detail.open=entry.open[index];});
    if(entry.top!==undefined)node.scrollTop=entry.top;if(entry.left!==undefined)node.scrollLeft=entry.left;
  }
  if(typeof refreshActionHelp==='function')refreshActionHelp();
  if(typeof syncKpiHelp==='function')syncKpiHelp();
  document.getElementById('language-'+lang).focus({preventScroll:true});
  return true;
}
for(const lang of ['de','en'])document.getElementById('language-'+lang).addEventListener('click',()=>setViewerLanguage(lang));
applyStaticLanguage();
