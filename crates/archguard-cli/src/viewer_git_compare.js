'use strict';
(() => {
 const $=id=>document.getElementById(id),params=new URLSearchParams(location.search),scope=new URLSearchParams();
 if(params.has('project'))scope.set('project',params.get('project'));
 const suffix=scope.size?'?'+scope.toString():'';
 $('back-to-graph').href='/'+suffix;
 let busy=false,ready=false;
 function enabled(){for(const id of ['compare-from','compare-to','compare-start','compare-index'])$(id).disabled=!ready||busy;}
 function compare(staged) {
  if(!ready||busy)return;
  const query=new URLSearchParams(scope);
  if(!staged){query.set('from',$('compare-from').value);query.set('to',$('compare-to').value);}
  busy=true;enabled();$('comparison-status').textContent='Architekturen werden verglichen…';
  $('comparison-frame').hidden=false;
  $('comparison-frame').src='/__archguard/diff'+(query.size?'?'+query.toString():'');
 }
 $('comparison-form').addEventListener('submit',e=>{e.preventDefault();compare(false);});
 $('compare-index').addEventListener('click',()=>compare(true));
 $('comparison-frame').addEventListener('load',()=>{
  if(!busy)return;
  busy=false;enabled();
  const doc=$('comparison-frame').contentDocument;
  $('comparison-status').textContent=doc?.getElementById('architecture-diff-data')?'Vergleich bereit.':doc?.body?.textContent?.trim()||'Der Vergleich konnte nicht geladen werden.';
 });
 (async()=>{
  try {
   const response=await fetch('/__archguard/refs'+suffix,{cache:'no-store'});
   if(!response.ok)throw Error(await response.text());
   const data=await response.json();
   for(const branch of data.branches)for(const id of ['compare-from','compare-to']) {
    const option=document.createElement('option');option.value=branch.reference;option.textContent=branch.name;$(id).append(option);
   }
   if(!data.branches.length){$('comparison-status').textContent='Keine Branches mit Commits gefunden.';return;}
   $('compare-from').value=data.branches.find(b=>b.reference==='refs/heads/main'||b.reference==='refs/heads/master')?.reference||data.branches[0].reference;
   $('compare-to').value=data.branches.some(b=>b.reference===data.current)?data.current:data.branches.at(-1).reference;
   ready=true;enabled();
   $('comparison-status').textContent=data.truncated?'Die ersten 2.000 Branches sind verfügbar.':'Zwei Branches auswählen und vergleichen.';
  } catch(error) {$('comparison-status').textContent='Branches konnten nicht geladen werden: '+error.message;}
 })();
})();
