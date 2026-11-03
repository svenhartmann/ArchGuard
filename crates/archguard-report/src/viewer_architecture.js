'use strict';
// The declared component model is separate from the navigable import graph.
const architectureUi={onlyViolations:false,query:'',limit:100,selected:null,evidenceLimit:20,componentLimit:100,ambiguousLimit:50};
function architectureModel() { return data.insights?.architecture?.observed?.target_model||data.insights?.review_session?.report?.architecture?.observed?.target_model||null; }
function architectureRelationKey(edge) { return JSON.stringify([edge.from,edge.to]); }
function architectureViolates(edge) { return !edge.allowed||!!edge.private_accesses?.length; }
function architectureRelations() {
  const model=architectureModel();if(!model)return [];
  const rows=new Map((model.allowed_dependencies||[]).map(edge=>[architectureRelationKey(edge),{...edge,allowed:true,source_edges:0,import_references:0,count_exact:true,evidence:[],evidence_total:0}]));
  for(const edge of model.dependencies||[])rows.set(architectureRelationKey(edge),edge);
  const query=architectureUi.query.trim().toLowerCase();
  return [...rows.values()].filter(edge=>(!architectureUi.onlyViolations||architectureViolates(edge))&&(edge.from+' '+edge.to).toLowerCase().includes(query))
    .sort((a,b)=>Number(architectureViolates(b))-Number(architectureViolates(a))||compareNames(a.from,b.from)||compareNames(a.to,b.to));
}
function architectureState(edge) {
  if(edge.allowed&&edge.private_accesses?.length)return t('Allowed direction · Private access','Erlaubte Richtung · Privater Zugriff');
  return edge.allowed?(edge.source_edges?t('Allowed','Erlaubt'):t('Allowed · not observed','Erlaubt · nicht beobachtet')):t('Forbidden','Nicht erlaubt');
}
function architectureSection(parent,title) {
  const section=element('section',undefined,'architecture-section');section.appendChild(element('h3',title));parent.appendChild(section);return section;
}
function architectureNodes(parent,names) {
  let count=0;
  const list=element('div');parent.appendChild(list);
  const more=button(t('Show more files','Weitere Dateien anzeigen'),append);parent.appendChild(more);
  function append() {
    const end=Math.min(count+50,names.length);
    while(count<end){const name=names[count++],row=element('div',undefined,'mono'),source=nodeByName(data,name),current=source&&nodeById(work,source.id);
      if(current){
        const label=current.name===name?name:name+' · '+t('simulated: {name}','simuliert: {name}',{name:current.name});
        row.appendChild(button(label,()=>{const target=nodeById(work,source.id);if(target){showTab('graph');revealNode(target.name);}},'btn btn-secondary'));
      }
      else row.textContent=name;
      list.appendChild(row);
    }
    more.hidden=count===names.length;
  }
  append();
}
function renderArchitecture() {
  let root=document.getElementById('architecture-content');hideMetricTooltip(root);root.textContent='';
  document.getElementById('tab-architecture').setAttribute('aria-label',t('Architecture documentation','Architekturdokumentation'));
  renderArc42Documentation(root);
  const model=architectureModel();
  if(!model){
    const setup=element('details',undefined,'context-note');setup.setAttribute('data-architecture-setup','true');setup.appendChild(element('summary',t('Optional: compare with a target model','Optional: mit einem Sollmodell vergleichen')));root.appendChild(setup);
    setup.appendChild(element('p',t('Define logical components and allowed dependencies under architecture.model in archguard.toml. Components can span several packages; their mapping is checked against the complete source dependency graph.','Definiere logische Komponenten und erlaubte Abhängigkeiten unter architecture.model in archguard.toml. Komponenten können mehrere Pakete umfassen; ihre Zuordnung wird am vollständigen Quell-Abhängigkeitsgraphen geprüft.')));
    return;
  }
  const target=element('details',undefined,'architecture-section');target.setAttribute('data-architecture-target','true');
  target.open=architectureUi.targetOpen??!data.insights?.arc42?.chapters?.length;target.addEventListener('toggle',()=>{architectureUi.targetOpen=target.open;});
  target.appendChild(element('summary',t('Target architecture','Sollarchitektur')+' · '+model.name));root.appendChild(target);root=target;
  root.appendChild(element('h2',model.name||t('Target architecture','Sollarchitektur')));
  const summary=element('p');summary.setAttribute('data-architecture-summary','true');
  summary.textContent=t('{mapped} / {total} files assigned · {violations} forbidden directions · mapping {coverage}','{mapped} / {total} Dateien zugeordnet · {violations} verbotene Richtungen · Zuordnung {coverage}',{mapped:model.mapped_nodes,total:model.source_nodes,violations:model.violating_dependencies,coverage:model.status==='complete'?t('complete','vollständig'):t('partial','teilweise')});
  root.appendChild(summary);
  root.appendChild(element('p',t('{interfaces} interface violations · {coupling} coupling limit violations','{interfaces} Schnittstellenverstöße · {coupling} verletzte Kopplungsgrenzen',{interfaces:model.violating_interfaces||0,coupling:model.coupling_violations||0})));
  summary.appendChild(contextHelp(t('Architecture assessment','Architekturbewertung'),t('Declared target and original source-dependency observations. Allowed directions need not be used. Coverage and policy conformance are separate; top KPIs describe the package view.','Deklarierte Sollarchitektur und Beobachtungen aus den ursprünglichen Quellabhängigkeiten. Erlaubte Richtungen müssen nicht verwendet werden. Abdeckung und Regelkonformität sind getrennt; die oberen Kennzahlen beschreiben die Paketansicht.')));
  if(ops.length)root.appendChild(element('p',t('Simulation is active. These target-model findings still describe the original report; rescan changed source to evaluate conformance.','Eine Simulation ist aktiv. Diese Sollmodell-Befunde beschreiben weiterhin den ursprünglichen Bericht; geänderte Quellen für eine Konformitätsprüfung erneut analysieren.'),'muted'));
  const toolbar=element('div',undefined,'architecture-controls');
  const filter=button(architectureUi.onlyViolations?t('Show all directions','Alle Richtungen anzeigen'):t('Only violations','Nur Verstöße'),()=>{architectureUi.onlyViolations=!architectureUi.onlyViolations;architectureUi.limit=100;renderArchitecture();});
  filter.setAttribute('aria-pressed',String(architectureUi.onlyViolations));toolbar.appendChild(filter);
  const search=element('input');search.type='search';search.value=architectureUi.query;search.placeholder=t('Filter components','Komponenten filtern');search.setAttribute('aria-label',search.placeholder);
  search.addEventListener('input',()=>{architectureUi.query=search.value;architectureUi.limit=100;renderArchitecture();const next=document.getElementById('architecture-content').querySelector('input');next.focus();});toolbar.appendChild(search);root.appendChild(toolbar);
  const relations=architectureRelations(),list=architectureSection(root,t('Component dependencies · source → target','Komponentenabhängigkeiten · Quelle → Ziel'));
  list.appendChild(element('p',t('{shown} / {total} directions shown','{shown} / {total} Richtungen angezeigt',{shown:Math.min(relations.length,architectureUi.limit),total:relations.length}),'muted'));
  for(const edge of relations.slice(0,architectureUi.limit)){
    const key=architectureRelationKey(edge),row=button(edge.from+' → '+edge.to+' · '+architectureState(edge)+' · '+edge.import_references+(edge.count_exact?'':'+')+' '+graphEvidenceUnit(),()=>{architectureUi.selected=key;architectureUi.evidenceLimit=20;renderArchitecture();},'btn btn-secondary architecture-relation');
    row.setAttribute('data-architecture-edge',key);row.setAttribute('aria-pressed',String(architectureUi.selected===key));row.setAttribute('data-policy',architectureViolates(edge)?'forbidden':'allowed');list.appendChild(row);
  }
  if(!relations.length)list.appendChild(element('p',t('No directions match this filter.','Keine Richtung entspricht diesem Filter.')));
  if(relations.length>architectureUi.limit)list.appendChild(button(t('Show more relationships','Weitere Beziehungen anzeigen'),()=>{architectureUi.limit+=100;renderArchitecture();}));
  const selected=(model.dependencies||[]).find(edge=>architectureRelationKey(edge)===architectureUi.selected);
  if(selected){
    const evidence=architectureSection(root,selected.from+' → '+selected.to+' · '+architectureState(selected));
    evidence.appendChild(element('p',t('{edges} source connections · {shown} / {total} evidence records shown','{edges} Quellverbindungen · {shown} / {total} Belege angezeigt',{edges:selected.source_edges,shown:Math.min(selected.evidence.length,architectureUi.evidenceLimit),total:selected.evidence_total})));
    for(const ref of selected.evidence.slice(0,architectureUi.evidenceLimit))evidence.appendChild(element('pre',ref.file+':'+ref.line+' '+ref.raw));
    if(selected.evidence.length>architectureUi.evidenceLimit)evidence.appendChild(button(t('Show more evidence','Weitere Belege anzeigen'),()=>{architectureUi.evidenceLimit+=20;renderArchitecture();}));
    if(selected.evidence_total>selected.evidence.length||selected.import_references>selected.evidence_total||!selected.count_exact)evidence.appendChild(element('p',t('Stored evidence is a sample; reference totals are reported separately.','Die gespeicherten Belege sind eine Stichprobe; Referenzsummen werden separat ausgewiesen.'),'muted'));
    if(selected.private_accesses?.length){
      evidence.appendChild(element('h4',t('Private access','Privater Zugriff')));
      evidence.appendChild(element('p',Math.min(selected.private_accesses.length,architectureUi.evidenceLimit)+' / '+selected.private_accesses.length));
      for(const access of selected.private_accesses.slice(0,architectureUi.evidenceLimit)){
        evidence.appendChild(element('p',access.from+' → '+access.to,'mono'));
        for(const ref of access.evidence.slice(0,architectureUi.evidenceLimit))evidence.appendChild(element('pre',ref.file+':'+ref.line+' '+ref.raw));
        if(access.evidence.length>architectureUi.evidenceLimit)evidence.appendChild(element('p',t('More source records are available in the full export.','Weitere Quellbelege stehen im vollständigen Export.')));
      }
      if(selected.private_accesses.length>architectureUi.evidenceLimit)evidence.appendChild(button(t('Show more private accesses','Weitere private Zugriffe anzeigen'),()=>{architectureUi.evidenceLimit+=20;renderArchitecture();}));
    }
  }
  const components=architectureSection(root,t('Logical components and mapping','Logische Komponenten und Zuordnung'));
  for(const component of (model.components||[]).slice(0,architectureUi.componentLimit)){
    const details=element('details');details.setAttribute('data-architecture-component',component.name);const label=element('summary',component.name+' · '+component.node_count+' '+t('files','Dateien'));details.appendChild(label);
    details.appendChild(element('p',(component.include||[]).join(', '),'mono'));
    if(component.public!=null)details.appendChild(element('p',t('Public interface: ','Öffentliche Schnittstelle: ')+(component.public.length?component.public.join(', '):t('closed to other components','für andere Komponenten geschlossen')),'mono'));
    for(const [direction,label,limitLabel] of [['incoming',t('Incoming components: ','Eingehende Komponenten: '),t('limit: ','Grenze: ')],['outgoing',t('Outgoing components: ','Ausgehende Komponenten: '),t('limit: ','Grenze: ')]]){
      const neighbors=component[direction+'_components'];if(!Array.isArray(neighbors))continue;
      const limit=component['max_'+direction+'_components'];
      details.appendChild(element('p',label+neighbors.length+' · '+limitLabel+(limit??'—')+(limit!=null&&neighbors.length>limit?' · '+t('exceeded','überschritten'):'')));
    }
    let loaded=false;details.addEventListener('toggle',()=>{if(details.open&&!loaded){loaded=true;architectureNodes(details,component.nodes||[]);}});
    components.appendChild(details);
  }
  if(model.components.length>architectureUi.componentLimit)components.appendChild(button(t('Show more components','Weitere Komponenten anzeigen'),()=>{architectureUi.componentLimit+=100;renderArchitecture();}));
  if(model.unmapped.length||model.ambiguous.length||model.unclassified_edges||model.empty_components.length){
    const coverage=architectureSection(root,t('Mapping coverage','Abdeckung der Zuordnung'));
    coverage.appendChild(element('p',t('{unmapped} unassigned files · {ambiguous} ambiguous files · {edges} unclassified connections','{unmapped} nicht zugeordnete Dateien · {ambiguous} mehrdeutige Dateien · {edges} nicht klassifizierte Verbindungen',{unmapped:model.unmapped_nodes,ambiguous:model.ambiguous_nodes,edges:model.unclassified_edges})));
    architectureNodes(coverage,model.unmapped);
    for(const mapping of model.ambiguous.slice(0,architectureUi.ambiguousLimit))coverage.appendChild(element('p',mapping.node+' → '+mapping.components.join(', '),'mono'));
    if(model.ambiguous.length>architectureUi.ambiguousLimit)coverage.appendChild(button(t('Show more ambiguous files','Weitere mehrdeutige Dateien anzeigen'),()=>{architectureUi.ambiguousLimit+=50;renderArchitecture();}));
    if(model.empty_components.length)coverage.appendChild(element('p',t('Components without assigned files: ','Komponenten ohne zugeordnete Dateien: ')+model.empty_components.join(', ')));
  }
  if(model.limitations?.length)summary.appendChild(contextHelp(t('Analysis limits','Analysegrenzen'),model.limitations.join('\n')));
}

