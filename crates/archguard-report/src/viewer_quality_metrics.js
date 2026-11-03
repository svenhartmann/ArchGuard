'use strict';
// Imported measurements and impact belong to the original source snapshot.
// Index once; browsing/simulation never reruns reachability or blends unknowns into zero.
const coverageUi={source:null,review:null,byNode:new Map(),rowsByNode:new Map(),rows:[],limit:20,query:''};
function testCoverageModel() {
  const source=data.insights?.test_coverage,review=data.insights?.coverage_review;
  if(coverageUi.source!==source||coverageUi.review!==review){
    coverageUi.source=source;coverageUi.review=review;coverageUi.limit=20;
    coverageUi.byNode=new Map((source?.files||[]).map(file=>[file.node,file]));
    const impacts=new Map((review?.files||[]).map(row=>[row.node,row]));
    const nodes=coverageUi.byNode.size?new Map(data.nodes.map(node=>[node.name,node.id])):new Map();
    coverageUi.rows=[...coverageUi.byNode.values()].map(file=>({file,impact:impacts.get(file.node),sourceId:nodes.get(file.node)}));
    coverageUi.rowsByNode=new Map(coverageUi.rows.map(row=>[row.file.node,row]));
    coverageUi.rows.sort((a,b)=>(coverageReviewScore(b)??-1)-(coverageReviewScore(a)??-1)||compareNames(a.file.node,b.file.node));
  }
  return source;
}
function coverageCounterText(counter) {
  return counter&&Number.isFinite(counter.covered)&&Number.isFinite(counter.total)&&counter.total>0&&counter.covered>=0&&counter.covered<=counter.total?
    (100*counter.covered/counter.total).toFixed(1)+'% ('+counter.covered+' / '+counter.total+')':'—';
}
function coverageReviewScore(row) {
  return ['ready','partial','stale'].includes(coverageUi.source?.status)&&row.file.status==='unchanged_since_import'&&coverageCounterText(row.file.lines)!=='—'&&Number.isFinite(row.impact?.score)?row.impact.score:null;
}
function coverageStatusText(status) {
  return {not_imported:t('not imported','nicht importiert'),ready:t('imported','importiert'),partial:t('incomplete report','unvollständiger Bericht'),stale:t('stale: sources changed','veraltet: Quellen geändert'),error:t('unavailable: import error','nicht verfügbar: Importfehler')}[status]||t('not available','nicht verfügbar');
}
function coverageFileText(file) {
  return t('Lines: ','Zeilen: ')+coverageCounterText(file.lines)+' · '+t('Branches: ','Zweige: ')+coverageCounterText(file.branches)+
    (file.status==='changed_since_import'?t(' · source changed since import',' · Quelle seit dem Import geändert'):file.status==='unchanged_since_import'?'':t(' · source freshness unknown',' · Aktualität der Quelle unbekannt'));
}
function coverageReviewText(row) {
  const score=coverageReviewScore(row);
  return t('Review index ','Prüfindex ')+(score===null?'—':score.toFixed(2))+' · '+
    t('{count} transitive dependent files','{count} transitiv abhängige Dateien',{count:Number.isFinite(row.impact?.dependents)?row.impact.dependents:'—'});
}
function openCoverageReview() {
  document.getElementById('review-open').click();document.getElementById('coverage-review').open=true;
  document.getElementById('coverage-heading').focus();
}
function coverageScopeSummary() {
  const source=testCoverageModel();
  if(!['ready','partial','stale'].includes(source?.status))return null;
  const scope=new Set(view.nodes.map(node=>node.name));
  let coveredLines=0,totalLines=0;
  for(const name of scope){
    const file=coverageUi.byNode.get(name);
    const lines=file?.lines;
    if(file&&file.status==='unchanged_since_import'&&Number.isFinite(lines?.covered)&&Number.isFinite(lines?.total)
       &&lines.total>0&&lines.covered>=0&&lines.covered<=lines.total){coveredLines+=lines.covered;totalLines+=lines.total;}
  }
  return totalLines>0?{covered:coveredLines,total:totalLines}:null;
}
function appendCoverageSummary(container) {
  const source=testCoverageModel(),action=button(t('Test coverage: {count} imported files · {status}','Testabdeckung: {count} importierte Dateien · {status}',{count:source?.files?.length||0,status:coverageStatusText(source?.status||'not_imported')}),openCoverageReview,'btn btn-secondary');
  action.setAttribute('data-coverage-summary','true');container.appendChild(action);
}
function renderTestCoveragePanel() {
  const source=testCoverageModel(),status=source?.status||'not_imported',q=currentFilter();
  if(coverageUi.query!==q){coverageUi.limit=20;coverageUi.query=q;}
  const rows=coverageUi.rows.filter(row=>!q||[row.file.node,row.file.path].join(' ').toLowerCase().includes(q));
  document.getElementById('coverage-status').textContent=t('Original source snapshot · {status} · {shown} / {total} imported files','Ursprünglicher Quellcode-Snapshot · {status} · {shown} / {total} importierte Dateien',{status:coverageStatusText(status),shown:Math.min(rows.length,coverageUi.limit),total:rows.length});
  const explanation=document.getElementById('coverage-explanation');explanation.textContent='';
  explanation.appendChild(element('p',t('Review index = uncovered line fraction × (1 + transitive dependent files). It is a review order, not a defect probability. Impact uses the original file graph and is unchanged by simulation.','Prüfindex = Anteil ungetesteter Zeilen × (1 + transitiv abhängige Dateien). Er bestimmt eine Prüfreihenfolge, keine Fehlerwahrscheinlichkeit. Die Reichweite stammt aus dem ursprünglichen Dateigraphen und bleibt bei Simulationen unverändert.')));
  explanation.appendChild(element('p',t('Report/source revision at import is unverified. Source changes after import make the affected index unavailable. Missing files and zero executable lines/branches are unknown (—), never 0% coverage. Line and branch counters have different denominators.','Die Quellcode-Revision des Berichts ist beim Import nicht verifiziert. Änderungen nach dem Import machen den betroffenen Index unverfügbar. Fehlende Dateien sowie null ausführbare Zeilen/Zweige bleiben unbekannt (—), niemals 0% Abdeckung. Zeilen- und Zweigzähler haben unterschiedliche Bezugsgrößen.')));
  if(source?.report_path)explanation.appendChild(element('p',(source.format||'')+' · '+source.report_path,'path'));
  if(source?.imported_at_unix!=null){const date=new Date(source.imported_at_unix*1000);explanation.appendChild(element('p',t('Imported: ','Importiert: ')+(Number.isFinite(date.getTime())?date.toISOString():t('unknown','unbekannt')),'muted'));}
  if(source?.unmatched_files)explanation.appendChild(element('p',source.unmatched_files+t(' unmatched report files',' nicht zugeordnete Berichtsdateien'),'muted'));
  for(const message of (source?.diagnostics||[]).slice(0,20))explanation.appendChild(element('p',message,'muted'));
  explanation.appendChild(element('p',t('Import or refresh a JaCoCo/Cobertura/Clover/LCOV report:','JaCoCo-, Cobertura-, Clover- oder LCOV-Bericht importieren oder aktualisieren:')));
  explanation.appendChild(element('pre','archguard coverage . --report path/to/report.xml\n# optional: --source-root src/main/java'));
  const list=document.getElementById('coverage-list');list.textContent='';
  for(const row of rows.slice(0,coverageUi.limit)){
    const item=element('li'),action=button('',()=>{showTab('graph');openSourceEvidence(row.sourceId,row.file.node);},'review-row');
    action.setAttribute('data-coverage-node',row.file.node);action.disabled=row.sourceId==null||!nodeById(work,row.sourceId);
    action.appendChild(element('strong',row.file.node));action.appendChild(element('small',coverageFileText(row.file)));
    action.appendChild(element('small',coverageReviewText(row)));item.appendChild(action);list.appendChild(item);
  }
  if(!rows.length)list.appendChild(element('li',status==='not_imported'?t('No test report imported. Coverage is unknown.','Kein Testbericht importiert. Die Testabdeckung ist unbekannt.'):t('No matching imported files. Missing measurements do not mean untested code.','Keine passenden importierten Dateien. Fehlende Messungen bedeuten nicht ungetesteten Code.'),'muted'));
  document.getElementById('coverage-more').hidden=rows.length<=coverageUi.limit;
}
function renderTestCoverageDetails(container,sourceId) {
  const node=data.nodes.find(row=>row.id===sourceId);if(node?.kind!=='file')return;
  const source=testCoverageModel(),file=coverageUi.byNode.get(node.name),section=element('details',undefined,'review-details inspector-section');
  section.setAttribute('data-test-coverage','true');section.open=!!file;
  section.appendChild(element('summary',t('Test coverage','Testabdeckung')));
  if(file){
    section.appendChild(element('p',coverageFileText(file)));
    section.appendChild(element('p',coverageReviewText(coverageUi.rowsByNode.get(file.node)),'muted'));
    section.appendChild(element('p',file.path,'path'));
  }else section.appendChild(element('p',t('No imported measurement for this file. Coverage is unknown.','Keine importierte Messung für diese Datei. Die Testabdeckung ist unbekannt.'),'muted'));
  section.appendChild(element('p',t('Imported source evidence; report/source revision unverified.','Importierte Quellcode-Belege; Berichts-/Quellcode-Revision nicht verifiziert.'),'muted'));
  if(source?.status==='error')section.appendChild(element('p',coverageStatusText(source.status),'muted'));
  section.appendChild(button(t('Coverage and impact review','Testabdeckung und Reichweite prüfen'),openCoverageReview));container.appendChild(section);
}
function exportTestCoverage() {
  const source=testCoverageModel();if(!source)return {};
  return {test_coverage:{...source,scope:'all_imported_files_original_snapshot'},coverage_review:{
    definition:data.insights?.coverage_review?.definition||'unavailable',scope:'all_imported_files_original_snapshot',
    files:coverageUi.rows.map(row=>({node:row.file.node,path:row.file.path,dependents:row.impact?.dependents??null,score:coverageReviewScore(row)}))}};
}
function appendCoverageMarkdown(lines,snapshot) {
  const source=snapshot.test_coverage;if(!source)return;
  const cell=exportMarkdownCell,impact=new Map((snapshot.coverage_review?.files||[]).map(row=>[row.node,row]));
  lines.push('','## Test coverage and change impact','',
    'Scope: all imported files in the original source snapshot, independent of graph filters and simulation. Missing source files have unknown coverage.',
    '', 'Status: '+cell(source.status)+'. Format: '+cell(source.format||'unknown')+'. Report: '+cell(source.report_path||'unknown')+'.',
    '', 'Report/source revision at import is unverified. Review index = uncovered line fraction × (1 + transitive dependent files); a heuristic review order, not defect probability. Stale or unknown measurements have no index (—).',
    '', '| Source node | Reported file | Line coverage (covered / total) | Branch coverage (covered / total) | Dependent files | Review index | Source freshness |',
    '| --- | --- | --- | --- | ---: | ---: | --- |');
  for(const file of source.files||[]){const row=impact.get(file.node);lines.push('| '+[file.node,file.path,coverageCounterText(file.lines),coverageCounterText(file.branches),row?.dependents??'—',Number.isFinite(row?.score)?row.score.toFixed(2):'—',file.status||'unknown'].map(cell).join(' | ')+' |');}
  if(source.imported_at_unix!=null)lines.push('','Imported at (Unix seconds): '+cell(source.imported_at_unix)+'.');
  if(source.report_modified_unix!=null)lines.push('','Report modified (Unix seconds): '+cell(source.report_modified_unix)+'.');
  if(source.unmatched_files)lines.push('',cell(source.unmatched_files)+' unmatched report files.');
  for(const diagnostic of source.diagnostics||[])lines.push('- '+cell(diagnostic));
}
document.getElementById('coverage-more').addEventListener('click',()=>{coverageUi.limit+=20;renderTestCoveragePanel();});

