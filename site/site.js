'use strict';
const german = document.documentElement.lang === 'de';
const assetBase = new URL('assets/', document.currentScript.src);
const views = {
 'world-complexity': {title:'Spot complex code at a glance.', description:'Surfaces are coloured from low to high by the maximum cyclomatic complexity of their methods. The status line names the highest recorded value: 29, in the map package.', project:'Apache Commons Collections · Java · Explore lens', alt:'Apache Commons Collections in World View, class surfaces coloured by cyclomatic complexity'},
 'world-cbo': {title:'See which classes depend on too many others.', description:'Coupling Between Objects counts the other classes a class uses. Green, amber and red mark low, elevated and high values. The highest recorded CBO is 75, in IteratorUtils.java.', project:'Apache Commons Collections · Java · Explore lens', alt:'World View with class surfaces coloured by CBO: low, elevated and high coupling'},
 'world-wmc': {title:'Find the heaviest classes.', description:'Weighted Methods per Class adds up the complexity of all methods in a class. The highest recorded WMC is 225, in the trie package.', project:'Apache Commons Collections · Java · Explore lens', alt:'World View with class surfaces coloured by WMC'},
 'world-lcom4': {title:'Find classes that do several unrelated things.', description:'LCOM4 counts the connected groups of methods in a class, linked by shared fields or calls; 1 means the class is cohesive. The highest recorded LCOM4 is 37, in the properties package.', project:'Apache Commons Collections · Java · Explore lens', alt:'World View with class surfaces coloured by LCOM4 cohesion'},
 'world-package': {title:'Open a package, keep the metric.', description:'Click a package to see its classes with their own values. Inside map, Flat3Map.java has the highest complexity: 29. The breadcrumb leads back to the whole repository.', project:'Apache Commons Collections · Java · map package', alt:'World View inside the map package with each class labelled by its maximum complexity'},
 graph: {title:'Keep the hierarchy. Follow the edges.', description:'Explore package dependencies in their tree structure. Incoming counts on the left, outgoing edges and cycles on the right. Select a package to focus its relationships and inspect the supporting source references.', project:'Apache Commons Collections · Java', alt:"Apache Commons Collections shown in ArchGuard's dependency graph"},
 matrix: {title:'Read dense dependencies one row at a time.', description:'Keep the package tree alongside a dependency matrix. Each cell counts the dependencies from a row package to a column package, and cycle groups are labelled next to the tree.', project:'Apache Commons Collections · Java', alt:'ArchGuard tree matrix showing package dependency counts and cycle groups for Apache Commons Collections'},
 calls: {title:'See which methods call which.', description:'Browse calls between classes and methods in a call tree. Each connection lists its call sites, the declared target and source locations, so a dependency can be traced to the exact line.', project:'Apache Commons Collections · Java', alt:'ArchGuard call overview listing BagUtils call connections with call sites and source locations'},
 quality: {title:'Find classes that need a second look.', description:'A class heatmap with coupling (CBO), response set (RFC), inheritance depth (DIT), weighted methods (WMC) and cohesion (LCOM4). Classes without measurements stay marked as unknown instead of zero.', project:'Apache Commons Collections · Java · partial analysis', alt:'ArchGuard quality heatmap with CBO, RFC, DIT, WMC and LCOM4 values per class'},
 arc42: {title:'Architecture documentation, derived from the code.', description:'The Architecture tab lays out all twelve arc42 chapters. The building block view starts with a bounded package overview. Open a package to inspect its detail and keep the source-derived diagram alongside the chapter text.', project:'Apache Commons Collections · Java · arc42 chapter 5', alt:'ArchGuard desktop app Architecture tab: arc42 building block view of Apache Commons Collections with a UML package diagram'},
 'arc42-context': {title:'See the system in its context.', description:'Chapter 3 derives a context diagram from the code: who uses the system, the programs it starts, the HTTP hosts it addresses and its datastores. It shows what the source states, not proof of completeness or actual use.', project:'Microsoft eShop · .NET / Aspire · arc42 chapter 3', alt:'ArchGuard desktop app Architecture tab: arc42 context and scope diagram of Microsoft eShop with users, the system and external HTTP hosts'},
 'eshop-world': {title:'Map services and the infrastructure between them.', description:'Applications, databases and the event bus from the Aspire app host in one overview. The selected lens shows inferred event relationships and a cycle group. The analysis is partial; these are static candidates, not observed runtime calls.', project:'Microsoft eShop · .NET / Aspire', alt:'Microsoft eShop services and infrastructure in the ArchGuard service map with an inferred event cycle'},
 'eshop-services': {title:'Check the source behind every connection.', description:'Select Catalog API to inspect its project and manifest evidence alongside the inferred relationships with Ordering API and Webhooks API. The view marks a cycle group and partial coverage. These static candidates are not runtime traces.', project:'Microsoft eShop · inferred event flow', alt:'Microsoft eShop catalog-api selected in the ArchGuard desktop app Services view, with its cyclic event group highlighted'}
};
for(const block of document.querySelectorAll('[data-gallery]'))for(const button of block.querySelectorAll('[data-view]'))button.addEventListener('click',()=>{
 const key=button.dataset.view, view=views[key], part=name=>block.querySelector(`[data-gallery-${name}]`);
 if(!view)return;
 for(const other of block.querySelectorAll('[data-view]'))other.setAttribute('aria-pressed',String(other===button));
 part('image').src=new URL(`${key}.png`,assetBase).href;
 part('image').alt=view.alt;
 part('link').href=new URL(`${key}.png`,assetBase).href;
 part('title').textContent=view.title;
 part('description').textContent=view.description;
 part('project').textContent=view.project;
});
// Quickstart OS switch: shows one command set, preselected from the visitor's OS.
for(const group of document.querySelectorAll('.os-tabs')){
 const box=group.closest('.terminal'),buttons=[...group.querySelectorAll('button')];
 const show=os=>{for(const b of buttons)b.setAttribute('aria-pressed',String(b.dataset.os===os));for(const pre of box.querySelectorAll('pre[data-os]'))pre.hidden=pre.dataset.os!==os;};
 for(const b of buttons)b.addEventListener('click',()=>show(b.dataset.os));
 if(/Windows/.test(navigator.userAgent))show('windows');
}
let statusTimeout;
for(const button of document.querySelectorAll('.copy'))button.addEventListener('click',async()=>{
 const code=button.closest('.code,.terminal').querySelector('pre:not([hidden]) code');const text=code.innerText;
 let status=document.getElementById('copy-status');
 if(!status){status=document.createElement('div');status.id='copy-status';status.setAttribute('role','status');status.setAttribute('aria-live','polite');document.body.append(status);}
 try{await navigator.clipboard.writeText(text);status.textContent=german?'Befehl kopiert.':'Command copied.';}
 catch{const range=document.createRange();range.selectNodeContents(code);const selection=window.getSelection();selection.removeAllRanges();selection.addRange(range);status.textContent=german?'Befehl markiert. Mit Strg+C oder ⌘C kopieren.':'Command selected. Press Ctrl+C or ⌘C to copy.';}
 clearTimeout(statusTimeout);statusTimeout=setTimeout(()=>{status.textContent='';},3500);
});

// The flow video plays only while visible, never with reduced motion, and stays paused once a visitor pauses it.
const flow=document.querySelector('[data-flow-video]');
if(flow&&'IntersectionObserver' in window&&!matchMedia('(prefers-reduced-motion: reduce)').matches){
 let visible=false,paused=false;
 flow.addEventListener('pause',()=>{if(visible&&!flow.ended)paused=true;});
 flow.addEventListener('play',()=>{paused=false;});
 new IntersectionObserver(([entry])=>{visible=entry.isIntersecting;if(visible&&!paused)flow.play().catch(()=>{});else if(!visible)flow.pause();},{threshold:.4}).observe(flow);
}
// On narrow screens tables stack into one block per row; each cell names its column.
for(const table of document.querySelectorAll('.table-wrap table')){
 const heads=[...table.querySelectorAll('thead th')].map(th=>th.textContent.trim());
 for(const row of table.querySelectorAll('tbody tr'))[...row.children].forEach((cell,i)=>{if(i&&heads[i])cell.dataset.label=heads[i];});
 table.classList.add('stackable');
}