// Parse the Markdown emitted by the arc42 exporter without an HTML sink or network dependency.
function arc42Inline(parent,text,depth=0) {
  const tokens=/\\([!"#$%&'()*+,\-./:;<=>?@[\]\\^_`{|}~])|`([^`]+)`|\*\*([^*]+)\*\*|\*([^*\n]+)\*|\[([^\]\n]+)\]\(([^\s)]+)\)|&(amp|lt|gt|quot|apos);/g;
  let cursor=0;
  for(const match of text.matchAll(tokens)){
    parent.appendChild(document.createTextNode(text.slice(cursor,match.index)));cursor=match.index+match[0].length;
    if(match[7]){parent.appendChild(document.createTextNode(({amp:'&',lt:'<',gt:'>',quot:'"',apos:"'"})[match[7]]));continue;}
    if(match[1]){parent.appendChild(document.createTextNode(match[1]));continue;}
    if(match[2]){parent.appendChild(element('code',match[2]));continue;}
    const node=element(match[3]?'strong':match[4]?'em':'span');
    if(match[5]&&/^(https?:\/\/|mailto:)/i.test(match[6])){
      const link=element('a');link.setAttribute('href',match[6]);link.setAttribute('rel','noopener noreferrer');arc42Inline(link,match[5],depth+1);parent.appendChild(link);continue;
    }
    if(depth<8)arc42Inline(node,match[3]||match[4]||match[5],depth+1);else node.textContent=match[3]||match[4]||match[5];parent.appendChild(node);
  }
  parent.appendChild(document.createTextNode(text.slice(cursor)));
}
function arc42Cells(line) {
  const cells=[];let cell='',escaped=false;
  for(const ch of line.trim().replace(/^\|/,'').replace(/\|$/,'')){
    if(ch==='|'&&!escaped){cells.push(cell.trim());cell='';}else cell+=ch;
    escaped=ch==='\\'&&!escaped;
  }
  cells.push(cell.trim());return cells;
}
function arc42Diagram(parent,source) {
  const lines=source.trim().split('\n').map(s=>s.trim()).filter(Boolean),nodes=new Map(),edges=[];
  if(lines.shift()!=='flowchart LR'||lines.length>500)return false;
  for(const line of lines){
    let match=line.match(/^([\w]+)\["([^"\n]*)"\]$/);
    if(match){if(nodes.has(match[1]))return false;nodes.set(match[1],{id:match[1],kind:'package',name:match[2].replace(/#(\d+);/g,(_,n)=>{const code=Number(n);return code<=0x10ffff?String.fromCodePoint(code):'�';})});continue;}
    match=line.match(/^(\w+)\s+-->(?:\|(\d+)\|)?\s+(\w+)$/);if(!match)return false;edges.push({from:match[1],to:match[3],count:Number(match[2]||1)});
  }
  if(!nodes.size||edges.some(e=>!nodes.has(e.from)||!nodes.has(e.to)))return false;
  const model={nodes:[...nodes.values()],edges,unit:t('connections','Verbindungen')},cycles=findSccs(model),membership=new Map();cycles.forEach((c,i)=>c.members.forEach(id=>membership.set(id,i)));
  for(const edge of edges)edge.cycle=membership.has(edge.from)&&membership.get(edge.from)===membership.get(edge.to);
  arc42Graph(parent,model,{all:true});return true;
}
function arc42Markdown(parent,markdown) {
  const lines=String(markdown||'').replace(/\r\n?/g,'\n').replace(/<!--[\s\S]*?-->/g,'').split('\n');
  const listItem=line=>line.match(/^(\s*)([-+*]|\d+[.)])\s+(.+)$/);
  const divider=line=>/^\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)+\|?$/.test(line?.trim()||'');
  const block=i=>/^\s*$|^#{1,6}\s|^\s*```|^\s*>|^\s*([-*_])\1{2,}\s*$/.test(lines[i])||listItem(lines[i])||divider(lines[i+1]);
  for(let i=0;i<lines.length;){
    const line=lines[i];if(!line.trim()){i++;continue;}
    const fence=line.match(/^\s*```(\w*)\s*$/);
    if(fence){const code=[];i++;while(i<lines.length&&!/^\s*```\s*$/.test(lines[i]))code.push(lines[i++]);i++;const source=code.join('\n');
      if(fence[1]==='mermaid'&&arc42Diagram(parent,source))continue;
      if(fence[1]==='mermaid')parent.appendChild(element('p',t('Diagram source · this diagram syntax is not supported in the viewer.','Diagrammquelle · diese Diagrammsyntax wird im Viewer nicht unterstützt.'),'muted'));
      const pre=element('pre');pre.appendChild(element('code',source));parent.appendChild(pre);continue;
    }
    const heading=line.match(/^(#{1,6})\s+(.+)$/);
    if(heading){const node=element('h'+Math.max(2,heading[1].length));arc42Inline(node,heading[2]);if(heading[2]==='Offen zu ergänzen')node.className='arc42-open-heading';parent.appendChild(node);i++;continue;}
    if(divider(lines[i+1])){
      const scroll=element('div',undefined,'arc42-table'),table=element('table'),head=element('thead'),body=element('tbody'),header=element('tr'),columns=arc42Cells(line),align=arc42Cells(lines[i+1]);
      scroll.setAttribute('tabindex','0');scroll.setAttribute('role','region');scroll.setAttribute('aria-label',t('Table: ','Tabelle: ')+columns.join(' · '));
      columns.forEach(text=>{const th=element('th');th.setAttribute('scope','col');arc42Inline(th,text);header.appendChild(th);});head.appendChild(header);table.appendChild(head);table.appendChild(body);scroll.appendChild(table);parent.appendChild(scroll);i+=2;
      while(i<lines.length&&lines[i].trim().includes('|')&&lines[i].trim()){
        const row=element('tr'),cells=arc42Cells(lines[i++]);columns.forEach((_,column)=>{const td=element('td');arc42Inline(td,cells[column]||'');if(align[column]?.endsWith(':'))td.style.textAlign=align[column].startsWith(':')?'center':'right';row.appendChild(td);});body.appendChild(row);
      }
      if(!body.children.length){const row=element('tr'),empty=element('td',t('No entries recorded.','Keine Einträge erfasst.'),'muted');empty.setAttribute('colspan',columns.length);row.appendChild(empty);body.appendChild(row);}continue;
    }
    if(listItem(line)){
      const stack=[];
      while(i<lines.length){const match=listItem(lines[i]);if(!match)break;i++;const indent=match[1].length,type=/\d/.test(match[2])?'ol':'ul';
        while(stack.length&&(indent<stack[stack.length-1].indent||indent===stack[stack.length-1].indent&&type!==stack[stack.length-1].type))stack.pop();
        if(!stack.length||indent>stack[stack.length-1].indent){const list=element(type);if(type==='ol')list.setAttribute('start',parseInt(match[2],10));const owner=stack.length?stack[stack.length-1].last:parent;owner.appendChild(list);stack.push({indent,type,list,last:null});}
        const entry=stack[stack.length-1],li=element('li');arc42Inline(li,match[3]);entry.list.appendChild(li);entry.last=li;
      }continue;
    }
    if(/^\s*>/.test(line)){const quote=[];while(i<lines.length&&/^\s*>/.test(lines[i]))quote.push(lines[i++].replace(/^\s*> ?/,''));const node=element('blockquote');arc42Markdown(node,quote.join('\n'));parent.appendChild(node);continue;}
    if(/^\s*([-*_])\1{2,}\s*$/.test(line)){parent.appendChild(element('hr'));i++;continue;}
    const paragraph=[line];i++;while(i<lines.length&&!block(i))paragraph.push(lines[i++]);const node=element('p');arc42Inline(node,paragraph.join('\n'));parent.appendChild(node);
  }
}
function renderArc42Documentation(root) {
  const model=data.insights?.arc42;
  if(!model?.chapters?.length){root.appendChild(element('h2',t('No arc42 documentation in this report','Keine arc42-Dokumentation in diesem Bericht')));root.appendChild(element('p',t('Generate a new report to see the architecture documentation.','Erzeuge einen neuen Bericht, um die Architekturdokumentation anzuzeigen.'),'muted'));return;}
  if(architectureUi.chapter==null&&model.chapters.some(c=>c.number===5))architectureUi.chapter='5';
  const section=element('section',undefined,'arc42-reader');section.setAttribute('data-arc42','true');root.appendChild(section);
  section.appendChild(element('style',`
    .arc42-reader{max-width:1440px;margin:0 auto 28px;min-width:0}
    .arc42-reader header{display:flex;align-items:center;justify-content:space-between;gap:14px;flex-wrap:wrap;margin:4px 0 18px}
    .arc42-reader h2{font-size:23px;letter-spacing:-.025em;margin:0 0 8px}.arc42-reader header p{margin:0;color:var(--muted)}
    .arc42-layout{display:grid;grid-template-columns:236px minmax(0,1fr);gap:22px;align-items:start}
    .arc42-nav{display:grid;gap:3px;position:sticky;top:0;padding:8px;background:var(--panel);border:1px solid var(--border);border-radius:12px}
    .arc42-nav button{text-align:left;display:flex;align-items:baseline;gap:10px;white-space:normal;line-height:1.4;border:0;border-radius:7px;padding:10px;background:transparent;color:var(--text);font:inherit;cursor:pointer}
    .arc42-nav button small{color:var(--muted);font-variant-numeric:tabular-nums;min-width:18px}.arc42-nav button{box-shadow:none}.arc42-nav button:hover{background:var(--panel-2)}
    .arc42-nav button[aria-current='page']{background:var(--panel-2);color:var(--accent);font-weight:650;box-shadow:inset 3px 0 var(--accent)}
    .arc42-page{min-width:0;background:var(--panel);border:1px solid var(--border);border-radius:12px;padding:26px 30px}
    .arc42-article:focus{outline:none}.arc42-article{font-size:14px;line-height:1.7;overflow-wrap:anywhere}.arc42-article h3{margin:28px 0 10px;font-size:17px}.arc42-article h4{margin:22px 0 8px}
    .arc42-article p{margin:10px 0 18px}.arc42-article li{margin:6px 0}.arc42-article ul,.arc42-article ol{padding-left:24px}
    .arc42-article code{padding:2px 5px;background:var(--panel-2);border-radius:4px;font-size:.92em}.arc42-article a{color:var(--accent)}
    .arc42-article blockquote{border-left:3px solid var(--accent);margin:16px 0;padding:1px 16px;background:var(--panel-2)}
    .arc42-reader pre{white-space:pre-wrap;overflow-wrap:anywhere;font-size:12px;line-height:1.6;padding:14px;background:var(--panel-2);border-radius:8px}
    .arc42-table{overflow:auto;max-width:100%;margin:16px 0 24px;border:1px solid var(--border);border-radius:8px}
    .arc42-table table{border-collapse:collapse;width:100%;font-size:13px}.arc42-table th,.arc42-table td{padding:10px 12px;text-align:left;border-bottom:1px solid var(--border);vertical-align:top;min-width:90px;max-width:340px}
    .arc42-table th{background:var(--panel-2);font-size:12px}.arc42-table tr:last-child td{border-bottom:0}
    .arc42-article .arc42-open-heading{border-top:1px solid var(--border);padding-top:22px;color:var(--accent)}
    .arc42-reader .detail-graph{margin:18px 0}.arc42-diagram{max-height:640px;overflow:auto}

    .arc42-visual{margin:22px 0 30px;min-width:0;padding:16px;border:1px solid var(--border);border-radius:10px;background:var(--panel-2)}
    .arc42-visual h3{margin:0 0 12px!important}.arc42-map-controls{display:flex;gap:8px;align-items:center;flex-wrap:wrap}
    .arc42-visual select{max-width:100%;min-width:0;padding:8px;border:1px solid var(--border);background:var(--panel);color:var(--text);border-radius:6px}
    .arc42-map-controls select{flex:1 1 240px;width:240px}.arc42-map-note{font-size:12px;color:var(--muted);margin:10px 0!important}
    .arc42-map{overflow:auto;max-width:100%;max-height:640px;border:1px solid var(--border);background:var(--bg);border-radius:8px}
    .arc42-map svg{display:block;width:100%;min-width:600px;height:auto;margin:0 auto}.arc42-map-kind{fill:var(--muted);font-size:10px;font-family:var(--font)}
    .arc42-map-name{fill:var(--text);font-size:13px;font-weight:650;font-family:var(--font)}.arc42-map-node[role=button]{cursor:pointer}
    .arc42-map-node:focus-visible{outline:2px solid var(--accent);outline-offset:3px}.arc42-map-selection{display:flex;gap:12px;align-items:center;flex-wrap:wrap;padding:12px 0}
    .arc42-relations{margin-top:12px;font-size:12px}.arc42-relations p{padding:7px 0;margin:0!important;border-bottom:1px solid var(--border)}
    @media(max-width:600px){.arc42-visual{padding:10px}.arc42-map svg{min-width:660px}}
    .arc42-source,.arc42-about{margin-top:24px;padding-top:16px;border-top:1px solid var(--border);font-size:12px;color:var(--muted)}
    .arc42-mobile{display:none;max-width:100%;width:100%;padding:10px;border:1px solid var(--border);border-radius:8px;background:var(--panel);color:var(--text)}
    @media(max-width:1050px){.arc42-layout{grid-template-columns:1fr}.arc42-nav{display:none}.arc42-mobile{display:block}.arc42-page{padding:20px}.arc42-reader header{margin-bottom:14px}}
    @media(max-width:600px){.arc42-page{padding:16px}.arc42-reader h2{font-size:20px}.arc42-article:focus{outline:none}.arc42-article{font-size:13px}.arc42-layout{gap:12px}}
  `));
  const header=element('header'),heading=element('div');heading.appendChild(element('h2',t('Architecture documentation','Architekturdokumentation')));heading.appendChild(element('p',t('arc42 · Evidence, decisions and open questions','arc42 · Belege, Entscheidungen und offene Fragen')));header.appendChild(heading);section.appendChild(header);
  const controls=element('div',undefined,'architecture-controls');header.appendChild(controls);
  const download=button(t('Download chapter','Kapitel herunterladen'),()=>downloadArc42(architectureUi.chapter),'btn btn-secondary');download.setAttribute('data-arc42-download','chapter');controls.appendChild(download);
  controls.appendChild(button(t('Download all chapters','Alle Kapitel herunterladen'),()=>downloadArc42(),'btn btn-secondary'));
  const layout=element('div',undefined,'arc42-layout'),nav=element('nav',undefined,'arc42-nav'),select=element('select',undefined,'arc42-mobile');nav.setAttribute('aria-label',t('arc42 chapters','arc42-Kapitel'));select.setAttribute('aria-label',t('arc42 chapter','arc42-Kapitel'));section.appendChild(layout);layout.appendChild(nav);layout.appendChild(select);
  const page=element('div',undefined,'arc42-page'),article=element('article',undefined,'arc42-article');article.setAttribute('tabindex','-1');layout.appendChild(page);page.appendChild(article);
  const source=element('details',undefined,'arc42-source');source.appendChild(element('summary',t('Markdown source','Markdown-Quelltext')));const raw=element('pre');raw.setAttribute('data-arc42-source','true');source.appendChild(raw);page.appendChild(source);
  const about=element('details',undefined,'arc42-about');about.appendChild(element('summary',t('Source and update','Quelle und Aktualisierung')));page.appendChild(about);
  about.appendChild(element('p',t('Generated from the original project evidence. View filters and simulations do not change this document. Add project context in the exported Markdown; archguard doc --arc42 preserves your text outside generated blocks.','Aus den ursprünglichen Projektbelegen erzeugt. Ansichtfilter und Simulationen ändern dieses Dokument nicht. Ergänze Projektkontext im exportierten Markdown; archguard doc --arc42 erhält eigene Texte außerhalb erzeugter Blöcke.')));
  const preamble=String(model.markdown||'').split(/^## \d+\./m)[0];arc42Markdown(about,preamble);
  const buttons=new Map();
  for(const c of model.chapters){const key=String(c.number),control=button('',()=>update(key,true));control.setAttribute('data-arc42-nav',key);control.appendChild(element('small',key.padStart(2,'0')));control.appendChild(element('span',c.title));nav.appendChild(control);buttons.set(key,control);const option=element('option',key+'. '+c.title);option.value=key;select.appendChild(option);}
  function update(number,focus=false){
    const chapter=model.chapters.find(c=>String(c.number)===String(number))||model.chapters[0];architectureUi.chapter=String(chapter.number);select.value=architectureUi.chapter;
    for(const [key,control]of buttons){if(key===architectureUi.chapter)control.setAttribute('aria-current','page');else control.removeAttribute('aria-current');}
    article.textContent='';article.setAttribute('data-arc42-chapter',architectureUi.chapter);const markdown=chapter.markdown||'',start=markdown.search(new RegExp('^## '+chapter.number+'\\.\\s','m'));
    article.appendChild(element('h2',chapter.number+'. '+chapter.title));
    const derived=renderArc42Derived(article,chapter.number);
    let body=start<0?markdown:markdown.slice(start).replace(/^## [^\n]+\n?/,'');
    if(derived&&chapter.number===5)body=body.replace(/\x60\x60\x60mermaid[\s\S]*?\x60\x60\x60/g,'');
    arc42Markdown(article,body);raw.textContent=markdown;source.open=false;
    if(focus){document.getElementById('tab-architecture').scrollTop=0;article.focus({preventScroll:true});}
  }
  select.addEventListener('change',()=>update(select.value,true));update(architectureUi.chapter);
}
function architectureExport(base) {
  const model=architectureModel(),edges=architectureRelations();
  return {...base,patterns:typeof patternReport==='function'?patternReport():null,kind:'architecture',status:model?.status||'unavailable',coverage:'Declared target architecture and original full-source dependency observations; graph simulations do not update this assessment.',
    filters:{...base.filters,only_violations:architectureUi.onlyViolations,query:architectureUi.query},model,
    metrics:{source_files:model?.source_nodes||0,mapped_files:model?.mapped_nodes||0,forbidden_directions:model?.violating_dependencies||0,interface_violations:model?.violating_interfaces||0,coupling_violations:model?.coupling_violations||0,unclassified_connections:model?.unclassified_edges||0},
    nodes:(model?.components||[]).map(component=>({id:component.name,name:component.name})),
    edges:edges.map(edge=>({...edge,kind:!edge.allowed?'forbidden':edge.private_accesses?.length?'private_access':'allowed',count:edge.import_references})),count_unit:data.semantic?'dependency evidence records':'source import references',
    ...(data.semantic?{semantic:data.semantic}:{}),
    notes:['Allowed but unobserved directions are policy permissions, not missing dependencies. Mapping coverage does not establish policy conformance. Original target assessment is separate from dependency-graph simulation.',...(data.semantic?[graphCoverageDescription()]:[]),...(model?.limitations||[])],diagnostics:data.semantic?.diagnostics||[]};
}

// Documentation projections always read the original report, never the simulated work graph.
let arc42GraphCache=null,arc42GraphSequence=0;
function arc42SourceModel(){
  if(arc42GraphCache)return arc42GraphCache;
  const nodes=new Map(data.nodes.map(n=>[String(n.id),n])),byName=new Map(data.nodes.map(n=>[n.name,String(n.id)])),children=new Map(),ancestors=new Map(),files=new Map();
  for(const n of data.nodes){const parent=n.parent==null?null:String(n.parent);if(!children.has(parent))children.set(parent,[]);children.get(parent).push(n);}
  for(const n of data.nodes){const chain=[],seen=new Set();let current=n;while(current&&!seen.has(String(current.id))){seen.add(String(current.id));chain.push(String(current.id));current=nodes.get(String(current.parent));}ancestors.set(String(n.id),chain);if(n.kind==='file')for(const id of chain){if(!files.has(id))files.set(id,[]);files.get(id).push(String(n.id));}}
  const overview=[];
  for(let root of children.get(null)||[]){
    if(root.kind!=='module')continue;
    while((children.get(String(root.id))||[]).length===1&&(children.get(String(root.id))||[])[0].kind==='module')root=children.get(String(root.id))[0];
    const kids=children.get(String(root.id))||[],packages=kids.filter(n=>n.kind==='module');
    overview.push(...(packages.length?packages:[root]));
    if(packages.length&&kids.some(n=>n.kind==='file'))overview.push(root);
  }
  arc42GraphCache={nodes,byName,children,ancestors,files,overview,callEdges:null};return arc42GraphCache;
}
function arc42CallEdges(){
  const model=arc42SourceModel();if(model.callEdges)return model.callEdges;
  const calls=data.calls||{},compact=calls.compact;
  const get=(table,row,key)=>compact?.encoding==='string-pool-v1'&&compact.fields?.[table]?.includes(key)&&row[key]!=null?compact.strings[row[key]]:row[key];
  const classes=new Map((calls.classes||[]).map(row=>[get('classes',row,'id'),model.byName.get(get('classes',row,'graph_node'))])),edges=[];
  for(const row of calls.calls||[]){if(get('calls',row,'resolution')!=='project')continue;const from=classes.get(get('calls',row,'from_class')),to=classes.get(get('calls',row,'to_class'));if(from!=null&&to!=null)edges.push({from,to});}
  model.callEdges=edges;return edges;
}
function arc42PackageProjection(scope=null,kind='dependencies'){
  const model=arc42SourceModel();scope=scope==null?null:String(scope);
  const candidates=scope==null?model.overview:(model.children.get(scope)||[]).filter(n=>n.kind==='module');
  const allNodes=candidates.map(n=>({id:String(n.id),name:n.name,files:0,members:[String(n.id)],kind:'package',drill:String(n.id)})),ids=new Set(allNodes.map(n=>n.id));
  const direct=(scope==null?model.children.get(null):model.children.get(scope))||[];
  if(direct.some(n=>n.kind==='file')){allNodes.push({id:'direct',name:scope==null?t('Files without a package','Dateien ohne Paket'):model.nodes.get(scope).name+' / '+t('direct files','direkte Dateien'),files:0,members:['direct'],kind:'files'});ids.add('direct');}
  const owner=new Map(),counts=new Map();
  for(const n of data.nodes){const id=String(n.id),chain=model.ancestors.get(id)||[];if(scope!=null&&!chain.includes(scope))continue;
    const group=chain.find(p=>ids.has(p))||'direct';if(!ids.has(group))continue;owner.set(id,group);if(n.kind==='file')counts.set(group,(counts.get(group)||0)+1);
  }
  allNodes.forEach(n=>n.files=counts.get(n.id)||0);allNodes.sort((a,b)=>b.files-a.files||compareNames(a.name,b.name));
  const fullEdges=new Map(),outsideFiles=new Set();let originalInternal=0,total=0;
  const sourceEdges=kind==='calls'?arc42CallEdges():data.edges;
  for(const edge of sourceEdges){
    const from=String(edge.from),to=String(edge.to);if(!model.nodes.has(from)||!model.nodes.has(to))continue;
    let a=owner.get(from),b=owner.get(to);if(!a&&!b)continue;
    if(!a){a='outside';outsideFiles.add(from);}if(!b){b='outside';outsideFiles.add(to);}total++;
    if(a===b){originalInternal++;continue;}const key=JSON.stringify([a,b]);
    if(!fullEdges.has(key))fullEdges.set(key,{from:a,to:b,count:0});fullEdges.get(key).count++;
  }
  const outside={id:'outside',name:t('Other project packages','Andere Projektpakete'),files:outsideFiles.size,members:[],kind:'context'};
  const fullNodes=outsideFiles.size?[...allNodes,outside]:allNodes,cycles=findSccs({nodes:fullNodes,edges:[...fullEdges.values()]}),cycleOf=new Map();
  cycles.forEach((c,i)=>c.members.forEach(id=>cycleOf.set(id,i)));
  for(const edge of fullEdges.values())edge.cycle=cycleOf.has(edge.from)&&cycleOf.get(edge.from)===cycleOf.get(edge.to);
  const nodes=fullNodes.map(n=>({...n,cycle:cycleOf.has(n.id)})),edges=[...fullEdges.values()];
  return {nodes,edges,allNodes,allEdges:edges,internal:originalInternal,total,scope,kind,unit:kind==='calls'?t('recorded call sites','erfasste Aufrufstellen'):t('source connections','Quellverbindungen')};
}
function arc42DeploymentProjection(file){
  const services=data.services||{},declared=(services.services||[]).filter(s=>s.compose_file===file&&s.compose_file),ids=new Set(declared.map(s=>s.id));
  return {nodes:declared.map(s=>({id:s.id,name:s.name,kind:'deployment',detail:[...(s.networks||[]),...(s.profiles||[])].join(' · ')||t('Compose service','Compose-Dienst'),members:[s.id]})),edges:(services.dependencies||[]).filter(e=>ids.has(e.from)&&ids.has(e.to)).map(e=>({...e,count:e.evidence?.length||1})),scope:file,unit:t('configuration evidence','Konfigurationsbelege')};
}
function arc42Graph(parent,model,{title,onNode,selected=null,all=false}={}){
  const svgNode=(tag,attrs={},text)=>{const n=document.createElementNS('http://www.w3.org/2000/svg',tag);for(const [key,value]of Object.entries(attrs))n.setAttribute(key,String(value));if(text!=null)n.textContent=text;return n;};
  const box=element('div',undefined,'arc42-map'),svg=svgNode('svg',{'data-arc42-graph':'true',role:'group','aria-label':title||t('Package diagram','Paketdiagramm')});box.setAttribute('tabindex','0');box.appendChild(svg);parent.appendChild(box);
  const nodes=model.nodes,columns=nodes.length<=4?2:3,rows=Math.ceil(nodes.length/columns),width=columns*270+40,height=Math.max(230,rows*148+64),positions=new Map(nodes.map((n,i)=>[n.id,{x:35+(i%columns)*270,y:55+Math.floor(i/columns)*148,w:204,h:98}]));
  svg.style.maxWidth=width+'px';svg.setAttribute('viewBox','0 0 '+width+' '+height);svg.setAttribute('width',width);svg.setAttribute('height',height);
  const edges=model.edges;
  const seq='arc42-arrow-'+(++arc42GraphSequence),defs=svgNode('defs');svg.appendChild(defs);
  for(const [kind,color]of [['normal','var(--accent)'],['cycle','var(--c-cycle)'],['startup','var(--muted)']]){const marker=svgNode('marker',{id:seq+'-'+kind,viewBox:'0 0 10 10',refX:9,refY:5,markerWidth:7,markerHeight:7,orient:'auto'});marker.appendChild(svgNode('path',{d:'M 0 0 L 10 5 L 0 10 Z',fill:color}));defs.appendChild(marker);}
  edges.forEach((edge,i)=>{
    const a=positions.get(edge.from),b=positions.get(edge.to);if(!a||!b)return;
    const kind=edge.kind==='startup'?'startup':edge.cycle?'cycle':'normal',color=kind==='cycle'?'var(--c-cycle)':kind==='startup'?'var(--muted)':'var(--accent)',right=b.x>=a.x,sign=right?1:-1;
    const x1=right?a.x+a.w:a.x,x2=right?b.x:b.x+b.w,y1=a.y+a.h/2-7,y2=b.y+b.h/2+7,lane=16+(i%5)*5;
    const route=a.x===b.x?'M '+(a.x+a.w)+' '+y1+' H '+(a.x+a.w+lane)+' V '+y2+' H '+(b.x+b.w):'M '+x1+' '+y1+' H '+(x1+sign*lane)+' V '+(Math.min(a.y,b.y)-lane)+' H '+(x2-sign*lane)+' V '+y2+' H '+x2;
    const group=svgNode('g',{'data-arc42-edge':JSON.stringify([edge.from,edge.to])}),path=svgNode('path',{d:route,fill:'none',stroke:color,'stroke-width':selected&&(edge.from===selected||edge.to===selected)?2.5:1.5,opacity:selected&&edge.from!==selected&&edge.to!==selected?.25:1,'stroke-dasharray':edge.kind==='startup'?'5 4':'none','marker-end':'url(#'+seq+'-'+kind+')'});
    const names=[nodes.find(n=>n.id===edge.from),nodes.find(n=>n.id===edge.to)];group.appendChild(svgNode('title',{},names.map(n=>n.name).join(' → ')+' · '+edge.count+' '+(model.unit||'')));group.appendChild(path);svg.appendChild(group);
  });
  for(const node of nodes){
    const p=positions.get(node.id),g=svgNode('g',{'data-arc42-node':node.id,role:onNode?'button':'img',tabindex:onNode?0:-1,'aria-label':node.name,'aria-pressed':String(selected===node.id),class:'arc42-map-node'+(selected===node.id?' selected':'')});
    if(node.kind==='package')g.appendChild(svgNode('rect',{x:p.x,y:p.y-10,width:55,height:16,rx:3,fill:'var(--panel-2)',stroke:'var(--border)'}));
    g.appendChild(svgNode('rect',{x:p.x,y:p.y,width:p.w,height:p.h,rx:node.kind==='deployment'?5:2,fill:node.kind==='context'||node.kind==='group'?'var(--panel-2)':'var(--panel)',stroke:selected===node.id?'var(--accent)':node.cycle?'var(--c-cycle)':'var(--border)','stroke-width':selected===node.id?2.5:1.5,'stroke-dasharray':node.kind==='group'?'5 3':'none'}));
    const type=node.kind==='deployment'?t('Deployment node · Docker','Deployment-Knoten · Docker'):node.kind==='group'?t('Grouped packages','Zusammengefasste Pakete'):node.kind==='context'?t('Project context','Projektkontext'):node.kind==='component'?t('Declared component','Deklarierte Komponente'):node.kind==='files'?t('Direct source nodes','Direkte Quellknoten'):t('Source package','Quellpaket');
    g.appendChild(svgNode('text',{x:p.x+12,y:p.y+19,class:'arc42-map-kind'},type));
    const parts=node.name.split('/'),base=parts.pop()||node.name,context=parts.join('/');
    const lines=[];for(let i=0;i<base.length;i+=24)lines.push(base.slice(i,i+24));lines.slice(0,2).forEach((line,i)=>g.appendChild(svgNode('text',{x:p.x+12,y:p.y+42+i*17,class:'arc42-map-name'},line+(i===1&&lines.length>2?'…':''))));
    const detail=node.files!=null?node.files+' '+t('source nodes','Quellknoten'):node.detail||'';
    g.appendChild(svgNode('text',{x:p.x+12,y:p.y+82,class:'arc42-map-kind'},detail.slice(0,30)));
    g.appendChild(svgNode('title',{},node.name));if(node.cycle)g.appendChild(svgNode('circle',{cx:p.x+p.w-12,cy:p.y+17,r:4,fill:'var(--c-cycle)'}));
    if(onNode){g.addEventListener('click',()=>onNode(node));g.addEventListener('keydown',event=>{if(event.key==='Enter'||event.key===' '){event.preventDefault();onNode(node);}});}
    svg.appendChild(g);
  }
  arc42UpgradeDiagram(box,{...model,edges},{title,onNode,selected});
  return edges.length;
}
// arc42 building-block zoom levels are relative to the project overview,
// not to namespace-only directory wrappers.
function arc42BuildingLevels(scope){
  const source=arc42SourceModel(),entries=new Map();
  for(const node of source.overview)entries.set(String(node.id),{node,level:2,parent:null});
  for(const node of source.overview)for(const child of source.children.get(String(node.id))||[])if(child.kind==='module'&&!entries.has(String(child.id)))entries.set(String(child.id),{node:child,level:3,parent:String(node.id)});
  let id=scope==null?null:String(scope);if(id!=null&&!entries.has(id))id=(source.ancestors.get(id)||[]).find(ancestor=>entries.has(ancestor))??null;
  const entry=entries.get(id),path=[];if(entry?.parent)path.push(entries.get(entry.parent));if(entry)path.push(entry);
  return {entries,scope:id,level:entry?.level||1,path};
}
function arc42BuildingNavigation(shell,levels,navigate){
  shell.appendChild(element('style',`
    .arc42-building-layout{display:grid;grid-template-columns:200px minmax(0,1fr);gap:16px}.arc42-building-nav{min-width:0;max-height:560px;overflow:auto;border-right:1px solid var(--border);padding-right:10px}
    .arc42-building-nav>strong{display:block;font-size:12px;margin:0 0 12px}.arc42-building-nav button{display:flex;gap:8px;width:100%;text-align:left;padding:9px 8px;border:0;border-left:3px solid transparent;border-radius:5px;background:transparent;color:var(--text);font-size:12px;cursor:pointer;overflow-wrap:anywhere}
    .arc42-building-nav button[data-level="3"]{padding-left:24px}.arc42-building-nav button[aria-current="page"]{border-left-color:var(--accent);background:var(--panel);color:var(--accent)}.arc42-building-nav button:hover{background:var(--panel)}
    .arc42-building-nav button small{color:var(--muted)}.arc42-building-content{min-width:0}.arc42-building-path{display:flex;gap:6px;align-items:center;flex-wrap:wrap;margin-bottom:16px}.arc42-building-path button{font-size:12px}.arc42-building-content .arc42-map svg{min-width:0}
    @media(max-width:1000px){.arc42-building-layout{grid-template-columns:1fr}.arc42-building-nav{max-height:190px;border-right:0;border-bottom:1px solid var(--border);padding-bottom:10px}}
  `));
  const layout=element('div',undefined,'arc42-building-layout'),nav=element('nav',undefined,'arc42-building-nav'),body=element('div',undefined,'arc42-building-content');layout.appendChild(nav);layout.appendChild(body);shell.appendChild(layout);
  nav.setAttribute('data-arc42-levels','true');nav.setAttribute('data-current-level',levels.level);nav.setAttribute('aria-label',t('Building-block levels','Bausteinebenen'));nav.appendChild(element('strong',t('Building blocks · Level {level} of 3','Bausteine · Ebene {level} von 3',{level:levels.level})));
  function entry(id,name,level){const b=button('',()=>navigate(id),'');b.setAttribute('data-arc42-level-scope',id??'');b.setAttribute('data-level',level);b.setAttribute('aria-label',t('Level {level}: {name}','Ebene {level}: {name}',{level,name}));b.title=name;b.appendChild(element('small',String(level)));b.appendChild(element('span',id==null?name:name.split('/').pop()));if(id===levels.scope)b.setAttribute('aria-current','page');nav.appendChild(b);}
  entry(null,t('Project overview','Projektübersicht'),1);
  const expanded=levels.path[0]?.node.id;
  for(const [id,item]of levels.entries)if(item.level===2){entry(id,item.node.name,2);if(String(expanded)===id)for(const [child,childItem]of levels.entries)if(childItem.parent===id)entry(child,childItem.node.name,3);}
  const path=element('nav',undefined,'arc42-building-path');path.setAttribute('data-arc42-breadcrumb','true');path.setAttribute('aria-label',t('Building-block path','Bausteinpfad'));body.appendChild(path);
  const crumbs=[{id:null,name:t('Project overview','Projektübersicht'),level:1},...levels.path.map(item=>({id:String(item.node.id),name:item.node.name,level:item.level}))];
  for(const [index,crumb]of crumbs.entries()){if(index)path.appendChild(element('span','›','muted'));const b=button(crumb.level+' · '+(crumb.id==null?crumb.name:crumb.name.split('/').pop()),()=>navigate(crumb.id),'btn btn-secondary');b.title=crumb.name;if(index===crumbs.length-1)b.setAttribute('aria-current','page');path.appendChild(b);}
  return body;
}
function renderArc42PackageMap(parent,kind='dependencies'){
  const states=architectureUi.diagramStates??={},state=states[kind]??={scope:null,selected:null};
  const shell=element('section',undefined,'arc42-visual');shell.setAttribute('data-arc42-package-map',kind);parent.appendChild(shell);
  function navigate(scope){state.scope=scope;state.selected=null;draw();[...shell.querySelectorAll('button[data-arc42-level-scope]')].find(b=>b.getAttribute('data-arc42-level-scope')===(state.scope??''))?.focus({preventScroll:true});}
  function draw(){
    const navigationTop=shell.querySelector('nav[data-arc42-levels]')?.scrollTop||0;
    shell.textContent='';const levels=kind==='dependencies'?arc42BuildingLevels(state.scope):null;if(levels)state.scope=levels.scope;
    const host=levels?arc42BuildingNavigation(shell,levels,navigate):shell;
    if(levels)shell.querySelector('nav[data-arc42-levels]').scrollTop=navigationTop;
    const projection=arc42PackageProjection(state.scope,kind==='calls'?'calls':'dependencies');
    let graph=projection;if(kind==='cycles'){const edges=projection.edges.filter(e=>e.cycle),ids=new Set(edges.flatMap(e=>[e.from,e.to]));graph={...projection,edges,nodes:projection.nodes.filter(n=>ids.has(n.id))};}
    host.appendChild(element('h3',kind==='calls'?t('Static calls between packages','Statische Aufrufe zwischen Paketen'):kind==='cycles'?t('Cyclic package relationships','Zyklische Paketbeziehungen'):t('UML · Packages and dependencies','UML · Pakete und Abhängigkeiten')));
    const controls=element('div',undefined,'arc42-map-controls'),scope=element('select');scope.setAttribute('data-arc42-scope','true');scope.setAttribute('aria-label',t('Package scope','Paketbereich'));
    const option=element('option',t('Project overview','Projektübersicht'));option.value='';scope.appendChild(option);
    for(const n of (levels?[...levels.entries.values()].map(entry=>entry.node):[...data.nodes].filter(n=>n.kind==='module')).sort((a,b)=>compareNames(a.name,b.name))){const o=element('option',n.name);o.value=String(n.id);scope.appendChild(o);}
    scope.value=state.scope||'';scope.addEventListener('change',()=>{state.scope=scope.value||null;state.selected=null;draw();host.querySelector('select[data-arc42-scope]').focus({preventScroll:true});});controls.appendChild(scope);host.appendChild(controls);
    host.appendChild(element('p',projection.total+' '+projection.unit+' · '+projection.internal+' '+t('inside the shown groups','innerhalb der dargestellten Gruppen'),'arc42-map-note'));
    if(kind==='calls')host.appendChild(element('p',t('Compiler status: ','Compilerstatus: ')+(data.calls?.status||'unavailable')+' · '+t('Only retained, resolved project calls with a source mapping. Call sites do not establish runtime order.','Enthaltene, aufgelöste Projektaufrufe mit Quellzuordnung. Aufrufstellen belegen keine Laufzeitreihenfolge.'),'arc42-map-note'));
    if(!graph.nodes.length){host.appendChild(element('p',t('No relationships of this kind are recorded in this scope.','In diesem Bereich sind keine Beziehungen dieser Art erfasst.')));return;}
    const legend=element('p',undefined,'arc42-map-note');host.appendChild(legend);
    const selected=graph.nodes.find(n=>n.id===state.selected),shown=arc42Graph(host,graph,{selected:selected?.id,all:true,onNode:node=>{state.selected=state.selected===node.id?null:node.id;draw();[...host.querySelectorAll('g[data-arc42-node]')].find(n=>n.getAttribute('data-arc42-node')===node.id)?.focus({preventScroll:true});}});
    legend.textContent=t('{shown} / {total} bundled relationships shown. Select a package to highlight its relationships. Arrows: source → dependency. Red: package cycle.','{shown} / {total} gebündelte Beziehungen sichtbar. Paket auswählen, um seine Beziehungen hervorzuheben. Pfeile: Quelle → Abhängigkeit. Rot: Paketzyklus.',{shown,total:graph.edges.length});
    if(selected){const info=element('div',undefined,'arc42-map-selection');info.appendChild(element('strong',selected.name));if(selected.drill&&(!levels||levels.level<3&&levels.entries.has(selected.drill)))info.appendChild(button(levels?t('Open level {level}','Ebene {level} öffnen',{level:levels.level+1}):t('Open package','Paket öffnen'),()=>navigate(selected.drill),'btn btn-secondary'));else if(selected.drill&&levels)info.appendChild(button(t('Open in graph','Im Graph öffnen'),()=>{showTab('graph');revealNode(selected.name);},'btn btn-secondary'));host.appendChild(info);}
    const detail=element('details',undefined,'arc42-relations');detail.appendChild(element('summary',t('Relationships and counts','Beziehungen und Anzahl')+' · '+graph.edges.length));host.appendChild(detail);
    const list=element('div');detail.appendChild(list);let limit=0;const edges=selected?graph.edges.filter(e=>e.from===selected.id||e.to===selected.id):graph.edges;
    const more=button(t('More relationships','Weitere Beziehungen'),fill,'btn btn-secondary');detail.appendChild(more);
    function fill(){for(const e of edges.slice(limit,limit+30)){const from=graph.nodes.find(n=>n.id===e.from),to=graph.nodes.find(n=>n.id===e.to);list.appendChild(element('p',from.name+' → '+to.name+' · '+e.count+' '+projection.unit));}limit+=30;more.hidden=limit>=edges.length;}
    fill();
  }
  draw();
}
function renderArc42Deployment(parent){
  const files=[...new Set((data.services?.services||[]).map(s=>s.compose_file).filter(Boolean))].sort(compareNames);if(!files.length)return false;
  const host=element('section',undefined,'arc42-visual');parent.appendChild(host);const state=architectureUi.deployment??={file:files[0],selected:null};
  function draw(){
    host.textContent='';host.appendChild(element('h3',t('C4 · Deployment configuration','C4 · Deployment-Konfiguration')));
    const select=element('select');select.setAttribute('data-arc42-deployment','true');select.setAttribute('aria-label',t('Deployment configuration','Deployment-Konfiguration'));
    for(const file of files){const option=element('option',file);option.value=file;select.appendChild(option);}select.value=files.includes(state.file)?state.file:files[0];state.file=select.value;
    select.addEventListener('change',()=>{state.file=select.value;state.selected=null;draw();host.querySelector('select').focus({preventScroll:true});});host.appendChild(select);
    host.appendChild(element('p',t('C4 containers from Compose services, grouped by deployment configuration. Dashed arrows: startup dependencies; blue arrows: configured HTTP targets. Runtime hosts and execution order are not measured.','C4-Container aus Compose-Diensten, gruppiert je Deployment-Konfiguration. Gestrichelte Pfeile: Startabhängigkeiten; blaue Pfeile: konfigurierte HTTP-Ziele. Laufzeithosts und Ausführungsreihenfolge sind nicht gemessen.'),'arc42-map-note'));
    const model=arc42DeploymentProjection(state.file);arc42Graph(host,model,{title:'C4 · '+state.file,all:true,selected:state.selected,onNode:node=>{state.selected=state.selected===node.id?null:node.id;draw();}});
    for(const edge of model.edges.filter(e=>!state.selected||e.from===state.selected||e.to===state.selected)){const from=model.nodes.find(n=>n.id===edge.from),to=model.nodes.find(n=>n.id===edge.to);
      const item=element('details');item.appendChild(element('summary',from.name+' → '+to.name+' · '+(edge.kind==='startup'?t('requires startup of','benötigt Start von'):t('configured HTTP','HTTP konfiguriert'))));
      for(const evidence of edge.evidence||[])item.appendChild(element('p',evidence.file+' '+(evidence.pointer||'')+' · '+(evidence.description||'')));host.appendChild(item);
    }
  }
  draw();return true;
}
function renderArc42Derived(parent,number){
  if(number===5&&data.nodes.length){renderArc42PackageMap(parent);return true;}
  if(number===6&&data.calls?.calls?.length){renderArc42PackageMap(parent,'calls');return true;}
  if(number===7)return renderArc42Deployment(parent);
  if(number===11&&data.edges.length){renderArc42PackageMap(parent,'cycles');return true;}
  if(number===4&&architectureModel()){
    const model=architectureModel();parent.appendChild(element('h3',t('Declared component model','Deklariertes Komponentenmodell')));
    const graph={nodes:model.components.map(n=>({id:n.name,name:n.name,files:n.node_count,kind:'component'})),edges:model.dependencies.map(e=>({from:e.from,to:e.to,count:e.source_edges})),unit:t('source connections','Quellverbindungen')},membership=new Map();
    findSccs(graph).forEach((cycle,i)=>cycle.members.forEach(id=>membership.set(id,i)));for(const edge of graph.edges)edge.cycle=membership.has(edge.from)&&membership.get(edge.from)===membership.get(edge.to);
    parent.appendChild(element('p',t('Declared boundaries and observed source dependencies. Red: component cycle.','Deklarierte Grenzen und beobachtete Quellabhängigkeiten. Rot: Komponentenzyklus.'),'arc42-map-note'));
    arc42Graph(parent,graph,{all:true,title:model.name});return true;
  }
  return false;
}
// Standard diagram vocabulary; the layout engine runs in the browser, never on a remote renderer.
let arc42VizPromise=null;
function arc42LayoutEngine(){
  if(arc42VizPromise)return arc42VizPromise;
  arc42VizPromise=new Promise((resolve,reject)=>{
    if(typeof Viz!=='undefined'){Viz.instance().then(resolve,reject);return;}
    const script=document.createElement('script');script.src='https://cdn.jsdelivr.net/npm/@viz-js/viz@3.11.0/lib/viz-standalone.js';
    script.crossOrigin='anonymous';script.integrity='sha384-6ECxW8G4FmLMEr8fKkk617NsYhxb7AhsiyyT6Gey/Sru4R+lyRpVM12ekzlOD08g';
    const timeout=setTimeout(()=>{script.remove();reject(new Error('Layout library unavailable'));},15000);
    script.onload=()=>{clearTimeout(timeout);Viz.instance().then(resolve,reject);};script.onerror=()=>{clearTimeout(timeout);script.remove();reject(new Error('Layout library unavailable'));};document.head.appendChild(script);
  });
  return arc42VizPromise;
}
function arc42StandardGraph(model,selected){
  const ids=new Map(model.nodes.map((n,i)=>[n.id,'n'+i]));
  const deployment=model.nodes.some(n=>n.kind==='deployment');
  const graph={directed:true,graphAttributes:{rankdir:model.nodes.length>4?'TB':'LR',bgcolor:'transparent',pad:'.3',nodesep:'.55',ranksep:'1',splines:'spline',fontname:'Arial',fontcolor:'#b6c4d8'},nodeAttributes:{fontname:'Arial',fontsize:'12',margin:'.18,.14',style:'filled',fillcolor:'#1c2b42',fontcolor:'#ecf2fa',color:'#607796',penwidth:'1.4'},edgeAttributes:{fontname:'Arial',fontsize:'10',fontcolor:'#c3cfe2',arrowsize:'.7'},nodes:model.nodes.map((n,i)=>({name:ids.get(n.id),attributes:{id:'arc42-standard-node-'+i,shape:n.kind==='package'?'folder':n.kind==='component'?'component':'box',label:(n.name.split('/').pop()||n.name)+'\n['+(n.kind==='deployment'?'Container · Docker':n.kind==='package'?'Package':n.kind==='group'?t('Grouped packages','Zusammengefasste Pakete'):n.kind==='context'?t('Project context','Projektkontext'):n.kind==='component'?'Component':t('Source files','Quelldateien'))+']'+(n.files!=null?'\n'+n.files+' '+t('source nodes','Quellknoten'):''),tooltip:n.name,color:selected===n.id?'#a7c4ff':n.cycle?'#ee7d87':'#607796',penwidth:selected===n.id?'3':'1.4'}})),edges:model.edges.filter(e=>ids.has(e.from)&&ids.has(e.to)).map(e=>({tail:ids.get(e.from),head:ids.get(e.to),attributes:{label:e.kind==='startup'?'startup':e.kind==='http'?'HTTP':String(e.count??1),tooltip:model.nodes.find(n=>n.id===e.from).name+' → '+model.nodes.find(n=>n.id===e.to).name+' · '+(e.count??1)+' '+(model.unit||''),color:selected&&e.from!==selected&&e.to!==selected?'#45556c':e.cycle?'#ee7d87':'#92b6ef',style:e.kind==='startup'?'dashed':'solid',penwidth:selected&&(e.from===selected||e.to===selected)?'2.5':'1.3'}}))};
  if(deployment){graph.subgraphs=[{name:'cluster_deployment',graphAttributes:{label:t('Deployment configuration · ','Deployment-Konfiguration · ')+model.scope,style:'dashed',color:'#607796',fontcolor:'#b6c4d8',labelloc:'t'},nodes:graph.nodes}];graph.nodes=[];}
  return graph;
}
function arc42PlantUml(model){
  // Encode delimiters and line controls as PlantUML Unicode escapes: source names stay labels.
  const label=value=>String(value??'').replace(/["\\<>\r\n\x00-\x1f]/g,c=>'<U+'+c.charCodeAt(0).toString(16).padStart(4,'0')+'>');
  const ids=new Map(model.nodes.map((n,i)=>[n.id,'n'+i])),deployment=model.nodes.some(n=>n.kind==='deployment');
  const lines=['@startuml','!pragma layout smetana','left to right direction','skinparam shadowing false','skinparam componentStyle uml2'];
  if(deployment)lines.push('!include <C4/C4_Deployment>','Deployment_Node(scope, "'+label(model.scope)+'", "Docker Compose configuration", "Declared configuration; runtime hosts are unknown") {');
  for(const n of model.nodes){const id=ids.get(n.id);if(deployment)lines.push('Container('+id+', "'+label(n.name)+'", "Compose service", "'+label(n.detail)+'")');else lines.push((n.kind==='package'?'package':n.kind==='component'?'component':'rectangle')+' "'+label(n.name)+'" as '+id+(n.cycle?' #FFE4E6':''));}
  if(deployment)lines.push('}');
  for(const e of model.edges){if(!ids.has(e.from)||!ids.has(e.to))continue;const description=e.kind==='startup'?'startup':e.kind==='http'?'configured HTTP':(e.count??1)+' '+(model.unit||'connections');
    if(deployment)lines.push('Rel('+ids.get(e.from)+', '+ids.get(e.to)+', "'+label(description)+'")');
    else lines.push(ids.get(e.from)+' -['+(e.cycle?'#ee7d87':'#607796')+(e.kind==='startup'?',dashed':'')+']-> '+ids.get(e.to)+' : '+label(description));
  }
  lines.push('@enduml');return lines.join('\n');
}
function arc42UpgradeDiagram(box,model,{title,onNode,selected}={}){
  const toolbar=element('div',undefined,'arc42-map-controls');if(box.parentElement.insertBefore)box.parentElement.insertBefore(toolbar,box);else box.parentElement.appendChild(toolbar);
  const status=element('span',t('Automatic layout is loading…','Automatisches Layout wird geladen…'),'arc42-map-note');toolbar.appendChild(status);
  const source=arc42PlantUml(model);
  toolbar.appendChild(button('PlantUML ↓',()=>{const blob=new Blob([source],{type:'text/plain;charset=utf-8'}),url=URL.createObjectURL(blob),link=element('a');link.href=url;link.download='arc42-'+(model.kind||'diagram')+'.puml';link.click();setTimeout(()=>URL.revokeObjectURL(url),1000);},'btn btn-secondary'));
  arc42LayoutEngine().then(viz=>{
    if(!box.isConnected)return;
    const svg=viz.renderSVGElement(arc42StandardGraph(model,selected));svg.setAttribute('data-arc42-graph','true');svg.setAttribute('data-arc42-layout','graphviz');svg.setAttribute('role','group');svg.setAttribute('aria-label',title||t('UML package diagram','UML-Paketdiagramm'));
    svg.style.minWidth='0';svg.style.maxWidth='none';svg.style.height='auto';
    const fit=()=>{const bounds=svg.viewBox.baseVal;svg.style.width=Math.min(box.clientWidth||bounds.width,bounds.width,560*bounds.width/bounds.height)+'px';};fit();
    model.nodes.forEach((n,i)=>{const group=svg.querySelector('#arc42-standard-node-'+i);if(!group)return;group.setAttribute('data-arc42-node',n.id);group.setAttribute('class','node arc42-map-node');group.setAttribute('aria-label',n.name);if(onNode){group.setAttribute('role','button');group.setAttribute('tabindex','0');group.setAttribute('aria-pressed',String(selected===n.id));group.addEventListener('click',()=>onNode(n));group.addEventListener('keydown',e=>{if(e.key==='Enter'||e.key===' '){e.preventDefault();onNode(n);}});}});
    const focused=box.contains(document.activeElement)?document.activeElement.getAttribute('data-arc42-node'):null;box.textContent='';box.appendChild(svg);if(focused!=null)[...svg.querySelectorAll('[data-arc42-node]')].find(n=>n.getAttribute('data-arc42-node')===focused)?.focus({preventScroll:true});status.textContent=t('Arrows show the dependency direction; edge labels show recorded counts.','Pfeile zeigen die Abhängigkeitsrichtung; Kantenbeschriftungen die erfasste Anzahl.');
    toolbar.appendChild(button(t('Fit','Einpassen'),fit,'btn btn-secondary'));
    for(const [label,factor]of [['−',.8],['+',1.25]])toolbar.appendChild(button(label,()=>{svg.style.width=Math.max(160,Math.min(5000,svg.getBoundingClientRect().width*factor))+'px';},'btn btn-secondary'));
    toolbar.appendChild(button('1:1',()=>{svg.style.width=svg.viewBox.baseVal.width+'px';},'btn btn-secondary'));
  }).catch(()=>{if(box.isConnected)status.textContent=t('Automatic layout unavailable. Basic diagram and PlantUML export remain available.','Automatisches Layout nicht verfügbar. Basisdiagramm und PlantUML-Export bleiben verfügbar.');});
}