// Setup help travels with standalone reports. Availability follows the shipped
// extractors and semantic adapters; installing a report producer adds coverage,
// not a compiler adapter for an unsupported language.
const kpiHelpUi={action:null,dialog:null,body:null,detail:null,language:null,previousFocus:null};
function kpiLanguageHelp() {
  const jsCoverage={tool:'Vitest + @vitest/coverage-v8',format:'LCOV',
    note:t('Example for an existing Vitest project. Install its matching coverage provider. Other runners such as c8/nyc can also export LCOV; TypeScript paths must map back to the original sources.','Beispiel für ein vorhandenes Vitest-Projekt. Den zur Vitest-Version passenden Coverage-Provider installieren. Auch andere Runner wie c8/nyc können LCOV liefern; TypeScript-Pfade müssen auf die ursprünglichen Quellen zeigen.'),
    commands:'npx --no-install vitest run --coverage --coverage.reporter=lcov\narchguard coverage . --report coverage/lcov.info',
    links:[['Vitest: coverage','https://vitest.dev/guide/coverage.html'],['Vitest: reporters','https://vitest.dev/config/coverage.html']]};
  const noCompiler=t('No compiler-backed class/method metrics in this version. Installing a compiler, linter or coverage tool does not enable CBO, RFC, DIT, NOC, WMC, LCOM4, Cognitive, Halstead or MI for this language. Import-graph KPIs, Git and imported coverage remain available.','In dieser Version keine compilerbasierten Klassen-/Methodenmetriken. Ein installierter Compiler, Linter oder Coverage-Scanner schaltet CBO, RFC, DIT, NOC, WMC, LCOM4, Cognitive, Halstead oder MI für diese Sprache nicht frei. Importgraph-KPIs, Git und importierte Testabdeckung bleiben verfügbar.');
  return [
    {id:'go',name:'Go',extensions:'.go',compiler:'Go SDK',semantic:true,
      requirement:t('Go SDK with go on PATH, or ARCHGUARD_GO pointing to the executable. Package imports work without the SDK.','Go SDK mit go auf PATH oder ARCHGUARD_GO als Pfad zur ausführbaren Datei. Paketimporte funktionieren auch ohne SDK.'),
      setup:t('The bundled go/parser and go/types helper reads selected source snapshots offline. It measures structs, interfaces, receiver methods and file-level function groups. WMC, nesting, Cognitive, Halstead, MI and field-based cohesion use source evidence. CBO/RFC remain partial for unresolved calls/types. DIT/NOC are not applicable: Go embedding is composition. Dynamic dispatch, missing external modules and combined build variants remain explicit gaps. Project code, generators and dependency downloads are never executed.','Der mitgelieferte go/parser- und go/types-Helfer liest ausgewählte Quellsnapshots offline. Er erfasst Structs, Interfaces, Receiver-Methoden und Funktionsgruppen je Datei. WMC, Verschachtelung, Cognitive, Halstead, MI und feldbasierte Kohäsion beruhen auf Quellbelegen. CBO/RFC bleiben bei unaufgelösten Aufrufen/Typen teilweise. DIT/NOC sind nicht anwendbar: Go-Embedding ist Komposition. Dynamische Aufrufe, fehlende externe Module und kombinierte Build-Varianten bleiben sichtbare Lücken. Projektcode, Generatoren und Downloads werden nicht ausgeführt.'),
      commands:'go version\narchguard calls . --format json',
      coverage:{tool:t('Optional external report','Optionaler externer Bericht'),format:'LCOV / Cobertura',note:t('Import an existing converted coverage report with paths matching the Go sources. Native Go coverprofiles are not currently accepted. No coverage is inferred from source metrics.','Einen vorhandenen konvertierten Coverage-Bericht mit passenden Go-Quellpfaden importieren. Native Go-Coverprofiles werden derzeit nicht akzeptiert. Quellmetriken ergeben keine Testabdeckung.'),commands:'archguard coverage . --report coverage.xml',links:[['Go parser','https://pkg.go.dev/go/parser'],['Go types','https://pkg.go.dev/go/types']]}},
    {id:'java',name:'Java',extensions:'.java',compiler:'JDK 17+',semantic:true,
      requirement:t('A full JDK 17+ with java on PATH; the selected Java release and dependency JARs must match the project. The javac scanner is included in ArchGuard. Maven/Gradle are only needed to prepare the project context or run tests.','Vollständiges JDK 17+ mit java auf PATH; Java-Release und Abhängigkeits-JARs müssen zum Projekt passen. Der javac-Scanner wird mit ArchGuard geliefert. Maven/Gradle werden nur zum Vorbereiten des Projektkontexts oder für Tests benötigt.'),
      setup:t('For Maven projects, export the existing dependency classpath. For Gradle or custom builds, supply the corresponding existing JARs/class directories through the configuration instead. ArchGuard does not run builds or annotation processors.','Bei Maven-Projekten den Abhängigkeits-Classpath exportieren. Bei Gradle oder eigenen Builds die passenden vorhandenen JARs/Klassenverzeichnisse über die Konfiguration angeben. ArchGuard startet keine Projektbuilds oder Annotation Processor.'),
      commands:'java -version\nmkdir -p .archguard\nmvn dependency:build-classpath -Dmdep.outputFile=.archguard/java-classpath.txt\narchguard calls . --format json',
      config:'[semantic.java]\nrelease = 21\nclasspath = ["lib/contracts.jar", "target/classes"]',
      configNote:t('Optional example in .archguard/archguard.toml: replace release and paths with your project values. Do not add nonexistent directories.','Optionales Beispiel in .archguard/archguard.toml: Release und Pfade an das Projekt anpassen. Keine nicht vorhandenen Verzeichnisse eintragen.'),
      coverage:{tool:'JaCoCo',format:'JaCoCo XML',note:t('Maven example with the JaCoCo plugin already configured in pom.xml. For Gradle, enable XML in jacocoTestReport. Import the XML report, not jacoco.exec or the HTML page.','Maven-Beispiel mit bereits in pom.xml konfiguriertem JaCoCo-Plugin. Für Gradle XML in jacocoTestReport aktivieren. Den XML-Bericht importieren, nicht jacoco.exec oder die HTML-Seite.'),commands:'mvn jacoco:prepare-agent test jacoco:report\narchguard coverage . --report target/site/jacoco/jacoco.xml --source-root src/main/java',links:[['JaCoCo: Maven','https://www.jacoco.org/jacoco/trunk/doc/maven.html'],['JaCoCo: XML report','https://www.jacoco.org/jacoco/trunk/doc/report-mojo.html'],['Maven: classpath','https://maven.apache.org/plugins/maven-dependency-plugin/build-classpath-mojo.html']]}},
    {id:'csharp',name:'C# / .NET',extensions:'.cs',compiler:'.NET SDK 8+',semantic:true,
      requirement:t('dotnet on PATH, .NET SDK 8+ and the reference packs required by the project target. Roslyn comes with the SDK; ArchGuard supplies its scanner. Restore project dependencies so obj/project.assets.json and the NuGet references exist.','dotnet auf PATH, .NET SDK 8+ sowie die Referenzpakete des Projekt-Targets. Roslyn kommt mit dem SDK; ArchGuard liefert den Scanner. Projektabhängigkeiten wiederherstellen, damit obj/project.assets.json und NuGet-Referenzen vorhanden sind.'),
      setup:t('Run restore yourself for the intended project or solution. ArchGuard does not start project builds, restore, analyzers or source generators. Check global.json and the selected target framework if analysis is partial.','Restore für das gewünschte Projekt bzw. die Solution selbst ausführen. ArchGuard startet keine Projektbuilds, Restores, Analyzer oder Source-Generatoren. Bei Teilabdeckung global.json und das ausgewählte Target-Framework prüfen.'),
      commands:'dotnet --list-sdks\ndotnet restore\narchguard calls . --format json',
      config:'[semantic.csharp]\nproject = "src/App/App.csproj"\nconfiguration = "Debug"\ntarget_framework = "net8.0"',
      configNote:t('Optional example in .archguard/archguard.toml; use your actual project path and target framework.','Optionales Beispiel in .archguard/archguard.toml; tatsächlichen Projektpfad und das passende Target-Framework einsetzen.'),
      coverage:{tool:'Coverlet',format:'Cobertura XML',note:t('Example for VSTest with coverlet.collector and a compatible Microsoft.NET.Test.Sdk in the test project. Microsoft.Testing.Platform uses a different integration. Replace Tests.csproj and RUN-ID with the actual paths.','Beispiel für VSTest mit coverlet.collector und passendem Microsoft.NET.Test.Sdk im Testprojekt. Microsoft.Testing.Platform verwendet eine andere Integration. Tests.csproj und RUN-ID durch die tatsächlichen Pfade ersetzen.'),commands:'dotnet test tests/Tests.csproj --collect:"XPlat Code Coverage" --results-directory TestResults\narchguard coverage . --report TestResults/RUN-ID/coverage.cobertura.xml',links:[['Coverlet: installation and integrations','https://github.com/coverlet-coverage/coverlet']]}},
    {id:'ts',name:'TypeScript',extensions:'.ts · .tsx',compiler:'Node.js + TypeScript',semantic:true,
      requirement:t('Node.js on PATH and the project’s installed typescript package. ArchGuard uses the nearest node_modules/typescript/lib/typescript.js, or the explicitly configured compiler, once ARCHGUARD_TRUST_PROJECT_COMPILER=1 marks the repository as trusted. Dependencies and tsconfig.json provide type resolution.','Node.js auf PATH und das im Projekt installierte Paket typescript. ArchGuard nutzt die nächste node_modules/typescript/lib/typescript.js oder den explizit konfigurierten Compiler, sobald ARCHGUARD_TRUST_PROJECT_COMPILER=1 das Repository als vertrauenswürdig markiert. Abhängigkeiten und tsconfig.json liefern die Typauflösung.'),
      setup:t('Use versions compatible with the project. Without tsconfig, only TypeScript is analyzed in a limited ES2020/CommonJS context. No application code, build scripts or compiler plugins are run by ArchGuard; the compiler itself comes from the project and only runs after the opt-in.','Zum Projekt passende Versionen verwenden. Ohne tsconfig wird nur TypeScript in einem eingeschränkten ES2020/CommonJS-Kontext analysiert. ArchGuard führt keinen Anwendungscode, keine Build-Skripte und keine Compiler-Plugins aus; der Compiler selbst stammt aus dem Projekt und läuft erst nach der Freigabe.'),
      commands:'node --version\nnpm install --save-dev typescript\narchguard calls . --format json',
      config:'[semantic.typescript]\nproject = "tsconfig.json"\ncompiler = "node_modules/typescript/lib/typescript.js"',configNote:t('Optional explicit configuration in .archguard/archguard.toml.','Optionale explizite Konfiguration in .archguard/archguard.toml.'),coverage:jsCoverage},
    {id:'js',name:'JavaScript',extensions:'.js · .jsx · .mjs',compiler:'Node.js + TypeScript + allowJs',semantic:true,
      requirement:t('JavaScript also needs Node.js and the installed TypeScript compiler for class/method KPIs. The actual tsconfig project must enable allowJs and include the JavaScript files. Installing Node.js alone is insufficient.','Auch JavaScript benötigt für Klassen-/Methoden-KPIs Node.js und den installierten TypeScript-Compiler. Das tatsächliche tsconfig-Projekt muss allowJs aktivieren und die JavaScript-Dateien einschließen. Node.js allein reicht nicht.'),
      setup:t('Merge the example into the project’s tsconfig, preserving its module, JSX and include settings. A jsconfig.json can be selected explicitly through [semantic.typescript].project. Dynamic or ambiguous targets remain unresolved.','Das Beispiel in die tsconfig des Projekts übernehmen; Modul-, JSX- und Include-Einstellungen beibehalten. Eine jsconfig.json lässt sich explizit über [semantic.typescript].project auswählen. Dynamische oder mehrdeutige Ziele bleiben unaufgelöst.'),
      commands:'node --version\nnpm install --save-dev typescript\narchguard calls . --format json',
      config:'{\n  "compilerOptions": { "allowJs": true, "noEmit": true },\n  "include": ["src/**/*"]\n}',configNote:t('Example for tsconfig.json; adapt include to the actual source paths.','Beispiel für tsconfig.json; include an die tatsächlichen Quellpfade anpassen.'),coverage:jsCoverage},
    {id:'py',name:'Python',extensions:'.py',compiler:'Python 3.9+',semantic:false,requirement:t('Python 3.9+ on PATH, or ARCHGUARD_PYTHON pointing to it. Classes, functions, Complexity, Cognitive and Halstead per function come from the standard ast module; project code is never imported. Calls and types stay unresolved, so CBO, RFC, DIT and NOC are not measured.','Python 3.9+ auf PATH oder ARCHGUARD_PYTHON mit dem Pfad dazu. Klassen, Funktionen, Complexity, Cognitive und Halstead pro Funktion stammen aus dem Standardmodul ast; Projektcode wird nie importiert. Aufrufe und Typen bleiben unaufgelöst, daher werden CBO, RFC, DIT und NOC nicht gemessen.'),
      coverage:{tool:'coverage.py + pytest',format:'Cobertura XML',note:t('Example for a pytest project with coverage.py and pytest installed in the active Python environment. Adapt the test runner and source selection to your project. --branch records branch data as well.','Beispiel für ein pytest-Projekt mit coverage.py und pytest in der aktiven Python-Umgebung. Test-Runner und Quellauswahl an das Projekt anpassen. --branch erfasst zusätzlich Zweigabdeckung.'),commands:'python -m coverage run --branch -m pytest\npython -m coverage xml -o coverage.xml\narchguard coverage . --report coverage.xml',links:[['coverage.py: XML','https://coverage.readthedocs.io/en/latest/commands/cmd_xml.html']]}},
    {id:'php',name:'PHP',extensions:'.php',compiler:null,semantic:false,requirement:noCompiler,
      coverage:{tool:'PHPUnit + Xdebug / PCOV',format:'Clover / Cobertura XML',note:t('PHPUnit needs a coverage driver in PHP CLI. The example uses an installed Xdebug with coverage mode. PCOV is an alternative for line coverage only. Configure the source filter in PHPUnit; a missing driver cannot produce measurements.','PHPUnit braucht einen Coverage-Treiber in PHP CLI. Das Beispiel nutzt ein installiertes Xdebug im Coverage-Modus. PCOV ist eine Alternative ausschließlich für Zeilenabdeckung. Den Quellfilter in PHPUnit konfigurieren; ohne Treiber entstehen keine Messwerte.'),commands:'php -d xdebug.mode=coverage vendor/bin/phpunit --coverage-clover coverage.xml\narchguard coverage . --report coverage.xml',links:[['PHPUnit: coverage drivers','https://docs.phpunit.de/en/12.5/code-coverage.html'],['PHPUnit: CLI','https://docs.phpunit.de/en/12.5/textui.html']]}},
    {id:'rs',name:'Rust',extensions:'.rs',compiler:null,semantic:false,requirement:t('No compiler needed. Complexity, Cognitive and Halstead per function come from syntax; calls and types stay unresolved, so CBO, RFC, DIT and NOC are not measured.','Kein Compiler nötig. Complexity, Cognitive und Halstead pro Funktion stammen aus der Syntax; Aufrufe und Typen bleiben unaufgelöst, daher werden CBO, RFC, DIT und NOC nicht gemessen.'),
      coverage:{tool:'cargo-llvm-cov',format:'LCOV',note:t('Requires the Rust toolchain, cargo-llvm-cov and the matching LLVM tools (usually the rustup component llvm-tools-preview). The command builds and runs the project tests. cargo-tarpaulin with Cobertura XML is another compatible producer.','Benötigt die Rust-Toolchain, cargo-llvm-cov und passende LLVM-Werkzeuge (üblicherweise die rustup-Komponente llvm-tools-preview). Der Aufruf baut das Projekt und führt Tests aus. cargo-tarpaulin mit Cobertura-XML ist ein weiterer kompatibler Producer.'),commands:'cargo llvm-cov --lcov --output-path lcov.info\narchguard coverage . --report lcov.info',links:[['cargo-llvm-cov: setup and reports','https://github.com/taiki-e/cargo-llvm-cov']]}}
  ];
}
function kpiHelpCommand(parent,command) {
  const block=element('div',undefined,'kpi-help-command');
  block.appendChild(element('pre',command));
  const copy=button(t('Copy','Kopieren'),()=>copyText(command),'btn btn-secondary');copy.setAttribute('aria-label',t('Copy command or configuration','Befehl oder Konfiguration kopieren'));block.appendChild(copy);parent.appendChild(block);
}
function kpiHelpLinks(parent,links) {
  const list=element('p',undefined,'kpi-help-links');
  for(const [name,url] of links){const link=element('a',name);link.setAttribute('href',url);link.setAttribute('target','_blank');link.setAttribute('rel','noopener noreferrer');list.appendChild(link);}parent.appendChild(list);
}
function kpiHelpSection(parent,title,description) {
  const section=element('section',undefined,'kpi-help-section');section.appendChild(element('h3',title));
  if(description)section.appendChild(element('p',description));parent.appendChild(section);return section;
}
// Keep entry points aligned with Command in archguard-cli/src/main.rs.
function cliHelpGroups(){return [
  {title:t('Everyday commands','Befehle für den Alltag'),commands:[
    ['doctor',t('Check installed external tools. Does not validate project setup; always exits 0.','Installierte externe Werkzeuge prüfen. Validiert nicht die Projekteinrichtung; beendet sich immer mit Exit-Code 0.')],
    ['init',t('Create project configuration once; existing configuration is not overwritten.','Projektkonfiguration einmalig anlegen; bestehende Konfiguration wird nicht überschrieben.')],
    ['serve',t('Start the local interactive viewer and refresh it after source/config changes. Best entry point for exploration.','Lokalen interaktiven Viewer starten und nach Quellcode-/Konfigurationsänderungen aktualisieren. Der Einstieg zum Erkunden.')],
    ['check',t('CI gate for cycles, layer rules and configured thresholds. Use the exit status to fail a pipeline.','CI-Prüfung für Zyklen, Schichtenregeln und konfigurierte Grenzwerte. Der Exit-Code kann die Pipeline fehlschlagen lassen.')],
    ['graph',t('Export a graph/report, including standalone HTML. Use for a saved report instead of a live server.','Graph/Bericht exportieren, auch als eigenständiges HTML. Für einen gespeicherten Bericht statt eines Live-Servers.')]
  ]},
  {title:t('Targeted source analysis','Gezielte Quellcodeanalyse'),commands:[
    ['scan',t('Extract the original graph as JSON with --out; useful for downstream tools.','Originalgraph mit --out als JSON extrahieren; für nachgelagerte Werkzeuge.')],
    ['cycles',t('List dependency cycles without a full CI check.','Abhängigkeitszyklen ohne vollständige CI-Prüfung auflisten.')],
    ['metrics',t('Read module coupling metrics; --extended includes additional metrics.','Kopplungsmetriken der Module lesen; --extended ergänzt weitere Metriken.')],
    ['hotspots',t('Rank review candidates using coupling, cycles and Git churn.','Prüfkandidaten anhand von Kopplung, Zyklen und Git-Änderungshäufigkeit priorisieren.')],
    ['co-change',t('Find source files that change together without a static import relationship. Needs Git history.','Gemeinsam geänderte Quelldateien ohne statische Importbeziehung finden. Benötigt Git-Historie.')],
    ['ownership',t('Inspect CODEOWNERS boundaries and configured forbidden team directions.','CODEOWNERS-Grenzen und konfigurierte verbotene Abhängigkeiten zwischen Teams prüfen.')],
    ['performance',t('Inspect structural load and heavy dependencies; not runtime profiling.','Strukturelle Last und schwere Abhängigkeiten untersuchen; kein Laufzeit-Profiling.')],
    ['calls',t('Read compiler-backed classes, methods, calls and analysis coverage/diagnostics.','Compilerbasierte Klassen, Methoden, Aufrufe sowie Analyseabdeckung und Diagnosen lesen.')],
    ['cohesion',t('Inspect cohesion of package members and candidates for splitting a package.','Zusammenhalt der Paketmitglieder und Kandidaten für eine Paketaufteilung untersuchen.')],
    ['patterns',t('Assess architecture pattern candidates from evidence. These are hypotheses, not a configured target architecture.','Architekturmuster-Kandidaten aus Belegen bewerten. Das sind Hypothesen, keine konfigurierte Zielarchitektur.')],
    ['dsm',t('Export/read the dependency-structure matrix of modules. The viewer also provides a matrix tab.','Abhängigkeitsmatrix der Module ausgeben/lesen. Der Viewer bietet ebenfalls einen Matrix-Tab.')]
  ]},
  {title:t('Optional evidence imports','Optionale Belegimporte'),commands:[
    ['coverage',t('Import JaCoCo, Cobertura, Clover or LCOV with --report; without it, read saved coverage. Does not run tests.','JaCoCo, Cobertura, Clover oder LCOV mit --report importieren; ohne diese Option gespeicherte Coverage lesen. Führt keine Tests aus.')],
    ['test-evidence',t('Import JUnit XML test results and optional source mappings; inspect affected-test recommendations.','JUnit-XML-Testergebnisse und optionale Quellzuordnungen importieren; Empfehlungen für betroffene Tests lesen.')],
    ['mutations',t('Import mutation-testing-elements JSON or read saved evidence. Does not run mutation tests.','mutation-testing-elements-JSON importieren oder gespeicherte Belege lesen. Führt keine Mutationstests aus.')],
    ['duplicates',t('Import CPD XML duplication groups or read the saved report. Does not invoke a duplicate scanner.','CPD-XML-Duplikatgruppen importieren oder den gespeicherten Bericht lesen. Startet keinen Duplikat-Scanner.')],
    ['quality',t('Import SARIF findings, read saved findings, or gate changed-code quality with --base and thresholds.','SARIF-Befunde importieren, gespeicherte Befunde lesen oder Qualität geänderten Codes mit --base und Grenzwerten prüfen.')],
    ['contracts',t('Import OpenAPI with --report and an exact --service ID; inspect saved contract comparisons.','OpenAPI mit --report und einer exakten --service-ID importieren; gespeicherte Vertragsvergleiche lesen.')],
    ['security',t('Run Trivy dependency-advisory scanning and save .archguard/security.json. Requires Trivy.','Mit Trivy Abhängigkeiten auf Sicherheitsmeldungen prüfen und .archguard/security.json speichern. Benötigt Trivy.')]
  ]},
  {title:t('Architecture, services & documentation','Architektur, Services & Dokumentation'),commands:[
    ['architecture',t('Compare explicitly modeled components and allowed directions with actual imports.','Explizit modellierte Komponenten und erlaubte Richtungen mit tatsächlichen Imports vergleichen.')],
    ['services',t('Discover projects, applications and Compose HTTP/startup dependencies.','Projekte, Anwendungen sowie Compose-HTTP-/Startabhängigkeiten entdecken.')],
    ['workspace',t('Inspect declared dependencies between local projects; --sources opts into source analysis.','Deklarierte Abhängigkeiten zwischen lokalen Projekten prüfen; --sources aktiviert zusätzlich die Quellcodeanalyse.')],
    ['doc',t('Generate arc42 Markdown. --stdout only prints it; --check checks whether the saved document is missing or outdated.','arc42-Markdown erzeugen. --stdout gibt es nur aus; --check prüft, ob das gespeicherte Dokument fehlt oder veraltet ist.')]
  ]},
  {title:t('Reviews, changes & history','Reviews, Änderungen & Verlauf'),commands:[
    ['review',t('Assemble findings, change impact and evidence gaps for a review, optionally against --base.','Befunde, Änderungsauswirkungen und Beleglücken für ein Review zusammenstellen, optional gegenüber --base.')],
    ['review-decision',t('record persists a review decision; show reads the decisions in .archguard/reviews.json.','record speichert eine Review-Entscheidung; show liest die Entscheidungen aus .archguard/reviews.json.')],
    ['diff',t('Compare Git refs with --from/--to or compare a ref with --staged. Does not compare the working tree.','Git-Referenzen mit --from/--to vergleichen oder eine Referenz mit --staged. Vergleicht nicht den Arbeitsbaum.')],
    ['suggest',t('Suggest cuts for cycles as review candidates, not automatic source edits.','Schnitte für Zyklen als Prüfkandidaten vorschlagen; keine automatischen Quellcodeänderungen.')],
    ['simulate',t('Simulate cut/move/merge operations and report their effect without changing source files.','Cut-/Move-/Merge-Operationen simulieren und ihre Wirkung ohne Änderungen an Quelldateien ausgeben.')],
    ['baseline',t('write accepts existing cycles as the baseline; diff compares against it to identify new cycles.','write übernimmt bestehende Zyklen als Baseline; diff vergleicht damit, um neue Zyklen zu erkennen.')],
    ['snapshot',t('Append a project summary snapshot to .archguard/history/.','Einen Projektzusammenfassungs-Snapshot an .archguard/history/ anhängen.')],
    ['trend',t('Read the summary snapshots and their deltas. Does not record a new measurement.','Zusammenfassungs-Snapshots und ihre Differenzen lesen. Zeichnet keine neue Messung auf.')],
    ['quality-history',t('Read per-package quality history; --record explicitly adds current measurements. More granular than snapshot/trend.','Qualitätsverlauf je Paket lesen; --record fügt explizit aktuelle Messungen hinzu. Detaillierter als snapshot/trend.')]
  ]},
  {title:t('Configuration & integrations','Konfiguration & Integrationen'),commands:[
    ['config',t('Show effective configuration and where its sections came from. Does not write configuration.','Wirksame Konfiguration und Herkunft ihrer Abschnitte anzeigen. Schreibt keine Konfiguration.')],
    ['db',t('sync mirrors analysis into the graph database; query runs openCypher. Requires a build with the db feature.','sync spiegelt die Analyse in die Graphdatenbank; query führt openCypher aus. Benötigt einen Build mit db-Feature.')],
    ['mcp',t('Start the stdio MCP server for assistant/tool integration; --root sets the project. This is not the web viewer.','Den stdio-MCP-Server für Assistenten-/Werkzeugintegration starten; --root setzt das Projekt. Das ist nicht der Web-Viewer.')],
    ['help',t('Show CLI usage and all options for a command.','CLI-Aufruf und alle Optionen eines Befehls anzeigen.')]
  ]}
];}
function renderCliHelp(parent){
  const section=kpiHelpSection(parent,t('CLI · which commands do I need?','CLI · welche Befehle brauche ich?'));
  section.setAttribute('data-cli-help','true');
  section.appendChild(element('p',t('For everyday use, start with doctor, init, serve, check and graph. The other entry points serve focused analyses, imports or automation; you do not need to run them all before opening the viewer.','Für den Alltag reichen meist doctor, init, serve, check und graph. Die anderen Einstiegspunkte dienen gezielten Analysen, Importen oder Automatisierung; du musst sie nicht alle ausführen, bevor du den Viewer öffnest.')));
  section.appendChild(element('p',t('Specialized commands expose individual analysis results to scripts and CI. Use only the ones relevant to your workflow.','Spezialbefehle machen einzelne Analyseergebnisse für Skripte und CI zugänglich. Nutze nur die Befehle, die zu deinem Ablauf passen.')));
  kpiHelpCommand(section,'archguard doctor\narchguard init .\narchguard serve . --port 8787');
  section.appendChild(element('p',t('Run from the project directory. init is a one-time optional configuration step; serve stays running until Ctrl+C. For CI or a standalone report, use a separate terminal:','Im Projektverzeichnis ausführen. init ist ein einmaliger optionaler Konfigurationsschritt; serve läuft bis Strg+C. Für CI oder einen eigenständigen Bericht ein separates Terminal verwenden:')));
  kpiHelpCommand(section,'archguard check .\narchguard graph . --format html');
  section.appendChild(element('p',t('The dot means the current project; replace it with a project path if needed. In a repository with several services, select the source project in the viewer or start serve with its path. External test/quality reports are optional imports, not prerequisites for the dependency graph.','Der Punkt steht für das aktuelle Projekt; bei Bedarf durch einen Projektpfad ersetzen. In einem Repository mit mehreren Services das Quellprojekt im Viewer wählen oder serve mit dessen Pfad starten. Externe Test-/Qualitätsberichte sind optionale Importe, keine Voraussetzung für den Abhängigkeitsgraphen.')));
  for(const group of cliHelpGroups()){
    const details=element('details');details.setAttribute('data-cli-group','true');details.appendChild(element('summary',group.title+' · '+group.commands.length));
    for(const [name,description] of group.commands){const entry=element('div');entry.setAttribute('data-cli-command',name);entry.appendChild(element('h4','archguard '+name));entry.appendChild(element('p',description));kpiHelpCommand(entry,'archguard '+name+' --help');details.appendChild(entry);}
    section.appendChild(details);
  }
  section.appendChild(element('p',t('Overlap is intentional: serve and graph provide interactive/saved views; individual analysis commands expose results to scripts. check enforces architecture rules, quality handles imported findings and changed-code thresholds, security runs vulnerability scanning. snapshot/trend track project summaries; quality-history tracks packages. See each command’s --help for required options and supported formats.','Überschneidungen sind bewusst: serve und graph liefern interaktive/gespeicherte Ansichten; einzelne Analysebefehle machen Ergebnisse für Skripte zugänglich. check prüft Architekturregeln, quality behandelt importierte Befunde und Grenzwerte für geänderten Code, security startet die Schwachstellenprüfung. snapshot/trend verfolgen Projektzusammenfassungen; quality-history verfolgt Pakete. Die jeweilige --help-Ausgabe nennt Pflichtoptionen und unterstützte Formate.')));
}

