// Preferred widths are user intent. Responsive limits only affect the current
// layout, so opening a wider window restores the user's chosen panel sizes.
const PANEL_WIDTH_KEY='archguard.panel-widths.v1';
const PANEL_SIZES={explorer:{initial:238,min:180,max:600},inspector:{initial:420,min:220,max:700}};
let panelWidths={explorer:238,inspector:420},panelResizeInitialized=false,panelDrag=null,panelFixed=null;

function savePanelWidths() {
  try{localStorage.setItem(PANEL_WIDTH_KEY,JSON.stringify(panelWidths));}catch(_error){}
}
function finishPanelResize(commit) {
  if(!panelDrag)return;
  const drag=panelDrag;panelDrag=null;
  if(!commit){panelWidths={...drag.previous};panelFixed=drag.fixed;}
  else if(panelWidths.explorer!==drag.previous.explorer||panelWidths.inspector!==drag.previous.inspector)savePanelWidths();
  document.getElementById('main').classList.remove('panel-resizing');
  try{drag.handle.releasePointerCapture?.(drag.pointerId);}catch(_error){}
}
function syncPanelSizes() {
  const main=document.getElementById('main'),measured=main.getBoundingClientRect().width;
  const top=document.getElementById('viewport').getBoundingClientRect().top-main.getBoundingClientRect().top;main.style.setProperty('--inspector-overlay-top',(Number.isFinite(top)?Math.max(0,top):0)+'px');
  const width=Number.isFinite(measured)&&measured>0?measured:window.innerWidth,compact=window.innerWidth<=1100;
  if(panelDrag&&(panelDrag.compact!==compact||panelDrag.containerWidth!==width||main.classList.contains(panelDrag.panel+'-closed')))finishPanelResize(false);
  if(compact&&!main.classList.contains('explorer-closed')&&!main.classList.contains('inspector-closed')){
    const close=ui.selection?'explorer':'inspector';main.classList.add(close+'-closed');
    document.getElementById('toggle-'+close).setAttribute('aria-expanded','false');
    if(panelDrag?.panel===close)finishPanelResize(false);
  }
  const names=Object.keys(PANEL_SIZES),open=Object.fromEntries(names.map(name=>[name,!main.classList.contains(name+'-closed')]));
  const visible=names.filter(name=>open[name]),reserved=visible.filter(name=>name!=='inspector'||inspectorPinned()),signature=[width,compact,inspectorPinned(),...visible].join('|');
  if(panelFixed?.signature!==signature)panelFixed=null;
  const budget=Math.max(0,width-360-reserved.length*6),minimum={},effective={};
  const minimumSum=reserved.reduce((sum,name)=>sum+PANEL_SIZES[name].min,0);
  for(const name of names){
    const config=PANEL_SIZES[name],available=compact||name==='inspector'&&!inspectorPinned()?Math.max(0,width-48):config.max;
    minimum[name]=compact?Math.min(config.min,available):reserved.includes(name)&&minimumSum>budget?config.min*budget/minimumSum:config.min;
    effective[name]=Math.max(minimum[name],Math.min(config.max,available,panelWidths[name]));
  }
  if(!compact){
    let excess=reserved.reduce((sum,name)=>sum+effective[name],0)-budget;
    for(let pass=0;excess>0.001&&pass<names.length;pass++){
      const adjustable=reserved.filter(name=>effective[name]>minimum[name]);
      if(!adjustable.length)break;
      const share=excess/adjustable.length;
      for(const name of adjustable){const reduction=Math.min(share,effective[name]-minimum[name]);effective[name]-=reduction;excess-=reduction;}
    }
  }
  // An explicit resize keeps the other panel visually still. Its saved wider
  // preference is retained; a changed window/visibility re-evaluates both.
  if(panelFixed)for(const name of visible)effective[name]=panelFixed.widths[name];
  const state={compact,containerWidth:width,signature};
  for(const name of names){
    const config=PANEL_SIZES[name],other=name==='explorer'?'inspector':'explorer';
    effective[name]=Math.floor(effective[name]);
    const max=compact||name==='inspector'&&!inspectorPinned()?Math.min(config.max,Math.max(0,width-48)):open[name]?Math.min(config.max,Math.max(0,budget-(reserved.includes(other)?Math.floor(effective[other]):0))):config.max;
    state[name]={width:effective[name],min:Math.floor(minimum[name]),max:Math.floor(max),open:open[name]};
    main.style.setProperty('--'+name+'-width',effective[name]+'px');
    const handle=document.getElementById(name+'-resizer');
    handle.hidden=!open[name];handle.setAttribute('tabindex',open[name]?'0':'-1');
    document.getElementById('toggle-'+name).setAttribute('aria-expanded',String(open[name]));
    handle.setAttribute('aria-valuemin',String(state[name].min));handle.setAttribute('aria-valuemax',String(state[name].max));
    handle.setAttribute('aria-valuenow',String(effective[name]));handle.setAttribute('aria-valuetext',t('{count} pixels','{count} Pixel',{count:effective[name]}));
  }
  syncInspectorControls();return state;
}
function changePanelWidth(panel, requested, state=syncPanelSizes()) {
  const bounds=state[panel],value=Math.max(bounds.min,Math.min(bounds.max,Math.round(requested)));
  panelWidths[panel]=value;
  panelFixed={signature:state.signature,widths:{explorer:state.explorer.width,inspector:state.inspector.width,[panel]:value}};
  setWidth();
}
function movePanelResize(event) {
  if(!panelDrag||event.pointerId!==panelDrag.pointerId||!Number.isFinite(event.clientX))return;
  const state=syncPanelSizes();if(!panelDrag)return;
  const delta=(event.clientX-panelDrag.startX)*(panelDrag.panel==='explorer'?1:-1);
  const bounds=state[panelDrag.panel],value=Math.max(bounds.min,Math.min(bounds.max,Math.round(panelDrag.startWidth+delta)));
  if(value===panelDrag.startWidth){panelWidths={...panelDrag.previous};panelFixed=panelDrag.fixed;setWidth();}
  else if(value!==bounds.width)changePanelWidth(panelDrag.panel,value,state);
  event.preventDefault();
}
function initPanelResize() {
  if(panelResizeInitialized){syncPanelSizes();return;}
  panelResizeInitialized=true;
  initInspectorOverlay();
  try{
    const saved=JSON.parse(localStorage.getItem(PANEL_WIDTH_KEY));
    if(saved&&typeof saved==='object'&&!Array.isArray(saved))for(const name of Object.keys(PANEL_SIZES)){
      const value=saved[name],config=PANEL_SIZES[name];
      if(typeof value==='number'&&Number.isFinite(value)&&value>0)panelWidths[name]=Math.max(config.min,Math.min(config.max,value));
    }
  }catch(_error){}
  for(const name of Object.keys(PANEL_SIZES)){
    const handle=document.getElementById(name+'-resizer');
    handle.addEventListener('pointerdown',event=>{
      if(event.button!==0||!Number.isFinite(event.clientX)||panelDrag)return;
      const state=syncPanelSizes();if(!state[name].open)return;
      panelDrag={panel:name,handle,pointerId:event.pointerId,startX:event.clientX,startWidth:state[name].width,previous:{...panelWidths},fixed:panelFixed,compact:state.compact,containerWidth:state.containerWidth};
      document.getElementById('main').classList.add('panel-resizing');handle.focus();
      try{handle.setPointerCapture?.(event.pointerId);}catch(_error){}
      event.preventDefault();
    });
    handle.addEventListener('lostpointercapture',event=>{
      if(panelDrag&&event.pointerId===panelDrag.pointerId){finishPanelResize(false);setWidth();}
    });
    handle.addEventListener('keydown',event=>{
      if(event.key==='Escape'&&panelDrag){finishPanelResize(false);setWidth();event.preventDefault();event.stopPropagation();return;}
      if(!['ArrowLeft','ArrowRight','Home','End'].includes(event.key))return;
      if(panelDrag)finishPanelResize(false);
      const state=syncPanelSizes();if(!state[name].open)return;
      const step=event.shiftKey?48:16,direction=(event.key==='ArrowRight'?1:-1)*(name==='explorer'?1:-1);
      const requested=event.key==='Home'?state[name].min:event.key==='End'?state[name].max:state[name].width+direction*step;
      const value=Math.max(state[name].min,Math.min(state[name].max,requested));
      if(value!==state[name].width){changePanelWidth(name,value,state);savePanelWidths();}
      event.preventDefault();event.stopPropagation();
    });
    handle.addEventListener('dblclick',()=>{
      if(panelDrag)finishPanelResize(false);
      const state=syncPanelSizes();if(!state[name].open)return;
      changePanelWidth(name,PANEL_SIZES[name].initial,state);
      panelWidths[name]=PANEL_SIZES[name].initial;savePanelWidths();
    });
  }
  document.addEventListener('pointermove',movePanelResize);
  document.addEventListener('pointerup',event=>{
    if(panelDrag&&event.pointerId===panelDrag.pointerId){movePanelResize(event);finishPanelResize(true);setWidth();}
  });
  document.addEventListener('pointercancel',event=>{
    if(panelDrag&&event.pointerId===panelDrag.pointerId){finishPanelResize(false);setWidth();}
  });
  window.addEventListener('blur',()=>{if(panelDrag){finishPanelResize(false);setWidth();}});
  syncPanelSizes();
}

