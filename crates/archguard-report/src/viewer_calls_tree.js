'use strict';
// A source-call outline, independent of the simulated import graph. Its scope
// precedes the identity filter so selecting one method never hides its siblings.
const callTreeUi={expanded:new Set(),limit:200,model:null,relations:null,relation:null,scope:null};
function callMatchesElement(call) {
  const selected=callUi.element;
  return !selected||(selected.kind==='class'?(call.from_class===selected.id||call.to_class===selected.id):(call.from_method===selected.id||call.to_method===selected.id));
}
function callTreeKey(kind,id) { return JSON.stringify([kind,id]); }
function buildCallTree(relations) {
  const nodes=new Map(),roots=[];let missingMethods=0;
  function add(kind,id,parent,properties={}) {
    const key=callTreeKey(kind,id);
    if(!nodes.has(key)){
      const node={key,kind,id,parent,children:[],count:0,depth:parent?nodes.get(parent).depth+1:0,...properties};
      nodes.set(key,node);(parent?nodes.get(parent).children:roots).push(key);
    }
    return nodes.get(key);
  }
  function addClass(id) {
    const cls=callClasses.get(id);if(!cls)return null;
    const existing=nodes.get(callTreeKey('class',id));if(existing)return existing;
    const language=callClassLanguage(cls),root=add('language',language,null);
    let parent=root.key,path='';
    for(const part of cls.package.split('/').filter(Boolean)){
      path=path?path+'/'+part:part;
      parent=add('package',JSON.stringify([language,path]),parent,{package:path,label:part}).key;
    }
    if(!cls.package)parent=add('package',JSON.stringify([language,'']),parent,{package:'',label:''}).key;
    const prefix=cls.package.replaceAll('/','.')+'.';
    return add('class',id,parent,{label:cls.name.startsWith(prefix)?cls.name.slice(prefix.length):cls.name,cls});
  }
  for(const relation of relations){
    for(const call of relation.calls){
      const from=addClass(call.from_class),to=addClass(call.to_class);
      for(const node of new Set([from,to]))if(node)node.count++;
      const seen=new Set();
      for(const [id,owner] of [[call.from_method,from],[call.to_method,to]]){
        const method=callMethods.get(id);if(!owner||!method||method.class_id!==owner.id||seen.has(id))continue;
        seen.add(id);add('method',id,owner.key,{label:method.signature,method}).count++;
      }
      if(!callMethods.has(call.from_method)||(call.resolution==='project'&&!callMethods.has(call.to_method)))missingMethods++;
    }
    if(relation.resolution!=='project'){
      const group=add('group',relation.resolution,null);
      add(relation.resolution,relation.key,group.key,{label:callClassLabel(relation.from)+' → '+(relation.to?callClassLabel(relation.to):relation.label),relation:relation.key,count:relation.calls.length});
    }
  }
  const sort=(a,b)=>compareNames(nodes.get(a).label??nodes.get(a).id,nodes.get(b).label??nodes.get(b).id)||compareNames(a,b);
  roots.sort(sort);for(const node of nodes.values())node.children.sort(sort);
  return {nodes,roots,missingMethods};
}
function callTreeOpenPath(model,kind,id) {
  let node=model.nodes.get(callTreeKey(kind,id));
  while(node){if(node.children.length)callTreeUi.expanded.add(node.key);node=model.nodes.get(node.parent);}
}
function callTreeRows(model) {
  const rows=[],stack=model.roots.slice().reverse();
  while(stack.length){const node=model.nodes.get(stack.pop());rows.push(node);if(callTreeUi.expanded.has(node.key))for(let i=node.children.length-1;i>=0;i--)stack.push(node.children[i]);}
  return rows;
}
function selectCallTreeElement(kind,id) {
  callUi.element={kind,id};callUi.limit=100;callUi.siteLimit=100;renderCalls();
}
function renderCallTree(relations,selected) {
  const root=document.getElementById('calls-tree'),scroll=root.scrollTop,active=document.activeElement;
  const focused=root.contains(active)?{label:active.getAttribute('data-call-tree-label'),toggle:active.getAttribute('data-call-toggle')}:null;
  if(callTreeUi.relations!==relations){
    callTreeUi.model=buildCallTree(relations);callTreeUi.relations=relations;
    if(callTreeUi.scope!==callRelationsCache.scope){callTreeUi.limit=200;callTreeUi.relation=null;}
    callTreeUi.scope=callRelationsCache.scope;
  }
  const model=callTreeUi.model;
  if(selected&&callTreeUi.relation!==selected.key){
    callTreeOpenPath(model,'class',selected.from);callTreeOpenPath(model,'class',selected.to);
    if(selected.resolution!=='project')callTreeOpenPath(model,'group',selected.resolution);
  }
  callTreeUi.relation=selected?.key??null;
  const visible=callTreeRows(model),fromMethods=new Set(selected?.calls.map(call=>call.from_method)),toMethods=new Set(selected?.calls.map(call=>call.to_method));
  // Reserve rows for the selected endpoints and their real ancestors. A large
  // earlier method branch must not push the corresponding target out of view.
  const visibleKeys=new Set(visible.map(node=>node.key)),shown=new Set();
  for(const key of [callTreeKey('class',selected?.from),callTreeKey('class',selected?.to),callTreeKey(selected?.resolution,selected?.key),callTreeKey(callUi.element?.kind,callUi.element?.id)]){
    let node=model.nodes.get(key);
    while(node){if(visibleKeys.has(node.key))shown.add(node.key);node=model.nodes.get(node.parent);}
  }
  for(const node of visible){if(shown.size>=callTreeUi.limit)break;shown.add(node.key);}
  root.textContent='';
  const focus=callUi.element;
  const selectedLabel=focus?(focus.kind==='class'?callClassLabel(focus.id):callMethodLabel(focus.id)):null;
  let context=t('{shown} / {total} expanded tree rows · all matching elements remain available','{shown} / {total} aufgeklappte Baumzeilen · alle passenden Elemente bleiben erreichbar',{shown:shown.size,total:visible.length});
  if(focus)context+=' · '+t('Element filter: {name}','Elementfilter: {name}',{name:selectedLabel});
  if(model.missingMethods)context+=' · '+t('{count} call sites without complete method identities (for example, initializers).','{count} Aufrufstellen ohne vollständige Methodenidentität (zum Beispiel Initialisierer).',{count:model.missingMethods});
  const contextNode=document.getElementById('calls-tree-context');
  contextNode.textContent=t('{shown} / {total} rows','{shown} / {total} Zeilen',{shown:shown.size,total:visible.length})+(focus?' · '+t('Element: {name}','Element: {name}',{name:selectedLabel}):'')+(model.missingMethods?' · '+t('{count} sites without method identity','{count} Stellen ohne Methodenidentität',{count:model.missingMethods}):'');
  explainContext(contextNode,context);
  document.getElementById('calls-tree-clear').disabled=!focus;
  for(const id of ['calls-tree-expand','calls-tree-collapse'])document.getElementById(id).disabled=!model.nodes.size;
  let skipped=0;
  for(const node of visible){
    if(!shown.has(node.key)){skipped++;continue;}
    if(skipped){root.appendChild(element('p',t('{count} additional rows · load more below','{count} weitere Zeilen · unten nachladen',{count:skipped}),'muted'));skipped=0;}
    const row=element('div',undefined,'call-tree-row');row.setAttribute('data-call-kind',node.kind);row.style.setProperty('--call-tree-depth',node.depth);
    if(node.kind==='package')row.setAttribute('data-call-package',node.package);
    const from=node.kind==='class'?selected?.from===node.id:node.kind==='method'&&fromMethods.has(node.id),to=node.kind==='class'?selected?.to===node.id:node.kind==='method'&&toMethods.has(node.id);
    const role=from&&to?'both':from?'caller':to?'target':null;if(role)row.setAttribute('data-call-role',role);
    const label=node.kind==='language'?callLanguageLabel(node.id):node.kind==='group'?(node.id==='external'?t('External targets · no project package mapping','Externe Ziele · keine Projekt-Paketzuordnung'):t('Unresolved targets · no package mapping','Unaufgelöste Ziele · keine Paketzuordnung')):node.kind==='package'?(node.label||t('(default package)','(Standardpaket)')):node.label;
    const expandable=node.children.length>0,expanded=callTreeUi.expanded.has(node.key);
    const toggle=()=>{if(callTreeUi.expanded.has(node.key))callTreeUi.expanded.delete(node.key);else callTreeUi.expanded.add(node.key);renderCallTree(relations,selected);};
    if(expandable){
      const control=button(expanded?'−':'+',toggle,'btn call-tree-toggle');control.setAttribute('aria-expanded',String(expanded));control.setAttribute('data-call-toggle',node.key);
      control.setAttribute('aria-label',(expanded?t('Collapse ','Zuklappen: '):t('Expand ','Aufklappen: '))+label);row.appendChild(control);
    }else row.appendChild(element('span','','call-tree-toggle'));
    const identity=node.kind==='class'||node.kind==='method';
    const buttonLabel=(node.kind==='package'?'▣ ':node.kind==='class'?'◇ ':node.kind==='method'?'ƒ ':'')+label;
    const action=button(buttonLabel,()=>{
      if(identity)selectCallTreeElement(node.kind,node.id);
      else if(node.relation){callUi.element=null;callUi.relation=node.relation;callUi.siteLimit=100;renderCalls();}
      else toggle();
    },'btn call-tree-label');
    action.setAttribute('data-call-tree-label',node.key);
    if(identity){action.setAttribute('data-call-element',JSON.stringify({kind:node.kind,id:node.id}));action.setAttribute('aria-pressed',String(focus?.kind===node.kind&&focus.id===node.id));}
    else if(expandable)action.setAttribute('aria-expanded',String(expanded));
    else if(node.relation){action.setAttribute('data-call-tree-relation',node.relation);action.setAttribute('aria-pressed',String(selected?.key===node.relation));}
    action.title=node.cls?node.cls.name+'\n'+node.cls.file+':'+node.cls.line+'\n'+node.id:node.method?callClassLabel(node.method.class_id)+'#'+node.label+'\n'+node.method.file+':'+node.method.line:node.package||label;
    row.appendChild(action);
    if(node.cls?.kind==='module'){
      const badge=element('small',t('module','Modul'),'call-tree-meta');badge.setAttribute('data-call-owner-kind','module');
      badge.title=t('Groups top-level functions and initializers; not a declared class.','Gruppiert freie Funktionen und Initialisierer; keine deklarierte Klasse.');row.appendChild(badge);
    }
    if(isCallCycleMember(node.kind,node.id)){
      row.setAttribute('data-call-cycle-member','true');
      const badge=element('span','↻','call-tree-cycle');badge.title=t('Member of the selected cyclic group','Element der gewählten zyklischen Gruppe');badge.setAttribute('aria-label',badge.title);row.appendChild(badge);
    }
    if(role)row.appendChild(element('small',role==='both'?t('Caller / target','Aufrufer / Ziel'):role==='caller'?t('Caller','Aufrufer'):t('Target','Ziel'),'call-tree-meta'));
    if(node.count){const count=element('small',String(node.count),'call-tree-meta');count.title=t('{count} call sites before the element filter','{count} Aufrufstellen vor dem Elementfilter',{count:node.count});row.appendChild(count);}
    root.appendChild(row);
  }
  if(!visible.length)root.appendChild(element('p',t('No call elements match the current scope and filters.','Keine Aufrufelemente passen zum aktuellen Umfang und den Filtern.'),'empty'));
  if(visible.length>callTreeUi.limit){const more=button(t('Show next 200 tree rows','Nächste 200 Baumzeilen anzeigen'),()=>{callTreeUi.limit+=200;renderCallTree(relations,selected);});more.setAttribute('data-call-tree-more','true');root.appendChild(more);}
  root.scrollTop=scroll;
  if(focused){const attribute=focused.label?'data-call-tree-label':'data-call-toggle',value=focused.label||focused.toggle;if(value)root.querySelectorAll('button['+attribute+']').forEach(button=>{if(button.getAttribute(attribute)===value)button.focus();});}
}
function expandCallTree(expand) {
  if(!callTreeUi.model)return;
  callTreeUi.expanded.clear();if(expand)for(const node of callTreeUi.model.nodes.values())if(node.children.length)callTreeUi.expanded.add(node.key);
  callTreeUi.limit=200;renderCallTree(activeCallRelations({ignoreElement:true}),activeCallRelations().find(relation=>relation.key===callUi.relation));
}