function mcpHelpPrompt(name){
  const examples={
    development_guide:['Use ArchGuard to give me an overview of this project. Where should I start to understand its architecture, and which analyses are available?','Gib mir mit ArchGuard einen Überblick über dieses Projekt. Wo sollte ich anfangen, um die Architektur zu verstehen, und welche Analysen sind verfügbar?'],
    scan:['Use ArchGuard to summarize the size and structure of this project: files, packages, dependencies, languages and cycles.','Fasse mit ArchGuard Größe und Struktur dieses Projekts zusammen: Dateien, Pakete, Abhängigkeiten, Sprachen und Zyklen.'],
    service_topology:['Use ArchGuard to show the services and projects in this repository. Distinguish applications, libraries and test tools, and explain their configured relationships.','Zeige mir mit ArchGuard die Services und Projekte dieses Repositorys. Unterscheide Anwendungen, Bibliotheken und Testwerkzeuge und erkläre ihre konfigurierten Beziehungen.'],
    workspace_context:['Use ArchGuard to examine dependencies between the projects in this monorepo, including available source evidence. Explain where cross-project analysis is incomplete.','Untersuche mit ArchGuard die Abhängigkeiten zwischen den Projekten dieses Monorepos einschließlich verfügbarer Quellbelege. Erkläre, wo die projektübergreifende Analyse unvollständig ist.'],
    architecture_atlas:['Use ArchGuard to show the main packages and their relationships. Then explore the package with the most connections and explain its responsibilities from the available evidence.','Zeige mir mit ArchGuard die wichtigsten Pakete und ihre Beziehungen. Vertiefe anschließend das Paket mit den meisten Verbindungen und erläutere seine Aufgaben anhand der verfügbaren Belege.'],
    architecture_world:['Use ArchGuard to explain the most complex classes in this project. Name the measured values and source files, and distinguish missing measurements from low complexity.','Erkläre mir mit ArchGuard die komplexesten Klassen dieses Projekts. Nenne Messwerte und Quelldateien und unterscheide fehlende Messungen von geringer Komplexität.'],
    graph_expand:['Use ArchGuard to expand the package we just discussed by one level. Show its direct subpackages and their dependencies.','Klappe mit ArchGuard das gerade besprochene Paket eine Ebene auf. Zeige seine direkten Unterpakete und deren Abhängigkeiten.'],
    dsm:['Use ArchGuard to show a dependency matrix for the package we just discussed. Explain the strongest connections and visible cycles.','Zeige mit ArchGuard eine Abhängigkeitsmatrix für das gerade besprochene Paket. Erkläre die stärksten Verbindungen und sichtbaren Zyklen.'],
    cycles_list:['Use ArchGuard to find the largest dependency cycles. Distinguish file cycles from cycles that only appear when packages are aggregated.','Finde mit ArchGuard die größten Abhängigkeitszyklen. Unterscheide Dateizyklen von Zyklen, die erst durch die Zusammenfassung zu Paketen entstehen.'],
    cycle_get:['Use ArchGuard to examine the cycle we just found. List its members and show a closed dependency path with the corresponding source locations.','Untersuche mit ArchGuard den gerade gefundenen Zyklus. Liste seine Mitglieder auf und zeige einen geschlossenen Abhängigkeitspfad mit den zugehörigen Quellstellen.'],
    module_neighborhood:['Use ArchGuard to show what depends directly on the package we just discussed and what that package itself depends on. Explain incoming and outgoing relationships separately.','Zeige mit ArchGuard, was direkt vom gerade besprochenen Paket abhängt und wovon dieses Paket selbst abhängt. Erkläre eingehende und ausgehende Beziehungen getrennt.'],
    edge_evidence:['Use ArchGuard to explain why the two packages we just discussed are connected. Show the direction and the import locations that establish this dependency.','Erkläre mit ArchGuard, warum die beiden gerade besprochenen Pakete verbunden sind. Zeige die Richtung und die Importstellen, die diese Abhängigkeit belegen.'],
    suggest_cuts:['Use ArchGuard to suggest three ways to break the cycle we just discussed. Explain the evidence and trade-offs for each suggestion without editing files.','Schlage mit ArchGuard drei Möglichkeiten vor, den gerade besprochenen Zyklus aufzubrechen. Erkläre jeweils die Belege und Nachteile, ohne Dateien zu ändern.'],
    simulate:['Use ArchGuard to simulate removing the dependency we just discussed. Compare cycles and coupling before and after, without changing source files.','Simuliere mit ArchGuard das Entfernen der gerade besprochenen Abhängigkeit. Vergleiche Zyklen und Kopplung vorher und nachher, ohne Quelldateien zu verändern.'],
    architecture_context:['Use ArchGuard to summarize the declared architecture goals, constraints and target structure. Clearly separate documented decisions from assumptions and missing information.','Fasse mit ArchGuard die dokumentierten Architekturziele, Einschränkungen und die Zielstruktur zusammen. Trenne dokumentierte Entscheidungen klar von Annahmen und fehlenden Angaben.'],
    architecture_document:['Use ArchGuard to produce the arc42 building block view for this project. Retrieve the complete chapter, explain its packages and mark information that cannot be inferred from the code.','Erstelle mit ArchGuard die arc42-Bausteinsicht dieses Projekts. Lade das vollständige Kapitel, erkläre die Pakete und kennzeichne Informationen, die sich nicht aus dem Code ableiten lassen.'],
    architecture_patterns:['Use ArchGuard to investigate which architecture patterns are supported by this project. For each candidate, show supporting evidence, counter-evidence and what is still unknown.','Prüfe mit ArchGuard, welche Architekturmuster in diesem Projekt belegbar sind. Zeige für jeden Kandidaten unterstützende Belege, Gegenbelege und offene Fragen.'],
    config_view:['Use ArchGuard to explain the architecture rules currently applied to this project. Show which settings were explicitly declared and which were derived or defaulted.','Erkläre mit ArchGuard die aktuell angewendeten Architekturregeln dieses Projekts. Zeige, welche Einstellungen ausdrücklich festgelegt, abgeleitet oder als Standard übernommen wurden.'],
    architecture_findings:['Use ArchGuard to find the current architecture problems. Prioritize the most important findings and explain their evidence, affected components and analysis gaps.','Finde mit ArchGuard die aktuellen Architekturprobleme. Priorisiere die wichtigsten Befunde und erkläre ihre Belege, betroffenen Komponenten und Analyselücken.'],
    architecture_finding:['Use ArchGuard to examine the finding we just discussed in detail. Show the relevant source locations, the violated rule and possible repair steps.','Untersuche mit ArchGuard den gerade besprochenen Befund im Detail. Zeige die relevanten Quellstellen, die verletzte Regel und mögliche Reparaturschritte.'],
    check:['Use ArchGuard to check this project against its architecture rules and configured baseline. Explain which findings fail the check and which are covered by the baseline.','Prüfe dieses Projekt mit ArchGuard gegen seine Architekturregeln und die konfigurierte Baseline. Erkläre, welche Befunde die Prüfung scheitern lassen und welche durch die Baseline abgedeckt sind.'],
    baseline_diff:['Use ArchGuard to compare the current cycles with the existing baseline. Which cycles are new, which have disappeared, and is a usable baseline available?','Vergleiche mit ArchGuard die aktuellen Zyklen mit der vorhandenen Baseline. Welche Zyklen sind neu, welche verschwunden, und ist eine nutzbare Baseline vorhanden?'],
    metrics:['Use ArchGuard to compare the coupling and instability of the main packages. Explain the values and identify packages that deserve a closer look.','Vergleiche mit ArchGuard Kopplung und Instabilität der wichtigsten Pakete. Erkläre die Werte und benenne Pakete, die genauer untersucht werden sollten.'],
    quality_metrics:['Use ArchGuard to show the available class metrics and complexity measurements. Explain conspicuous values with source evidence and identify missing measurements.','Zeige mit ArchGuard die verfügbaren Klassenmetriken und Komplexitätsmessungen. Erkläre auffällige Werte mit Quellbelegen und benenne fehlende Messungen.'],
    review_kpis:['Use ArchGuard to summarize the available architecture review KPIs. Explain the most concerning measured values and which KPIs could not be measured.','Fasse mit ArchGuard die verfügbaren Architektur-Review-KPIs zusammen. Erkläre die auffälligsten gemessenen Werte und welche KPIs nicht gemessen werden konnten.'],
    semantic_model:['Use ArchGuard to show the methods and callers of the class we just discussed. Distinguish compiler-resolved calls from unresolved or external targets.','Zeige mit ArchGuard die Methoden und Aufrufer der gerade besprochenen Klasse. Unterscheide compileraufgelöste Aufrufe von ungeklärten oder externen Zielen.'],
    package_cohesion:['Use ArchGuard to examine the cohesion of the package we just discussed. Are there disconnected groups of classes, and what evidence would support splitting the package?','Untersuche mit ArchGuard den Zusammenhalt des gerade besprochenen Pakets. Gibt es getrennte Klassengruppen, und welche Belege würden für eine Aufteilung sprechen?'],
    hotspots:['Use ArchGuard to identify ten review priorities from dependencies, cycles and Git changes. Explain why each is ranked highly without treating the ranking as proof of a defect.','Ermittle mit ArchGuard zehn Review-Schwerpunkte anhand von Abhängigkeiten, Zyklen und Git-Änderungen. Erkläre die jeweilige Priorität, ohne die Rangfolge als Fehlernachweis darzustellen.'],
    co_change:['Use ArchGuard to find files that are frequently changed together despite having no recorded direct dependency. Explain the Git evidence and possible reasons.','Finde mit ArchGuard Dateien, die häufig gemeinsam geändert werden, obwohl keine direkte Abhängigkeit erfasst ist. Erkläre die Git-Belege und mögliche Ursachen.'],
    ownership:['Use ArchGuard to show team ownership from CODEOWNERS. Identify unassigned files and dependencies across team boundaries.','Zeige mit ArchGuard die Team-Zuständigkeiten aus CODEOWNERS. Benenne nicht zugeordnete Dateien und Abhängigkeiten über Teamgrenzen hinweg.'],
    performance:['Use ArchGuard to investigate structural performance risks such as large dependency closures and heavy imports. Distinguish static indicators from measured runtime performance.','Untersuche mit ArchGuard strukturelle Performance-Risiken wie große Abhängigkeitsketten und schwere Imports. Unterscheide statische Hinweise von gemessener Laufzeitperformance.'],
    change_context:['Use ArchGuard to assess my uncommitted changes against HEAD. Which components depend on the changed code, and which architecture constraints should I consider?','Bewerte mit ArchGuard meine noch nicht committeten Änderungen gegenüber HEAD. Welche Komponenten hängen vom geänderten Code ab, und welche Architekturvorgaben muss ich beachten?'],
    staged_architecture_diff:['Use ArchGuard to review only my staged changes against HEAD. Which architecture findings and dependency changes would this commit introduce?','Prüfe mit ArchGuard ausschließlich meine vorgemerkten Änderungen gegenüber HEAD. Welche Architekturbefunde und Abhängigkeitsänderungen würde dieser Commit einführen?'],
    compare:['Use ArchGuard to compare my working tree with HEAD. Explain new and resolved architecture findings and changes in coupling.','Vergleiche mit ArchGuard meinen Arbeitsstand mit HEAD. Erkläre neue und behobene Architekturbefunde sowie Änderungen der Kopplung.'],
    review_context:['Use ArchGuard to summarize the last saved architecture review. Which findings and decisions remain relevant, and is the review still current?','Fasse mit ArchGuard das letzte gespeicherte Architekturreview zusammen. Welche Befunde und Entscheidungen sind noch relevant, und ist das Review noch aktuell?'],
    review_decisions:['Use ArchGuard to list the recorded review decisions. Which findings were deferred or accepted as risks, and which explanations and ADR references were recorded? Do not change the register.','Liste mit ArchGuard die erfassten Review-Entscheidungen auf. Welche Befunde wurden vertagt oder als Risiko akzeptiert, und welche Begründungen und ADR-Verweise wurden hinterlegt? Ändere das Register nicht.'],
    pr_comment:['Use ArchGuard to draft an architecture review comment for my changes against HEAD. Include new and resolved findings and recorded accepted risks, but do not publish the comment.','Entwirf mit ArchGuard einen Architektur-Review-Kommentar für meine Änderungen gegenüber HEAD. Nenne neue und behobene Befunde sowie dokumentierte akzeptierte Risiken, veröffentliche den Kommentar aber nicht.'],
    quality_history:['Use ArchGuard to compare the recorded quality history of the packages. Which measured values improved or worsened, and which comparisons are not reliable?','Vergleiche mit ArchGuard den aufgezeichneten Qualitätsverlauf der Pakete. Welche Messwerte wurden besser oder schlechter, und welche Vergleiche sind nicht belastbar?'],
    trend:['Use ArchGuard to show the trend in cycles, coupling and dependency counts over the last ten recorded snapshots. Explain any gaps in the history.','Zeige mit ArchGuard die Entwicklung von Zyklen, Kopplung und Abhängigkeitszahlen über die letzten zehn aufgezeichneten Snapshots. Erkläre mögliche Lücken im Verlauf.'],
    quality_findings:['Use ArchGuard to summarize the imported quality findings and show the most relevant source locations. Identify stale reports and do not start a new scan.','Fasse mit ArchGuard die importierten Qualitätsbefunde zusammen und zeige die wichtigsten Quellstellen. Benenne veraltete Berichte und starte keinen neuen Scan.'],
    security_findings:['Use ArchGuard to summarize the saved dependency vulnerability findings. Show affected packages, versions and available fixes, and state when the scan was recorded.','Fasse mit ArchGuard die gespeicherten Schwachstellenbefunde zu Abhängigkeiten zusammen. Zeige betroffene Pakete, Versionen und verfügbare Korrekturen sowie den Zeitpunkt des Scans.'],
    duplication_findings:['Use ArchGuard to show the largest imported code duplication groups. List the affected files and explain where shared code might be worth considering.','Zeige mit ArchGuard die größten importierten Code-Duplikatgruppen. Liste die betroffenen Dateien auf und erläutere, wo gemeinsamer Code sinnvoll sein könnte.'],
    test_coverage:['Use ArchGuard to identify important files with test coverage gaps, considering how much code depends on them. Distinguish missing coverage data from measured uncovered lines.','Ermittle mit ArchGuard wichtige Dateien mit Testabdeckungslücken und berücksichtige, wie viel Code von ihnen abhängt. Unterscheide fehlende Abdeckungsdaten von gemessenen, nicht abgedeckten Zeilen.'],
    test_impact:['Use ArchGuard to recommend tests for the file we just discussed. Explain the recorded source mappings and dependencies, and which additional tests cannot safely be ruled out.','Empfiehl mit ArchGuard Tests für die gerade besprochene Datei. Erkläre die erfassten Quellzuordnungen und Abhängigkeiten und welche weiteren Tests nicht sicher ausgeschlossen werden können.'],
    mutation_findings:['Use ArchGuard to examine the imported mutation test results. Show surviving mutations and their source locations without executing new mutation tests.','Untersuche mit ArchGuard die importierten Mutationstest-Ergebnisse. Zeige überlebende Mutationen und ihre Quellstellen, ohne neue Mutationstests auszuführen.'],
    api_contracts:['Use ArchGuard to compare the imported API contract with its previous import. Explain contract changes and identify declared consumers that may be affected.','Vergleiche mit ArchGuard den importierten API-Vertrag mit dem vorherigen Import. Erkläre Vertragsänderungen und benenne deklarierte Verbraucher, die betroffen sein könnten.'],
    graph_query:['If the ArchGuard graph database tool is available and a synchronized database exists, list five module names with a read-only query. Otherwise explain what is missing.','Falls das ArchGuard-Graphdatenbank-Tool verfügbar ist und eine synchronisierte Datenbank vorliegt, liste mit einer lesenden Abfrage fünf Modulnamen auf. Erkläre andernfalls, was fehlt.']
  };
  return t(...examples[name]);
}