// Only an explicitly pinned inspector consumes a grid track.
const inspectorUi={bar:null,title:null,pin:null,close:null,previousFocus:null};
function inspectorPinned(){return document.getElementById('main').classList.contains('inspector-pinned')&&window.innerWidth>1100;}
function inspectorIsOpen(){return !document.getElementById('main').classList.contains('inspector-closed');}
function syncInspectorControls(){
  const action=document.getElementById('toggle-inspector'),panel=document.getElementById('inspector');
  action.textContent=t('Details','Details');action.setAttribute('data-i18n-de','Details');action.setAttribute('aria-expanded',String(inspectorIsOpen()));
  action.setAttribute('aria-keyshortcuts','Enter');panel.setAttribute('aria-label',t('Selection details','Details zur Auswahl'));
  panel.setAttribute('data-i18n-aria-label-de','Details zur Auswahl');
  document.getElementById('inspector-resizer').setAttribute('aria-label',t('Resize details','Details-Breite ändern'));
  panel.setAttribute('role',inspectorPinned()?'complementary':'dialog');panel.setAttribute('aria-modal','false');
  if(!inspectorUi.bar)return;
  inspectorUi.title.textContent=t('Details','Details');
  inspectorUi.pin.textContent=inspectorPinned()?t('Unpin','Lösen'):t('Pin','Anheften');
  inspectorUi.pin.setAttribute('aria-pressed',String(inspectorPinned()));inspectorUi.pin.disabled=window.innerWidth<=1100;
  inspectorUi.pin.setAttribute('title',inspectorPinned()?t('Float above the graph without reserving space.','Über dem Graphen anzeigen, ohne Platz zu reservieren.'):t('Reserve a column beside the graph.','Eine Spalte neben dem Graphen reservieren.'));
  inspectorUi.close.setAttribute('aria-label',t('Close details (Esc)','Details schließen (Esc)'));
}
function selectionFocusRestorer(){
  const active=document.activeElement,panel=document.getElementById('inspector');
  if(active&&panel.contains(active))return ()=>{if(inspectorIsOpen())document.getElementById('sidebar').focus({preventScroll:true});};
  for(const attr of ['data-tree-source','data-tree-relation','data-tree-edge','data-service-node','data-service-edge','data-node']){
    const value=active?.getAttribute?.(attr);if(value==null)continue;
    const container=document.getElementById(attr==='data-node'?'module-list':attr.startsWith('data-service')?'services-tree':'package-tree'),selector=active.tagName.toLowerCase()+'['+attr+']';
    return ()=>{[...container.querySelectorAll(selector)].find(node=>node.getAttribute(attr)===value)?.focus({preventScroll:true});};
  }
  return ()=>{};
}
function setInspectorOpen(open,focus=false){
  const main=document.getElementById('main'),panel=document.getElementById('inspector');
  if(panelDrag?.panel==='inspector')finishPanelResize(false);
  if(open&&!inspectorIsOpen())inspectorUi.previousFocus=document.activeElement;
  main.classList.toggle('inspector-closed',!open);
  if(open&&window.innerWidth<=1100){main.classList.add('explorer-closed');document.getElementById('toggle-explorer').setAttribute('aria-expanded','false');}
  hideMetricTooltip(panel);hideContextTooltip();
  if(inspectorPinned())setWidth();else syncPanelSizes();
  if(open){renderSidebar();if(focus)document.getElementById('sidebar').focus({preventScroll:true});}
  else if(focus){
    const current=document.activeElement;if(current?.isConnected&&current!==document.body&&!panel.contains(current))return;
    const previous=inspectorUi.previousFocus;
    if(previous?.isConnected&&!panel.contains(previous))previous.focus({preventScroll:true});
    else document.getElementById('toggle-inspector').focus({preventScroll:true});
  }
}
function pinInspector(){
  if(window.innerWidth<=1100)return;
  if(panelDrag)finishPanelResize(false);
  document.getElementById('main').classList.toggle('inspector-pinned');
  setWidth();syncInspectorControls();
}
function initInspectorOverlay(){
  if(inspectorUi.bar)return;
  const main=document.getElementById('main'),panel=document.getElementById('inspector');
  main.classList.add('inspector-closed');
  const bar=element('div',undefined,'inspector-toolbar'),title=element('strong'),pin=button('',pinInspector,'btn btn-quiet'),close=button('×',()=>setInspectorOpen(false,true),'btn btn-quiet');
  pin.setAttribute('data-inspector-action','pin');close.setAttribute('data-inspector-action','close');
  bar.appendChild(title);bar.appendChild(pin);bar.appendChild(close);panel.appendChild(bar);
  const style=element('style',[
    '#main:not(.inspector-pinned){--inspector-track:0px;--inspector-divider:0px}',
    '#main #inspector{display:flex;flex-direction:column;padding:0;box-sizing:border-box;overflow:hidden}',
    '#inspector>.panel-heading{display:none}',
    '#inspector .inspector-toolbar{order:-1;display:flex;align-items:center;gap:8px;flex:0 0 auto;padding:10px 12px;border-bottom:1px solid var(--border);background:var(--panel)}',
    '.inspector-toolbar strong{flex:1;font-size:14px}.inspector-toolbar button{min-height:34px}',
    '#inspector #sidebar{flex:1;min-height:0;overflow:auto;padding:16px;overscroll-behavior:contain}',
    '#main:not(.inspector-pinned) #inspector{position:absolute;top:var(--inspector-overlay-top,0px);bottom:0;right:0;width:var(--inspector-width);z-index:8;border-left:1px solid var(--border);box-shadow:-8px 0 28px #0002}',
    '#main:not(.inspector-pinned) #inspector-resizer{position:absolute;top:var(--inspector-overlay-top,0px);bottom:0;right:var(--inspector-width);width:6px;z-index:9}',
    '#main.inspector-closed #inspector,.report-workspace #main #inspector,.report-workspace #main #inspector-resizer{display:none}',
    '@media(max-width:600px){#main #inspector,#main #inspector-resizer{top:0!important}}',
    '@media(max-width:1100px){#main.inspector-pinned #inspector{position:absolute;top:var(--inspector-overlay-top,0px);bottom:0;right:0;width:var(--inspector-width);z-index:8}#main.inspector-pinned #inspector-resizer{top:var(--inspector-overlay-top,0px);z-index:9}}'
  ].join('\n'));bar.appendChild(style);Object.assign(inspectorUi,{bar,title,pin,close});
  document.getElementById('graph').setAttribute('tabindex','0');syncInspectorControls();
}
