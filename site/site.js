'use strict';
const german = document.documentElement.lang === 'de';
const assetBase = new URL('assets/', document.currentScript.src);
const views = {
 'world-complexity': {title:"Complexity hotspots", description:"Identify branching-heavy code and focus reviews on testability and maintenance effort.", project:'Apache Commons Collections · Java · Explore lens', alt:'Apache Commons Collections in World View, class surfaces coloured by cyclomatic complexity'},
 'world-cbo': {title:"Coupling across class boundaries", description:"Identify classes with many dependencies and investigate where changes may propagate.", project:'Apache Commons Collections · Java · Explore lens', alt:'World View with class surfaces coloured by CBO: low, elevated and high coupling'},
 'world-wmc': {title:"Class complexity and responsibility", description:"Use accumulated method complexity to identify classes that merit closer review or decomposition.", project:'Apache Commons Collections · Java · Explore lens', alt:'World View with class surfaces coloured by WMC'},
 'world-lcom4': {title:"Cohesion and responsibility", description:"Investigate disconnected method groups as candidates for separating responsibilities.", project:'Apache Commons Collections · Java · Explore lens', alt:'World View with class surfaces coloured by LCOM4 cohesion'},
 'world-package': {title:"Focused package reviews", description:"Compare class metrics within a package to refine a refactoring candidate.", project:'Apache Commons Collections · Java · map package', alt:'World View inside the map package with each class labelled by its maximum complexity'},
 graph: {title:"Review module dependencies", description:"Assess package boundaries and cycles, then trace disputed dependencies to their source locations.", project:'Apache Commons Collections · Java', alt:"Apache Commons Collections shown in ArchGuard's dependency graph"},
 matrix: {title:"Evaluate dependency density", description:"Review coupling between packages and recurring cycles in a compact matrix.", project:'Apache Commons Collections · Java', alt:'ArchGuard tree matrix showing package dependency counts and cycle groups for Apache Commons Collections'},
 calls: {title:"Assess call dependencies", description:"Inspect call sites and targets when preparing an interface change or investigating a dependency.", project:'Apache Commons Collections · Java', alt:'ArchGuard call overview listing BagUtils call connections with call sites and source locations'},
 quality: {title:"Prioritize structural reviews", description:"Combine coupling, complexity and cohesion to choose where a code review should go deeper.", project:'Apache Commons Collections · Java · partial analysis', alt:'ArchGuard quality heatmap with CBO, RFC, DIT, WMC and LCOM4 values per class'},
 arc42: {title:"Maintain architecture documentation", description:"Keep source-derived building blocks aligned with the implementation and integrate them into arc42.", project:'Apache Commons Collections · Java · arc42 chapter 5', alt:'ArchGuard desktop app Architecture tab: arc42 building block view of Apache Commons Collections with a UML package diagram'},
 'arc42-context': {title:"Review system boundaries", description:"Ground context discussions in the applications, external systems and datastores recognized in the project.", project:'Microsoft eShop · .NET / Aspire · arc42 chapter 3', alt:'ArchGuard desktop app Architecture tab: arc42 context and scope diagram of Microsoft eShop with users, the system and external HTTP hosts'},
 'eshop-world': {title:"Assess service coupling", description:"Investigate service boundaries and inferred event relationships from project configuration and source code.", project:'Microsoft eShop · .NET / Aspire', alt:'Microsoft eShop services and infrastructure in the ArchGuard service map with an inferred event cycle'},
 'eshop-services': {title:"Verify an integration dependency", description:"Trace inferred service relationships to project and manifest evidence before changing an integration point.", project:'Microsoft eShop · inferred event flow', alt:'Microsoft eShop catalog-api selected in the ArchGuard desktop app Services view, with its cyclic event group highlighted'}
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

// On narrow screens tables stack into one block per row; each cell names its column.
for(const table of document.querySelectorAll('.table-wrap table')){
 const heads=[...table.querySelectorAll('thead th')].map(th=>th.textContent.trim());
 for(const row of table.querySelectorAll('tbody tr'))[...row.children].forEach((cell,i)=>{if(i&&heads[i])cell.dataset.label=heads[i];});
 table.classList.add('stackable');
}