function mcpHelpGroups(){
  const tool=(name,en,de,args={})=>({name,description:t(en,de),prompt:mcpHelpPrompt(name),arguments:args});
  return [
    {title:t('Start & project structure','Einstieg & Projektstruktur'),tools:[
      tool('development_guide','Task-oriented routes through the tools and current analysis coverage.','Aufgabenbezogene Einstiege in die Tools und aktuelle Analyseabdeckung.',{limit:5}),
      tool('scan','Count nodes, dependencies, languages and cycles.','Knoten, Abhängigkeiten, Sprachen und Zyklen zählen.',{level:'module'}),
      tool('service_topology','Discover projects, applications and configured service relationships.','Projekte, Anwendungen und konfigurierte Service-Beziehungen erkennen.'),
      tool('workspace_context','Read dependencies between local projects. sources enables bounded source analysis; no builds are run.','Abhängigkeiten zwischen lokalen Projekten lesen. sources aktiviert begrenzte Quellanalyse; Builds werden nicht ausgeführt.',{sources:true,limit:5}),
      tool('architecture_atlas','Explore packages and their connections; use returned area names to drill down.','Pakete und ihre Verbindungen erkunden; zurückgegebene Bereichsnamen zum Vertiefen verwenden.',{limit:5}),
      tool('architecture_world','Inspect class/file evidence and complexity. Missing measurements remain unknown.','Klassen-/Dateibelege und Komplexität ansehen. Fehlende Messwerte bleiben unbekannt.',{limit:5}),
      tool('graph_expand','Expand one exact module name from the graph.','Einen exakten Modulnamen aus dem Graphen aufklappen.',{node:'app',expanded:[]}),
      tool('dsm','Read the dependency matrix; within optionally selects a package.','Abhängigkeitsmatrix lesen; within wählt optional ein Paket.',{within:'app'})
    ]},
    {title:t('Dependencies & cycles','Abhängigkeiten & Zyklen'),tools:[
      tool('cycles_list','List cycle IDs; level=file distinguishes file cycles from module aggregation.','Zyklus-IDs auflisten; level=file unterscheidet Dateizyklen von Modulaggregation.',{level:'file',limit:5}),
      tool('cycle_get','Read members and source evidence for an ID returned by cycles_list.','Mitglieder und Quellbelege zu einer ID aus cycles_list lesen.',{cycle_id:'scc-1'}),
      tool('module_neighborhood','Follow incoming, outgoing or both dependency directions.','Eingehende, ausgehende oder beide Abhängigkeitsrichtungen verfolgen.',{module:'app',depth:1,direction:'both'}),
      tool('edge_evidence','Read import locations behind one directed module connection.','Importstellen hinter einer gerichteten Modulverbindung lesen.',{from:'app',to:'domain'}),
      tool('suggest_cuts','Suggest dependency cuts for a returned cycle ID; does not edit sources.','Abhängigkeitsschnitte für eine zurückgegebene Zyklus-ID vorschlagen; ändert keine Quellen.',{cycle_id:'scc-1',max:3}),
      tool('simulate','Evaluate a virtual refactoring on a graph copy without editing files.','Virtuelles Refactoring auf einer Graphkopie bewerten, ohne Dateien zu ändern.',{ops:[{cut_edge:{from:'app',to:'domain'}}]})
    ]},
    {title:t('Architecture & documentation','Architektur & Dokumentation'),tools:[
      tool('architecture_context','Read declared goals, constraints, target model and missing context.','Deklarierte Ziele, Einschränkungen, Zielmodell und fehlenden Kontext lesen.'),
      tool('architecture_document','Return evidence-backed arc42 Markdown; concatenate pages in order. No file is written.','Belegtes arc42-Markdown liefern; Seiten in Reihenfolge zusammenfügen. Schreibt keine Datei.',{template:'arc42',chapter:5,limit:1500}),
      tool('architecture_patterns','Inspect pattern hypotheses with supporting and contradicting evidence.','Architekturhypothesen mit unterstützenden und widersprechenden Belegen prüfen.',{limit:5}),
      tool('config_view','Show effective configuration and its provenance.','Wirksame Konfiguration und ihre Herkunft anzeigen.',{section:'layers'}),
      tool('architecture_findings','Find current problems, stable finding IDs and evidence freshness.','Aktuelle Probleme, stabile Befund-IDs und Aktualität der Belege finden.',{kind:'cycle',limit:5}),
      tool('architecture_finding','Read evidence and repair context for a finding_id from architecture_findings.','Belege und Reparaturkontext zu einer finding_id aus architecture_findings lesen.',{finding_id:'<finding_id>',limit:5}),
      tool('check','Check architecture rules, optionally considering the configured baseline.','Architekturregeln prüfen, optional mit konfigurierter Baseline.',{use_baseline:true}),
      tool('baseline_diff','Compare current cycles with an existing baseline. Requires a baseline file/configuration.','Aktuelle Zyklen mit bestehender Baseline vergleichen. Benötigt eine Baseline-Datei/-Konfiguration.',{path:'.'})
    ]},
    {title:t('Metrics & code understanding','Metriken & Codeverständnis'),tools:[
      tool('metrics','Read module coupling and instability.','Modulkopplung und Instabilität lesen.',{top:10}),
      tool('quality_metrics','Read class/source metrics with explicit measurement coverage.','Klassen-/Quellmetriken mit ausgewiesener Messabdeckung lesen.',{limit:5}),
      tool('review_kpis','Read available review KPIs; unmeasured values are not zero.','Verfügbare Review-KPIs lesen; nicht gemessene Werte sind nicht null.',{limit:5}),
      tool('semantic_model','Inspect compiler-backed classes, methods or calls; provider gaps remain visible.','Compilerbelegte Klassen, Methoden oder Aufrufe prüfen; Abdeckungslücken bleiben sichtbar.',{kind:'classes',limit:5}),
      tool('package_cohesion','Inspect structural package cohesion and disconnected groups.','Strukturellen Paketzusammenhalt und getrennte Gruppen prüfen.',{limit:5}),
      tool('hotspots','Rank review candidates using dependency and Git evidence.','Review-Kandidaten anhand von Abhängigkeits- und Git-Belegen priorisieren.',{top:10,since:'180.days'}),
      tool('co_change','Find Git co-change candidates; shared changes do not prove a dependency.','Gemeinsam geänderte Dateien in Git finden; gemeinsame Änderungen beweisen keine Abhängigkeit.',{top:10}),
      tool('ownership','Read CODEOWNERS assignments and cross-team dependencies.','CODEOWNERS-Zuordnungen und teamübergreifende Abhängigkeiten lesen.'),
      tool('performance','Review structural dependency costs; does not measure runtime performance.','Strukturelle Abhängigkeitskosten prüfen; misst keine Laufzeitperformance.')
    ]},
    {title:t('Changes & review','Änderungen & Review'),tools:[
      tool('change_context','Read dependencies, policies and impact for files changed since a Git base.','Abhängigkeiten, Regeln und Auswirkungen für seit einer Git-Basis geänderte Dateien lesen.',{base:'HEAD',depth:1,limit:5}),
      tool('staged_architecture_diff','Compare the complete staged index with HEAD; excludes unstaged changes.','Den vollständigen vorgemerkten Index mit HEAD vergleichen; ohne nicht vorgemerkte Änderungen.',{base:'HEAD',limit:5}),
      tool('compare','Compare the working tree against a Git ref; optional target compares two exact branch tips.','Arbeitsstand mit einer Git-Referenz vergleichen; optional vergleicht target zwei exakte Branch-Stände.',{base:'HEAD'}),
      tool('review_context','Read the last saved review and check its freshness. Create it with archguard review --save.','Letztes gespeichertes Review lesen und Aktualität prüfen. Mit archguard review --save erzeugen.',{limit:5}),
      tool('review_decisions','List decisions. action=record writes .archguard/reviews.json and requires finding and decision; accept-risk may require adr. This example only reads.','Entscheidungen auflisten. action=record schreibt .archguard/reviews.json und benötigt finding und decision; accept-risk kann adr erfordern. Dieses Beispiel liest nur.',{action:'list',limit:5}),
      tool('pr_comment','Return a Markdown PR summary against a Git base; does not post a comment.','Markdown-PR-Zusammenfassung gegenüber einer Git-Basis liefern; veröffentlicht keinen Kommentar.',{base:'HEAD',limit:5}),
      tool('quality_history','Read explicitly recorded package metric history; does not create snapshots.','Explizit aufgezeichneten Verlauf der Paketmetriken lesen; erzeugt keine Snapshots.',{limit:5}),
      tool('trend','Read the snapshot history recorded by archguard snapshot.','Mit archguard snapshot aufgezeichneten Verlauf lesen.',{last:10})
    ]},
    {title:t('Imported quality, tests & security','Importierte Qualität, Tests & Sicherheit'),tools:[
      tool('quality_findings','Read saved SARIF findings; does not start linters or import reports.','Gespeicherte SARIF-Befunde lesen; startet keine Linter und importiert keine Berichte.',{limit:5}),
      tool('security_findings','Read saved dependency advisories. Refresh explicitly with archguard security.','Gespeicherte Schwachstellenmeldungen zu Abhängigkeiten lesen. Explizit mit archguard security aktualisieren.',{limit:5}),
      tool('duplication_findings','Read imported CPD groups and source occurrences.','Importierte CPD-Duplikatgruppen und Quellstellen lesen.',{limit:5}),
      tool('test_coverage','Read imported coverage and uncovered-file priorities; does not run tests.','Importierte Testabdeckung und Prioritäten nicht abgedeckter Dateien lesen; führt keine Tests aus.',{limit:5}),
      tool('test_impact','Read imported test identities/durations; targets optionally requests impact-based recommendations.','Importierte Testnamen/-laufzeiten lesen; targets fordert optional Empfehlungen anhand betroffener Quellen an.',{limit:5}),
      tool('mutation_findings','Read imported mutation outcomes; does not execute mutation tests.','Importierte Mutationsergebnisse lesen; führt keine Mutationstests aus.',{limit:5}),
      tool('api_contracts','Read imported OpenAPI contracts, changes and declared consumers.','Importierte OpenAPI-Verträge, Änderungen und deklarierte Verbraucher lesen.',{limit:5})
    ]},
    {title:t('Optional database build','Optionaler Datenbank-Build'),tools:[
      tool('graph_query','Available only with the db build feature. First create the mirror with archguard db sync; queries are read-only. tools/list shows whether this tool is available.','Nur mit Build-Feature db verfügbar. Zuerst Spiegel mit archguard db sync erzeugen; Abfragen lesen nur. tools/list zeigt, ob das Tool verfügbar ist.',{cypher:'MATCH (m:module) RETURN m.name LIMIT 5'})
    ]}
  ];
}
function renderMcpHelp(parent){
  const section=kpiHelpSection(parent,t('MCP · what to ask your agent','MCP · so fragst du deinen Agenten'));
  section.setAttribute('data-mcp-help','true');
  section.appendChild(element('p',t('Copy a request below into your agent chat. ArchGuard must be connected through MCP. The agent selects the matching tools and retrieves the project evidence; you do not need to enter JSON.','Kopiere eine der folgenden Anfragen in deinen Agenten-Chat. ArchGuard muss dort über MCP verbunden sein. Der Agent wählt die passenden Tools und lädt die Projektbelege; du musst kein JSON eingeben.')));
  section.appendChild(element('p',t('References such as “the package we just discussed” refer to your conversation. Alternatively, insert an exact package, class or file name. In a monorepo, name the service to analyze. Missing measurements and incomplete results should remain explicit.','Formulierungen wie „das gerade besprochene Paket“ beziehen sich auf euren Gesprächskontext. Alternativ den konkreten Paket-, Klassen- oder Dateinamen einsetzen. Im Monorepo den gewünschten Service nennen. Fehlende Messwerte und unvollständige Ergebnisse sollen ausdrücklich erkennbar bleiben.')));
  for(const group of mcpHelpGroups()){
    const details=element('details');details.setAttribute('data-mcp-group','true');details.appendChild(element('summary',group.title+' · '+group.tools.length));
    for(const entry of group.tools){
      const row=element('div');row.setAttribute('data-mcp-tool',entry.name);
      row.appendChild(element('h4',entry.name));row.appendChild(element('p',entry.description));
      const prompt=element('div');prompt.setAttribute('data-mcp-prompt',entry.name);kpiHelpCommand(prompt,entry.prompt);row.appendChild(prompt);
      const api=element('details');api.setAttribute('data-mcp-api',entry.name);api.appendChild(element('summary',t('Technical tool call','Technischer Tool-Aufruf')));
      kpiHelpCommand(api,JSON.stringify({name:entry.name,arguments:entry.arguments},null,2));row.appendChild(api);details.appendChild(row);
    }
    section.appendChild(details);
  }
  const technical=element('details');technical.setAttribute('data-mcp-setup','true');technical.appendChild(element('summary',t('MCP setup & technical reference','MCP-Einrichtung & technische Referenz')));
  technical.appendChild(element('p',t('Start the local stdio MCP server for your project. Your MCP client launches this command; these tool names are not CLI subcommands.','Lokalen MCP-Server über stdio für dein Projekt starten. Dein MCP-Client startet diesen Befehl; die Toolnamen sind keine CLI-Unterbefehle.')));
  kpiHelpCommand(technical,'archguard mcp --root /path/to/project');
  technical.appendChild(element('p',t('For a client with an mcpServers configuration, use this example and replace the project path. Use an absolute executable path if archguard is not on the client’s PATH.','Für Clients mit mcpServers-Konfiguration dieses Beispiel verwenden und den Projektpfad ersetzen. Falls archguard nicht im PATH des Clients liegt, den absoluten Programmpfad einsetzen.')));
  kpiHelpCommand(technical,JSON.stringify({mcpServers:{archguard:{command:'archguard',args:['mcp','--root','/path/to/project']}}},null,2));
  technical.appendChild(element('p',t('After MCP initialization, tools/list returns the tools and full input schemas of your installed build. The technical examples are params objects for tools/call. The agent chooses the tools and fills in these arguments.','Nach der MCP-Initialisierung liefert tools/list die Tools und vollständigen Eingabeschemas deiner installierten Version. Die technischen Beispiele sind params-Objekte für tools/call. Der Agent wählt die Tools und setzt diese Argumente ein.')));
  kpiHelpCommand(technical,JSON.stringify({jsonrpc:'2.0',id:1,method:'tools/call',params:{name:'development_guide',arguments:{limit:5}}},null,2));
  technical.appendChild(element('p',t('Example names app/domain, scc-1 and <finding_id> are placeholders: use exact names and IDs returned by your project. Git examples require a Git repository with HEAD. Paths resolve inside the configured server root. For a service in a monorepo, point --root to that service or use a supported path argument.','Beispielnamen app/domain, scc-1 und <finding_id> sind Platzhalter: exakte Namen und IDs aus deinem Projekt verwenden. Git-Beispiele benötigen ein Git-Repository mit HEAD. Pfade werden innerhalb des konfigurierten Server-Roots aufgelöst. Für einen Service im Monorepo --root auf diesen Service setzen oder ein unterstütztes path-Argument verwenden.')));
  technical.appendChild(element('p',t('Start with development_guide, architecture_atlas or architecture_findings. Follow a returned finding_id with architecture_finding, or a cycle ID with cycle_get and suggest_cuts.','Mit development_guide, architecture_atlas oder architecture_findings beginnen. Eine zurückgegebene finding_id mit architecture_finding vertiefen; eine Zyklus-ID mit cycle_get und suggest_cuts.')));
  technical.appendChild(element('p',t('Pagination: keep filters unchanged and use the returned next_offset as offset until it is null. For tools supporting expected_revision, pass metadata.revision from the first page; if it changes, restart. Do not assume a short or truncated response is complete. architecture_document uses character offsets. Missing or stale imported evidence does not mean zero findings.','Seitennavigation: Filter unverändert lassen und den zurückgegebenen next_offset als offset verwenden, bis er null ist. Bei Tools mit expected_revision den Wert metadata.revision der ersten Seite mitsenden; bei Änderungen neu beginnen. Kurze oder gekürzte Antworten sind nicht automatisch vollständig. architecture_document verwendet Zeichenpositionen. Fehlende oder veraltete Importbelege bedeuten nicht null Befunde.')));
  section.appendChild(technical);
}

function renderKpiHelp() {
  if(!kpiHelpUi.dialog)return;
  const {dialog,body}=kpiHelpUi,scroll=body.scrollTop||0;
  kpiHelpUi.action.textContent=t('KPI help','KPI-Hilfe');
  kpiHelpUi.title.textContent=t('CLI, MCP · Languages, tools & KPIs','CLI, MCP, Sprachen, Werkzeuge & KPIs');
  kpiHelpUi.close.textContent=t('Close','Schließen');
  kpiHelpUi.de.setAttribute('aria-pressed',String(viewerLanguage==='de'));kpiHelpUi.en.setAttribute('aria-pressed',String(viewerLanguage==='en'));
  body.textContent='';
  const intro=element('div',undefined,'kpi-help-intro');intro.appendChild(element('p',t('What to install, which reports to generate, and where the measurements appear. Choose the source language below.','Was du installieren musst, welche Berichte du erzeugst und wo die Messwerte erscheinen. Wähle unten die Quellsprache.')));body.appendChild(intro);
  renderCliHelp(body);
  renderMcpHelp(body);
  const cards=element('div',undefined,'kpi-help-cards');
  for(const [title,description] of [
    [t('01 · Graph KPIs','01 · Graph-KPIs'),t('Included: packages, dependencies, cycles, In/Out, ACD, CCD and NCCD. No separate scanner required. Available compiler evidence enriches the graph.','Enthalten: Pakete, Abhängigkeiten, Zyklen, Ein/Aus, ACD, CCD und NCCD. Kein separater Scanner nötig. Verfügbare Compiler-Belege ergänzen den Graphen.')],
    [t('02 · Classes & methods','02 · Klassen & Methoden'),t('Java, C# and TypeScript/JavaScript need the matching compiler context. ArchGuard calculates the metrics; no SonarQube or separate metrics service is required.','Java, C# und TypeScript/JavaScript benötigen den passenden Compilerkontext. ArchGuard berechnet die Metriken selbst; SonarQube oder ein separater Metrikdienst ist nicht erforderlich.')],
    [t('03 · Test coverage','03 · Testabdeckung'),t('Run tests with a coverage tool, then import its report. ArchGuard reads JaCoCo, Cobertura, Clover and LCOV. This is optional and separate from compiler setup.','Tests mit einem Coverage-Werkzeug ausführen und dessen Bericht importieren. ArchGuard liest JaCoCo, Cobertura, Clover und LCOV. Das ist optional und unabhängig vom Compiler-Setup.')]
  ])kpiHelpSection(cards,title,description);body.appendChild(cards);
  const overview=kpiHelpSection(body,t('Requirements by language','Voraussetzungen je Sprache'),t('The listed coverage tools are compatible examples, not additional requirements for every KPI.','Die genannten Coverage-Werkzeuge sind kompatible Beispiele, keine zusätzlichen Voraussetzungen für alle KPIs.'));
  const tableWrap=element('div',undefined,'kpi-help-table-wrap'),table=element('table'),thead=element('thead'),header=element('tr');
  table.setAttribute('aria-label',t('Language requirements','Sprachvoraussetzungen'));
  for(const label of [t('Language','Sprache'),t('Import graph','Importgraph'),t('Classes / methods','Klassen / Methoden'),t('Optional coverage tool','Optionales Coverage-Werkzeug')]){const th=element('th',label);th.setAttribute('scope','col');header.appendChild(th);}thead.appendChild(header);table.appendChild(thead);
  const rows=element('tbody'),languages=kpiLanguageHelp();
  for(const language of languages){
    const tr=element('tr');tr.setAttribute('data-kpi-language-row',language.id);if(language.id===kpiHelpUi.language)tr.className='is-selected';
    const name=element('th');name.setAttribute('scope','row');
    const select=button(language.name,()=>{kpiHelpUi.language=language.id;renderKpiHelp();kpiHelpUi.detail.scrollIntoView?.({block:'start'});kpiHelpUi.detail.focus({preventScroll:true});},'btn btn-secondary');select.setAttribute('data-kpi-language',language.id);select.setAttribute('aria-pressed',String(language.id===kpiHelpUi.language));name.appendChild(select);name.appendChild(element('small',language.extensions));tr.appendChild(name);
    tr.appendChild(element('td',t('Included','Enthalten')));tr.appendChild(element('td',language.compiler||t('Not supported yet','Noch nicht unterstützt')));
    const coverage=element('td',language.coverage.tool);coverage.appendChild(element('small',language.coverage.format));tr.appendChild(coverage);rows.appendChild(tr);
  }table.appendChild(rows);tableWrap.appendChild(table);overview.appendChild(tableWrap);
  overview.appendChild(element('p',t('Only the languages and extensions above are scanned. Kotlin, C/C++ and Swift are not currently supported; a compatible coverage format alone does not add source-language support.','Gescannt werden nur die Sprachen und Endungen oben. Kotlin, C/C++ und Swift werden derzeit nicht unterstützt; ein kompatibles Coverage-Format allein ergänzt keine Quellsprachen-Unterstützung.'),'muted'));
  const chosen=languages.find(row=>row.id===kpiHelpUi.language)||languages[0];
  const detail=kpiHelpSection(body,chosen.name+' · '+t('Setup & import','Einrichtung & Import'));detail.setAttribute('data-kpi-detail',chosen.id);detail.setAttribute('tabindex','-1');kpiHelpUi.detail=detail;
  const columns=element('div',undefined,'kpi-help-columns');detail.appendChild(columns);
  const compiler=kpiHelpSection(columns,t('Class and method KPIs','Klassen- und Methoden-KPIs'),chosen.requirement);
  if(chosen.semantic){
    compiler.appendChild(element('p',t('Provides applicable CBO, RFC, DIT, NOC, WMC, LCOM4, method complexity/nesting, Cognitive, Halstead and Maintainability Index when the necessary evidence is available. See Quality and Calls; World uses the same source metrics.','Liefert die jeweils anwendbaren CBO, RFC, DIT, NOC, WMC, LCOM4, Methodenkomplexität/Verschachtelung, Cognitive, Halstead und Maintainability Index bei vorhandenen Belegen. Anzeige unter Qualität und Aufrufe; World verwendet dieselben Quellmetriken.')));
    compiler.appendChild(element('p',chosen.setup));kpiHelpCommand(compiler,chosen.commands);
    if(chosen.config){const config=element('details');config.appendChild(element('summary',t('Configuration example','Konfigurationsbeispiel')));config.appendChild(element('p',chosen.configNote));kpiHelpCommand(config,chosen.config);compiler.appendChild(config);}
  }else compiler.appendChild(element('p',t('Coverage tools below measure executed tests; they do not supply these missing class metrics.','Die folgenden Coverage-Werkzeuge messen ausgeführte Tests; sie liefern keine fehlenden Klassenmetriken.')));
  const coverage=kpiHelpSection(columns,t('Optional: test coverage','Optional: Testabdeckung'),chosen.coverage.tool+' → '+chosen.coverage.format);
  coverage.appendChild(element('p',chosen.coverage.note));kpiHelpCommand(coverage,chosen.coverage.commands);kpiHelpLinks(coverage,chosen.coverage.links);
  detail.appendChild(element('p',t('Examples assume an already configured project and test suite. Run them from the project root; adapt paths, package versions and test selection. These buttons only copy text; the viewer does not install tools or execute the commands.','Die Beispiele setzen ein eingerichtetes Projekt mit Tests voraus. Im Projektverzeichnis ausführen; Pfade, Paketversionen und Testauswahl anpassen. Die Schaltflächen kopieren nur Text; der Viewer installiert keine Werkzeuge und führt die Befehle nicht aus.'),'muted'));
  const additional=kpiHelpSection(body,t('Additional measurements · independent of source language','Weitere Messwerte · unabhängig von der Quellsprache'));
  for(const [title,description,command,links] of [
    [t('Git history and knowledge concentration','Git-Historie und Wissensverteilung'),t('Requires git on PATH and local repository history. Churn, co-change and bus-factor observations need a sufficiently complete history; a shallow clone can leave values unknown. No external metrics service is needed.','Benötigt git auf PATH und lokale Repository-Historie. Änderungsaufkommen, Co-Change und Bus-Faktor benötigen eine ausreichend vollständige Historie; bei flachen Klonen können Werte unbekannt bleiben. Kein externer Metrikdienst nötig.'),'git --version\narchguard metrics . --extended --format json',[]],
    [t('Duplicates','Duplikate'),t('Import CPD XML from a compatible duplication scanner, for example PMD CPD with a tokenizer for the source language. Its supported languages and version are separate from ArchGuard’s.','CPD-XML eines kompatiblen Duplikatscanners importieren, etwa PMD CPD mit einem Tokenizer für die Quellsprache. Dessen Sprachumfang und Version sind unabhängig von ArchGuard.'),'archguard duplicates . --report cpd.xml',[['PMD CPD','https://pmd.github.io/pmd/pmd_userdocs_cpd.html']]],
    [t('Linter and analyzer findings','Linter- und Analyzer-Befunde'),t('Use a language-appropriate analyzer that exports SARIF 2.1.0. A plain text, HTML or arbitrary JSON report is insufficient. These findings are separate from compiler-based class KPIs.','Einen zur Sprache passenden Analyzer mit SARIF-2.1.0-Export verwenden. Reiner Text, HTML oder beliebiges JSON reicht nicht. Die Befunde sind getrennt von compilerbasierten Klassen-KPIs.'),'archguard quality . --report findings.sarif',[]],
    [t('Test outcomes and duration','Testergebnisse und Dauer'),t('Import JUnit XML from the project’s test runner. A .NET TRX file is not JUnit XML. Exact test-to-source relationships need a separate explicit mapping; a single test report does not measure flakiness.','JUnit-XML des Test-Runners importieren. Eine .NET-TRX-Datei ist kein JUnit-XML. Exakte Test-zu-Quellcode-Beziehungen benötigen eine separate explizite Zuordnung; ein einzelner Testbericht misst keine Testinstabilität.'),'archguard test-evidence . --report junit.xml',[]],
    [t('Mutation testing','Mutationstests'),t('Import mutation-testing-elements JSON (schema 1/2), for example a compatible Stryker report. Native reports from other tools need that schema; an arbitrary mutation JSON is not sufficient.','mutation-testing-elements-JSON (Schema 1/2) importieren, beispielsweise einen kompatiblen Stryker-Bericht. Native Berichte anderer Werkzeuge benötigen dieses Schema; beliebiges Mutations-JSON reicht nicht.'),'archguard mutations . --report mutation-report.json',[['Stryker.NET: reporters','https://stryker-mutator.io/docs/stryker-net/reporters/']]],
    [t('Build/runtime measurements and test stability','Build-/Laufzeitmessungen und Teststabilität'),t('A CI runner or benchmark must produce real observations in ArchGuard’s measurement-runs JSON schema: schema_version 1, runs with run_id, kind, name, revision, environment, producer, timestamp, outcome and optional duration_seconds. Stability needs repeated comparable runs. Ordinary JUnit or coverage reports do not replace this format.','CI-Runner oder Benchmark müssen echte Beobachtungen im ArchGuard-JSON-Schema für Messläufe liefern: schema_version 1, runs mit run_id, kind, name, revision, environment, producer, timestamp, outcome und optional duration_seconds. Stabilität benötigt wiederholte vergleichbare Läufe. Gewöhnliche JUnit- oder Coverage-Berichte ersetzen dieses Format nicht.'),'archguard metrics . --extended --report runs.json --format json',[]],
    [t('Dependency vulnerabilities','Schwachstellen in Abhängigkeiten'),t('Optional: Trivy on PATH with a vulnerability database and supported dependency manifests/lockfiles. This command explicitly runs the scanner; --cached only reads the saved snapshot. Database updates may need network access.','Optional: Trivy auf PATH mit Schwachstellendatenbank und unterstützten Abhängigkeitsmanifesten/Lockfiles. Dieser Befehl startet den Scanner ausdrücklich; --cached liest nur den gespeicherten Snapshot. Datenbankupdates können Netzwerkzugriff benötigen.'),'archguard security .\narchguard security . --cached',[['Trivy: filesystem scanning','https://trivy.dev/docs/latest/target/filesystem/']]],
    [t('Trends and before/after comparison','Verlauf und Vorher-/Nachher-Vergleich'),t('No additional tool; record ArchGuard snapshots after refreshing the compiler analysis and imported reports. Missing or incompatible measurements do not produce reliable deltas.','Kein Zusatzwerkzeug; ArchGuard-Snapshots nach Aktualisierung der Compileranalyse und Importberichte aufzeichnen. Fehlende oder inkompatible Messungen liefern keine belastbaren Differenzen.'),'archguard quality-history . --record --tag before-change',[]]
  ]){
    const item=element('details');item.appendChild(element('summary',title));item.appendChild(element('p',description));kpiHelpCommand(item,command);if(links.length)kpiHelpLinks(item,links);additional.appendChild(item);
  }
  const missing=kpiHelpSection(body,t('Why does a KPI show “—”?','Warum steht bei einer KPI „—“?'));
  const checks=element('ol');
  for(const text of [
    t('Check availability first: PHP has no class adapter; Python and Rust are measured from syntax only. Abstractness A and main-sequence distance D′ currently require complete Java/C# compiler coverage, even though TypeScript has other class KPIs.','Zuerst die Unterstützung prüfen: PHP hat keinen Klassenadapter; Python und Rust werden nur aus der Syntax gemessen. Abstraktheit A und Hauptsequenz-Abstand D′ benötigen derzeit vollständige Java-/C#-Compilerabdeckung, obwohl TypeScript andere Klassen-KPIs besitzt.'),
    t('Run archguard doctor for tool availability. It probes executable versions, not project references or coverage plugins. Inspect archguard calls . --format json for the project’s actual compiler status and diagnostics.','Mit archguard doctor die Werkzeugverfügbarkeit prüfen. Es prüft Programmversionen, nicht Projektverweise oder Coverage-Plugins. archguard calls . --format json zeigt den tatsächlichen Compilerstatus und Diagnosen des Projekts.'),
    t('Match SDK, compiler, target framework, dependencies and source configuration to the project. Restart archguard serve after installing a missing compiler. Read the new analysis, or regenerate a static HTML report.','SDK, Compiler, Target-Framework, Abhängigkeiten und Quellkonfiguration auf das Projekt abstimmen. Nach Installation eines fehlenden Compilers archguard serve neu starten. Die neue Analyse öffnen bzw. einen statischen HTML-Bericht neu erzeugen.'),
    t('Generate and import coverage for the selected project. --source-root can map report paths to source files. An import replaces that project’s saved coverage snapshot; combine reports first when one project needs several test runs.','Coverage für das ausgewählte Projekt erzeugen und importieren. --source-root kann Berichtspfade Quelldateien zuordnen. Ein Import ersetzt den gespeicherten Coverage-Snapshot dieses Projekts; bei mehreren Testläufen eines Projekts Berichte vorher zusammenführen.'),
    t('Missing data is unknown, not zero. Source changes after import make affected measurements stale. Compare the same language, metric definition and measurement scope; AST metrics need not equal values from another tool.','Fehlende Daten sind unbekannt, nicht null. Quelländerungen nach dem Import machen betroffene Messwerte veraltet. Nur gleiche Sprache, Metrikdefinition und Messabdeckung vergleichen; AST-Metriken müssen nicht mit Werten anderer Werkzeuge übereinstimmen.')
  ])checks.appendChild(element('li',text));missing.appendChild(checks);
  kpiHelpCommand(missing,'archguard doctor\narchguard calls . --format json\narchguard metrics . --extended --format json\narchguard coverage . --format json');
  body.scrollTop=scroll;dialog.setAttribute('aria-label',kpiHelpUi.title.textContent);
}
function closeKpiHelp() {kpiHelpUi.dialog?.close();kpiHelpUi.previousFocus?.focus({preventScroll:true});}
function openKpiHelp() {
  if(!kpiHelpUi.language){
    const aliases={cs:'csharp','c#':'csharp',rust:'rs',typescript:'ts',javascript:'js',python:'py'},counts=new Map(),known=new Set(kpiLanguageHelp().map(row=>row.id));
    for(const node of data.nodes){const source=String(node.lang||'').toLowerCase(),lang=aliases[source]||source;if(known.has(lang))counts.set(lang,(counts.get(lang)||0)+1);}
    kpiHelpUi.language=[...counts].sort((a,b)=>b[1]-a[1])[0]?.[0]||'java';
  }
  kpiHelpUi.previousFocus=document.activeElement;hideMetricTooltip();hideContextTooltip();renderKpiHelp();kpiHelpUi.dialog.showModal();kpiHelpUi.close.focus();
}
function syncKpiHelp() {if(kpiHelpUi.action)kpiHelpUi.action.textContent=t('KPI help','KPI-Hilfe');if(kpiHelpUi.dialog?.open)renderKpiHelp();}
function initKpiHelp() {
  const style=element('style',`
    #kpi-help-dialog { width:min(1160px,96vw); max-width:96vw; height:90vh; max-height:90vh; padding:0; overflow:hidden; }
    #kpi-help-dialog[open] { display:flex; flex-direction:column; }
    .kpi-help-header { flex:none; display:flex; align-items:center; gap:10px; padding:18px 24px; border-bottom:1px solid var(--border); }
    .kpi-help-header h2 { flex:1; margin:0; font-size:22px; line-height:1.25; }
    .kpi-help-header button[aria-pressed=true],.kpi-help-table-wrap button[aria-pressed=true] { border-color:var(--accent); color:var(--accent); }
    .kpi-help-body { min-height:0; overflow:auto; padding:0 24px 28px; overscroll-behavior:contain; line-height:1.6; }
    .kpi-help-intro { color:var(--muted); font-size:15px; }
    .kpi-help-cards { display:grid; grid-template-columns:repeat(3,minmax(0,1fr)); gap:12px; }
    .kpi-help-section { margin:22px 0; min-width:0; scroll-margin-top:16px; }
    .kpi-help-section h3 { margin:0 0 10px; font-size:17px; }
    .kpi-help-section p { margin:8px 0 12px; overflow-wrap:anywhere; }
    .kpi-help-cards > section { margin:0; padding:16px; border:1px solid var(--border); border-radius:10px; background:var(--panel-2); }
    .kpi-help-table-wrap { overflow:auto; border:1px solid var(--border); border-radius:10px; }
    .kpi-help-table-wrap table { width:100%; min-width:660px; border-collapse:collapse; font-size:13px; }
    .kpi-help-table-wrap th,.kpi-help-table-wrap td { padding:11px 14px; text-align:left; border-bottom:1px solid var(--border); vertical-align:middle; }
    .kpi-help-table-wrap thead { background:var(--panel-2); }
    .kpi-help-table-wrap .is-selected { background:var(--accent-light,var(--panel-2)); }
    .kpi-help-table-wrap small { display:block; margin-top:4px; color:var(--muted); font-weight:normal; }
    .kpi-help-columns { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:20px; }
    .kpi-help-columns > section { margin:0; border:1px solid var(--border); border-radius:10px; padding:18px; }
    .kpi-help-command { margin:12px 0; padding:12px; border-radius:8px; border:1px solid var(--border); background:var(--panel-2); }
    .kpi-help-command pre { margin:0 0 10px; white-space:pre-wrap; overflow-wrap:anywhere; font-size:12px; line-height:1.6; color:var(--text); }
    .kpi-help-links { display:flex; flex-wrap:wrap; gap:8px 18px; }.kpi-help-links a { color:var(--accent); text-decoration:underline; }
    .kpi-help-body details { padding:12px 0; border-bottom:1px solid var(--border); }.kpi-help-body summary { cursor:pointer; font-weight:600; }
    .kpi-help-body li { margin:10px 0; }.kpi-help-body ol { padding-left:22px; }
    @media(max-width:760px) { .kpi-help-cards,.kpi-help-columns {grid-template-columns:1fr;} .kpi-help-body {padding:0 14px 20px;} .kpi-help-header {padding:14px; flex-wrap:wrap;} .kpi-help-header h2 {flex-basis:100%;font-size:20px;} }
  `);
  const dialog=element('dialog');dialog.setAttribute('id','kpi-help-dialog');dialog.appendChild(style);
  const header=element('div',undefined,'kpi-help-header'),title=element('h2');title.setAttribute('id','kpi-help-title');dialog.setAttribute('aria-labelledby','kpi-help-title');header.appendChild(title);
  const de=button('DE',()=>setViewerLanguage('de'),'btn btn-secondary'),en=button('EN',()=>setViewerLanguage('en'),'btn btn-secondary');de.setAttribute('aria-label','Deutsch');en.setAttribute('aria-label','English');header.appendChild(de);header.appendChild(en);
  const close=button(t('Close','Schließen'),closeKpiHelp,'btn btn-secondary');close.setAttribute('data-kpi-help-close','true');header.appendChild(close);dialog.appendChild(header);
  const body=element('div',undefined,'kpi-help-body');dialog.appendChild(body);document.getElementById('main').appendChild(dialog);
  const action=button(t('KPI help','KPI-Hilfe'),openKpiHelp,'btn btn-quiet');action.setAttribute('id','kpi-help-open');action.setAttribute('aria-haspopup','dialog');document.getElementById('topbar').appendChild(action);
  Object.assign(kpiHelpUi,{action,dialog,body,title,close,de,en});
  dialog.addEventListener('keydown',event=>{if(event.key==='Escape')event.stopPropagation();});
  dialog.addEventListener('close',()=>kpiHelpUi.previousFocus?.focus({preventScroll:true}));
}
