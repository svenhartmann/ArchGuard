'use strict';
// ─── archguard viewer v2 (T39) ────────────────────────────────────────────────
// Reads the JSON payload from <script id="archguard-data"> and renders an
// interactive, drill-downable, cycle-aware force graph. Sections below
// mirror server-side Rust one-to-one (each comment names the function):
//   Cut::at_depth / Cut::expand / Cut::collapse / aggregate /
//   nearest_visible / Graph::children_of / Graph::ancestors_of
//     → archguard-core/src/view.rs, archguard-core/src/lib.rs
//   apply_ops / cut_edge / move_node (+ subtree, rename_subtree)
//     → archguard-analysis/src/simulate.rs
//   find_sccs (Tarjan, ordering rules) → archguard-analysis/src/scc.rs
//   lakos_metrics (condense + reachable + CCD/ACD)
//     → archguard-analysis/src/lakos.rs
//   ModuleMetrics instability = fan_out/(fan_in+fan_out)
//     → archguard-core/src/lib.rs
// F17 edge geometry: directed pairs stay SEPARATE links; line width is the
// bounded log scale widthFor() of the F12 total (never the sample), cycle
// status stays in the colour. The custom arrow uses the same quadratic
// curve as pinned force-graph 1.52 and clips to the painted node bounds.
// Client SCC ids ("scc-N" from the client re-run) are positional on the
// CURRENT view and MAY DIFFER from the server finding ids; the cycles
// panel lists server findings until a simulation recomputes the cycles.
// The browser NEVER edits code: repair mode only records SimOps
// (frozen T37 contract, archguard-analysis simulate.rs) for export.

// Large HTML reports pool repeated compiler symbol strings. Decode once into
// the same source model used by legacy reports, simulation and exports.
function decodeCallPayload(model) {
  if(!model?.compact)return model;
  const compact=model.compact;
  if(compact.encoding!=='string-pool-v1'||!Array.isArray(compact.strings)||!compact.strings.every(value=>typeof value==='string')||!compact.fields||typeof compact.fields!=='object')throw new Error('Unsupported compact call encoding');
  const string=id=>{
    if(!Number.isSafeInteger(id)||id<0||id>=compact.strings.length)throw new Error('Invalid call string-pool reference');
    return compact.strings[id];
  };
  for(const table of ['classes','methods','calls']){
    const rows=model[table],fields=compact.fields[table];
    if(!Array.isArray(rows)||!Array.isArray(fields)||!fields.every(field=>typeof field==='string'))throw new Error('Invalid compact call table');
    const pooled=new Set(fields),nullable=id=>id===null?null:string(id);
    const fastCalls=table==='calls'&&fields.length===9&&fields.every(field=>['code','dispatch','file','from_class','from_method','resolution','target_label','to_class','to_method'].includes(field));
    model[table]=rows.map(row=>{
      if(!row||typeof row!=='object'||Array.isArray(row))throw new Error('Invalid compact call row');
      const keys=Object.keys(row);
      // Construct string-valued rows directly: converting every numeric field
      // in place repeatedly changes V8 object representations on large reports.
      if(fastCalls&&keys.length===11&&Object.hasOwn(row,'column')&&Object.hasOwn(row,'line'))return {
        code:string(row.code),column:row.column,dispatch:string(row.dispatch),file:string(row.file),
        from_class:string(row.from_class),from_method:nullable(row.from_method),line:row.line,
        resolution:string(row.resolution),target_label:string(row.target_label),to_class:nullable(row.to_class),to_method:nullable(row.to_method)
      };
      const output={};
      for(const field of keys){
        const cell=row[field],value=pooled.has(field)&&cell!==null?(Array.isArray(cell)?cell.map(string):string(cell)):cell;
        if(field==='__proto__')Object.defineProperty(output,field,{value,writable:true,enumerable:true,configurable:true});else output[field]=value;
      }
      return output;
    });
  }
  delete model.compact;
  return model;
}

let data;
try {
  const source=document.getElementById('archguard-data');
  data = JSON.parse(source.textContent);
  if (!Array.isArray(data.nodes) || !Array.isArray(data.edges) || !data.view) throw new Error('Unsupported report data');
  source.textContent='';
} catch (error) {
  document.getElementById('graph-state').textContent = t('This report could not be read. Generate a new HTML report with archguard graph --format html.','Der Bericht konnte nicht gelesen werden. Erzeuge einen neuen HTML-Bericht mit archguard graph --format html.');
  throw error;
}
const ui = { selection: null, cycle: null, workspace: 'graph', diagram: 'overview', explorer: 'modules', simulation: false,
  matrixPage: 0, compact: false, searchLimit: 100, pending: null, previousFocus: null, showFiles:false, edgeMode:'all', edgeModeAutomatic:true, cyclesOnly:false, highlightPriorities:true,reviewLimit:20,reviewOrphans:false };
const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
let viewRevision = 0, matrixRevision = -1, graphSignature = '', currentAcd = 0, baselineStats = null;
function feedback(message) { document.getElementById('feedback').textContent = message; }
function element(tag, text, className) {
  const node = document.createElement(tag);
  if (text !== undefined) node.textContent = text;
  if (className) node.className = className;
  return node;
}
const METRIC_HELP = {
  cbo: 'CBO — Coupling Between Objects. Distinct resolved target types used by declaration types and calls, excluding self and inheritance. Higher values mean broader coupling to inspect. Missing compiler evidence leaves the value unknown (—); observed counts are lower bounds.',
  rfc: 'RFC — Response For a Class. Distinct own non-constructor methods and their direct call targets, counting each member once. A larger response set increases the behavior to understand and test. This uses static resolution, not all possible runtime dispatch targets. Incomplete evidence remains —.',
  dit: 'DIT — Depth of Inheritance Tree. Superclass edges up to the compiler-known root, including known external bases; interfaces are excluded. Deeper inheritance means more inherited behavior to trace, not an automatic design error. Unknown hierarchy remains —.',
  noc: 'NOC — Number of Children. Direct subclasses inside the analyzed source scope. External subclasses are not counted. High values can mean intended reuse or a broad change impact; this is informational, not a good/bad score. Incomplete hierarchy remains —.',
  wmc: 'WMC — Weighted Methods per Class. Sum of source cyclomatic complexity of declared non-constructor methods. Static methods count; local functions and constructors are excluded. High values suggest a large or complex responsibility. Missing method measurements leave the sum unknown (—).',
  lcom4: 'LCOM4 — Lack of Cohesion in Methods, variant 4. Connected groups of instance methods sharing a non-static field/property or calling each other. One group is connected; multiple groups invite a responsibility review. Constructors, static members and local functions are excluded. Incomplete compiler evidence remains —.',
  cognitive: 'Cognitive — versioned source complexity. Counts control flow, nesting, else branches and logical sequences; nested function bodies and recursion are excluded. Higher values indicate more control flow to understand. Compare the same source language and measurement definition; this is not a runtime measurement.',
  maintainability: 'MI — Maintainability Index, 0–100. Higher values are favorable in this source-based formula: (171 − 5.2 ln(Halstead volume) − 0.23 × cyclomatic complexity − 16.2 ln(source lines)) × 100 / 171, clamped to 0–100. No comment term. Missing inputs remain —; the index is not proof of maintainability.',
  halsteadVolume: 'Halstead volume V = (operators + operands) × log2(distinct operators + distinct operands). ArchGuard counts AST operators and identifier/literal operands. Higher values indicate more source vocabulary and occurrences; compare the same language and definition.',
  halsteadDifficulty: 'Halstead difficulty D = distinct operators / 2 × operand occurrences / distinct operands. A source vocabulary heuristic. Without distinct operands it remains unknown (—); it does not measure how difficult a person finds the code.',
  halsteadEffort: 'Halstead effort E = difficulty × volume. A derived source metric using ArchGuard AST counting rules. Larger values reflect the formula, not measured hours, cost or developer productivity. Missing inputs remain —.',
  scope: 'Packages and files in the current dependency-analysis scope. Graph filters may hide elements without changing this count. Expanding a package changes the scope. The separate Calls view does not change the graph metrics.',
  dependencies: 'Directed connections between nodes in the current dependency-analysis scope. A → B and B → A count separately. Multiple import references between the same pair form one connection. These are import connections, not method calls.',
  cycles: 'Groups of nodes that depend on each other directly or indirectly. Counts cyclic groups, not individual cycle paths. Includes hidden nodes in the analysis scope.',
  acd: 'ACD — Average Component Dependency. In the dependency graph, the average number of distinct nodes reachable through direct or indirect outgoing dependencies, including the starting node itself. Each reachable node counts once. Includes hidden nodes in the analysis scope; this is not a percentage or a quality score.',
  incoming: 'Distinct nodes with recorded direct dependencies on this node in the current analysis scope. Counts directed connections, not individual source evidence records.',
  outgoing: 'Distinct nodes this node has recorded direct dependencies on in the current analysis scope. Counts directed connections, not individual source evidence records.',
  instability: 'Outgoing connections divided by incoming plus outgoing connections. 0 means no outgoing connections; 1 means only outgoing connections. Defined as 0 when there are no connections. Describes dependency balance, not runtime reliability.',
  simulation: 'Compares the original graph with the simulated graph at the same current scope and expansion level. Changes affect the model; source files remain unchanged.',
  report: 'Error findings in the original report. Simulation and display filters do not rewrite this report status. No historical baseline comparison was supplied.',
  ccd: 'CCD — Cumulative Component Dependency. Sum of the distinct reachable nodes for every node in the current dependency scope, including each starting node itself. Structural coupling, not measured compilation time.',
  nccd: 'NCCD — Normalized Cumulative Component Dependency. CCD divided by (N + 1) × log2(N + 1) − N, the balanced-tree reference for N scope nodes. Dimensionless; a comparison of structure, not a runtime or build-time measurement.',
  largestScc: 'Largest cyclic strongly connected component in the current scope. Each member can reach every other member. Size highlights structural entanglement; dependency cycles alone do not prove that a compiler cannot build files in parallel.',
  cyclicNodes: 'Number of current-scope nodes that belong to any cyclic strongly connected component. Aggregated package cycles and file-level cycles can differ.',
  priority: 'Review priority is a transparent heuristic from 0 to 100, not a probability of defects: cycle min(40, 30 + 2 × (SCC size − 1)); coupling min(20, 2 × (fan-in + fan-out)); impact min(20, transitive dependents); available Git change touches × min(5, fan-in + fan-out), capped at 20. High ≥ 40, medium ≥ 20. Missing Git evidence contributes no points and is reported as unavailable.',
  orphans: 'Nodes with no incoming dependencies in this scope and at least one source file, excluding configured entry points. These are review candidates, not proof of dead code: dynamic imports, frameworks, external users and test discovery may consume them.',
  sourceLines: 'Physical lines in the source files represented by this scope. The value is unavailable when any required file could not be mapped/read. Simulation changes graph structure, not source text.',
  sourceChanges: 'Sum of Git commit touches across the source files in this scope for the reported time window. A commit touching two files counts twice. This is not a count of distinct package commits; shallow/truncated history gives a lower bound.',
  coverage: 'Imported test-report line coverage of files in the current graph scope: covered executable lines / reported executable lines, counting only files whose sources are unchanged since import. Files with unknown, aggregate-only or changed-since-import measurements are excluded, never counted as 0%; without any countable file the tile shows —. The tile follows graph filters and simulated renames like other scope KPIs; the coverage review keeps the original snapshot. Report/source revision at import is unverified.',
  abstractness: 'Abstractness A: compiler-declared abstract classes and interfaces divided by all Java/C# type declarations in this report scope. Available only with complete compiler coverage for the scope’s source languages. This is separate from instability I.',
  distance: 'Normalized Main Sequence Distance D′ = |A + I − 1|, from 0 to 1. A is compiler-backed Java/C# abstractness; I is outgoing / (incoming + outgoing) dependency boundary coupling. Larger values invite architectural review; the metric does not prove a design flaw, DIP violation or LSP violation.',
  closure: 'Project source files transitively reachable through dependencies from a configured entry point, counting each file once. Longest path counts components of the SCC-condensed graph. External and runtime-loaded code are not represented; these are structural counts, not measured startup or bundle size.',
};
const METRIC_HELP_DE={
  cbo:'CBO — Coupling Between Objects (Klassenkopplung). Verschiedene aufgelöste Zieltypen aus Deklarationstypen und Aufrufen; eigener Typ und Vererbung sind ausgenommen. Höhere Werte bedeuten breitere Kopplung und geben Anlass zur Prüfung. Fehlende Compiler-Belege lassen den Wert unbekannt (—); beobachtete Zahlen sind Untergrenzen.',
  rfc:'RFC — Response For a Class (Antwortumfang). Menge eigener Methoden ohne Konstruktoren und ihrer direkten Aufrufziele; jedes Mitglied zählt einmal. Ein größerer Umfang bedeutet mehr Verhalten zum Verstehen und Testen. Statische Auflösung, nicht alle möglichen Laufzeitziele. Unvollständige Belege bleiben —.',
  dit:'DIT — Depth of Inheritance Tree (Vererbungstiefe). Basisklassenkanten bis zur compilerbekannten Wurzel, einschließlich bekannter externer Basen; Interfaces zählen nicht. Tiefere Vererbung bedeutet mehr nachzuverfolgendes Verhalten, aber keinen automatischen Entwurfsfehler. Unbekannte Hierarchie bleibt —.',
  noc:'NOC — Number of Children (direkte Unterklassen). Zählt Unterklassen im analysierten Quellumfang; externe Unterklassen fehlen. Hohe Werte können gewollte Wiederverwendung oder eine breite Änderungswirkung zeigen. Eine Information, keine Gut-/Schlecht-Note. Unvollständige Hierarchie bleibt —.',
  wmc:'WMC — Weighted Methods per Class (gewichtete Methoden). Summe der zyklomatischen Quellcodekomplexität deklarierter Methoden. Statische Methoden zählen; Konstruktoren und lokale Funktionen sind ausgenommen. Hohe Werte geben Anlass, Umfang und Verantwortung zu prüfen. Fehlende Methodenmessungen lassen die Summe unbekannt (—).',
  lcom4:'LCOM4 — Lack of Cohesion in Methods, Variante 4 (Zusammenhalt). Gruppen von Instanzmethoden, die dasselbe nichtstatische Feld bzw. dieselbe Property nutzen oder einander aufrufen. Eine Gruppe ist zusammenhängend; mehrere Gruppen geben Anlass zur Verantwortungsprüfung. Konstruktoren, statische Mitglieder und lokale Funktionen zählen nicht. Unvollständige Compiler-Belege bleiben —.',
  cognitive:'Cognitive — versionierte Quellcodekomplexität. Zählt Kontrollfluss, Verschachtelung, else-Zweige und logische Folgen; verschachtelte Funktionskörper und Rekursion sind ausgenommen. Höhere Werte bedeuten mehr zu verstehenden Kontrollfluss. Nur gleiche Quellsprache und Messdefinition vergleichen; keine Laufzeitmessung.',
  maintainability:'MI — Maintainability Index (Wartbarkeitsindex), 0–100. Höher ist in dieser Quellcodeformel günstiger: (171 − 5,2 ln(Halstead-Volumen) − 0,23 × zyklomatische Komplexität − 16,2 ln(Quellzeilen)) × 100 / 171, begrenzt auf 0–100. Ohne Kommentaranteil. Fehlende Eingaben bleiben —; kein Nachweis guter Wartbarkeit.',
  halsteadVolume:'Halstead-Volumen V = (Operatoren + Operanden) × log2(verschiedene Operatoren + verschiedene Operanden). ArchGuard zählt AST-Operatoren sowie Bezeichner und Literale. Höhere Werte bedeuten mehr Quellvokabular und Vorkommen; nur gleiche Sprache und Definition vergleichen.',
  halsteadDifficulty:'Halstead-Schwierigkeit D = verschiedene Operatoren / 2 × Operandenvorkommen / verschiedene Operanden. Ein Hinweis aus dem Quellvokabular. Ohne verschiedene Operanden bleibt der Wert unbekannt (—); keine Messung der von Menschen empfundenen Schwierigkeit.',
  halsteadEffort:'Halstead-Aufwand E = Schwierigkeit × Volumen. Abgeleitete Quellkennzahl mit den ArchGuard-AST-Zählregeln. Größere Werte beschreiben die Formel, keine gemessenen Arbeitsstunden, Kosten oder Entwicklerproduktivität. Fehlende Eingaben bleiben —.',
  scope:'Pakete und Dateien im aktuellen Umfang der Abhängigkeitsanalyse. Graphfilter können Elemente ausblenden, ohne diese Zahl zu ändern. Das Öffnen eines Pakets ändert den Analyseumfang. Die separate Aufrufansicht ändert die Graphkennzahlen nicht.',
  dependencies:'Gerichtete Verbindungen zwischen Knoten im aktuellen Umfang der Abhängigkeitsanalyse. A → B und B → A zählen getrennt. Mehrere Importreferenzen zwischen demselben Paar ergeben eine Verbindung. Gemeint sind Imports, keine Methodenaufrufe.',
  cycles:'Gruppen von Knoten, die direkt oder indirekt voneinander abhängen. Gezählt werden zyklische Gruppen, nicht einzelne Zykluspfade. Ausgeblendete Knoten im Analyseumfang zählen mit.',
  acd:'ACD — Average Component Dependency (durchschnittliche Komponentenabhängigkeit). Im Abhängigkeitsgraphen die durchschnittliche Anzahl verschiedener Knoten, die über direkte oder indirekte ausgehende Abhängigkeiten erreichbar sind, einschließlich des Ausgangsknotens selbst. Jeder erreichbare Knoten zählt einmal. Ausgeblendete Knoten im Analyseumfang zählen mit; dies ist weder ein Prozentwert noch eine Qualitätsbewertung.',
  incoming:'Verschiedene Knoten mit erfassten direkten Abhängigkeiten zu diesem Knoten im aktuellen Analyseumfang. Gezählt werden gerichtete Verbindungen, nicht einzelne Quellbelege.',
  outgoing:'Verschiedene Knoten, zu denen dieser Knoten im aktuellen Analyseumfang erfasste direkte Abhängigkeiten hat. Gezählt werden gerichtete Verbindungen, nicht einzelne Quellbelege.',
  instability:'Ausgehende Verbindungen geteilt durch die Summe eingehender und ausgehender Verbindungen. 0 bedeutet keine ausgehenden, 1 ausschließlich ausgehende Verbindungen. Ohne Verbindungen ist der Wert als 0 definiert. Beschreibt das Verhältnis der Abhängigkeiten, nicht die Zuverlässigkeit zur Laufzeit.',
  simulation:'Vergleicht den ursprünglichen und den simulierten Graphen im selben aktuellen Analyseumfang und auf derselben Aufklappebene. Die Änderungen betreffen das Modell; Quelldateien bleiben unverändert.',
  report:'Fehlerbefunde im ursprünglichen Bericht. Simulation und Anzeigefilter ändern diesen Berichtstatus nicht. Es wurde kein historischer Baseline-Vergleich mitgeliefert.',
  ccd:'CCD — Cumulative Component Dependency (kumulierte Komponentenabhängigkeit). Summe der verschiedenen erreichbaren Knoten für jeden Knoten im aktuellen Abhängigkeitsumfang, jeweils einschließlich des Ausgangsknotens. Strukturelle Kopplung, keine gemessene Kompilierungszeit.',
  nccd:'NCCD — Normalized Cumulative Component Dependency. CCD geteilt durch (N + 1) × log2(N + 1) − N, die Referenz eines ausgeglichenen Baums mit N Knoten. Dimensionsloser Strukturvergleich, keine Messung von Laufzeit oder Build-Dauer.',
  largestScc:'Größte zyklische, stark zusammenhängende Komponente im aktuellen Umfang. Jedes Mitglied kann jedes andere erreichen. Die Größe zeigt strukturelle Verflechtung; Abhängigkeitszyklen allein beweisen nicht, dass ein Compiler Dateien nicht parallel bauen kann.',
  cyclicNodes:'Anzahl der Knoten im aktuellen Umfang, die zu einer zyklischen, stark zusammenhängenden Komponente gehören. Aggregierte Paketzyklen und Zyklen auf Dateiebene können sich unterscheiden.',
  priority:'Die Prüfpriorität ist eine nachvollziehbare Heuristik von 0 bis 100, keine Fehlerwahrscheinlichkeit: Zyklus min(40, 30 + 2 × (SCC-Größe − 1)); Kopplung min(20, 2 × (Fan-in + Fan-out)); Auswirkung min(20, transitive abhängige Knoten); verfügbare Git-Dateiänderungen × min(5, Fan-in + Fan-out), begrenzt auf 20. Hoch ≥ 40, mittel ≥ 20. Fehlende Git-Belege geben keine Punkte und werden als nicht verfügbar ausgewiesen.',
  orphans:'Knoten ohne eingehende Abhängigkeiten in diesem Umfang und mit mindestens einer Quelldatei; konfigurierte Einstiegspunkte sind ausgenommen. Dies sind Prüfkandidaten, kein Beweis für toten Code: dynamische Imports, Frameworks, externe Nutzer und Testerkennung können sie verwenden.',
  sourceLines:'Physische Zeilen in den Quelldateien dieses Umfangs. Der Wert ist nicht verfügbar, wenn eine benötigte Datei nicht zugeordnet oder gelesen werden konnte. Simulationen ändern die Graphstruktur, nicht den Quelltext.',
  sourceChanges:'Summe der durch Git-Commits berührten Quelldateien in diesem Umfang und Zeitfenster. Ein Commit mit zwei Dateien zählt zweimal. Dies ist keine Anzahl verschiedener Paket-Commits; flache oder abgeschnittene Historie liefert eine Untergrenze.',
  coverage:'Importierte Zeilen-Testabdeckung der Dateien im aktuellen Graph-Umfang: gedeckte ausführbare Zeilen geteilt durch berichtete ausführbare Zeilen; gezählt werden nur Dateien, deren Quellen seit dem Import unverändert sind. Dateien mit unbekannten, nur aggregierten oder seit dem Import geänderten Messwerten sind ausgeschlossen und zählen niemals als 0 %; ohne zählbare Datei zeigt die Kachel —. Sie folgt wie die anderen Umfangs-Kennzahlen Graphfiltern und simulierten Umbenennungen; der Prüfindex bleibt beim ursprünglichen Snapshot. Die Quellcode-Revision des Berichts ist beim Import nicht verifiziert.',
  abstractness:'Abstraktheit A: vom Compiler deklarierte abstrakte Klassen und Interfaces geteilt durch alle Java/C#-Typdeklarationen dieses Berichtsumfangs. Nur bei vollständiger Compiler-Abdeckung der Quellsprachen im Ausschnitt verfügbar. Unabhängig von der Instabilität I.',
  distance:'Normalisierter Abstand von der Hauptsequenz D′ = |A + I − 1|, von 0 bis 1. A ist die compilerbasierte Java/C#-Abstraktheit; I ist die Abhängigkeitskopplung ausgehend / (eingehend + ausgehend). Größere Werte geben Anlass zur Architekturprüfung; sie beweisen keinen Entwurfsfehler und keine DIP- oder LSP-Verletzung.',
  closure:'Projektquelldateien, die von einem konfigurierten Einstiegspunkt transitiv über Abhängigkeiten erreichbar sind; jede Datei zählt einmal. Der längste Pfad zählt Komponenten im SCC-kondensierten Graphen. Externer und dynamisch geladener Code fehlen; dies sind strukturelle Zählwerte, keine gemessene Startzeit oder Bundle-Größe.'
};
function graphEvidenceUnit() { return data.semantic?t('dependency evidence records','Abhängigkeitsbelege'):t('import references','Importreferenzen'); }
function graphCoverageDescription() {
  if(!data.semantic)return '';
  return t('Dependency graph: imports + resolved calls + supertypes + available declaration types. Compiler coverage: {status} (original source snapshot). Missing references, unsupported syntax and runtime wiring remain unknown.','Abhängigkeitsgraph: Imports + aufgelöste Aufrufe + Obertypen + verfügbare Deklarationstypen. Compiler-Abdeckung: {status} (ursprünglicher Quellcode-Snapshot). Fehlende Referenzen, nicht unterstützte Syntax und Laufzeitverdrahtung bleiben unbekannt.',{status:callCoverageLabel(data.semantic.status)});
}
function graphCycleCoverageWarning() {
  return data.semantic&&['partial','unavailable'].includes(data.semantic.status)?t('Zero detected cycles does not establish a cycle-free architecture: compiler-backed dependencies are incomplete.','Keine erkannten Zyklen sind kein Nachweis einer zyklusfreien Architektur: Compilerbasierte Abhängigkeiten sind unvollständig.'):'';
}
function connectionCountNotice() {
  return data.semantic&&['partial','unavailable'].includes(data.semantic.status)?t('* In/Out: recorded connections · analysis incomplete. 0 means none recorded; dependencies may be missing.','* Ein/Aus: erfasste Beziehungen · Analyse unvollständig. 0 bedeutet keine erfassten Beziehungen; Abhängigkeiten können fehlen.'):'';
}
function connectionCountLabel(value) { return String(value)+(connectionCountNotice()?'*':''); }
function metricHelp(key,includeCoverage=true) {
  if(key==='methodComplexity')return methodComplexityDescription();
  if(data.semantic&&key==='dependencies')return t('Directed file or package connections in the current dependency-analysis scope. A → B and B → A count separately. Imports, resolved project calls and uniquely resolved supertypes provide source evidence; several records for the same pair form one connection. External and unresolved call targets are excluded.','Gerichtete Datei- oder Paketverbindungen im aktuellen Analyseumfang. A → B und B → A zählen getrennt. Imports, aufgelöste Projektaufrufe und eindeutig aufgelöste Obertypen liefern Quellbelege; mehrere Belege desselben Paares ergeben eine Verbindung. Externe und unaufgelöste Aufrufziele sind ausgeschlossen.');
  const text=t(METRIC_HELP[key],METRIC_HELP_DE[key]);
  if(includeCoverage&&(key==='incoming'||key==='outgoing')&&connectionCountNotice())return text+' '+connectionCountNotice();
  return key==='cycles'&&data.semantic?text+' '+graphCoverageDescription():text;
}
const metricHelpBound=new WeakSet(),actionHelpProviders=new WeakMap(),actionHelpButtons=new WeakSet();
let metricTooltipTarget=null,metricTooltipTimer=null,metricAnchorHovered=false,metricTooltipHovered=false,metricTooltipPinned=false;
function hideMetricTooltip(container) {
  if(container&&metricTooltipTarget&&!container.contains(metricTooltipTarget))return;
  clearTimeout(metricTooltipTimer);metricTooltipTimer=null;
  if(metricTooltipTarget){
    const ids=(metricTooltipTarget.getAttribute('aria-describedby')||'').split(/\s+/).filter(id=>id&&id!=='metric-tooltip');
    if(ids.length)metricTooltipTarget.setAttribute('aria-describedby',ids.join(' '));else metricTooltipTarget.removeAttribute('aria-describedby');
  }
  metricTooltipTarget=null;metricAnchorHovered=false;metricTooltipHovered=false;metricTooltipPinned=false;
  document.getElementById('metric-tooltip').hidden=true;
}
function showMetricTooltip(node) {
  if(!node.isConnected)return;
  const describe=actionHelpProviders.get(node);if(describe){node.setAttribute('data-metric-description',describe());node.removeAttribute('title');}
  if(metricTooltipTarget!==node)hideMetricTooltip();
  clearTimeout(metricTooltipTimer);metricTooltipTimer=null;metricTooltipTarget=node;
  const tip=document.getElementById('metric-tooltip');
  tip.textContent=node.getAttribute('data-metric-description');tip.hidden=false;
  const ids=new Set((node.getAttribute('aria-describedby')||'').split(/\s+/).filter(Boolean));ids.add('metric-tooltip');node.setAttribute('aria-describedby',[...ids].join(' '));
  const anchor=node.getBoundingClientRect(),box=tip.getBoundingClientRect(),width=window.innerWidth,height=window.innerHeight||600;
  const boxWidth=Math.min(box.width,width-16),boxHeight=Math.min(box.height,height-16),top=anchor.top||0,bottom=anchor.bottom??top+anchor.height;
  tip.style.left=Math.max(8,Math.min(anchor.left||0,width-boxWidth-8))+'px';
  tip.style.top=Math.max(8,Math.min(bottom+8+boxHeight<=height-8?bottom+8:top-boxHeight-8,height-boxHeight-8))+'px';
}
function deferMetricTooltipHide() {
  clearTimeout(metricTooltipTimer);
  metricTooltipTimer=setTimeout(()=>{
    if(!metricAnchorHovered&&!metricTooltipHovered&&!metricTooltipPinned&&document.activeElement!==metricTooltipTarget)hideMetricTooltip();
  },120);
}
function explainMetric(node, key, detail='') {
  node.setAttribute('data-metric-help','true');
  return explainContext(node,metricHelp(key)+(detail?' '+detail:''));
}
function explainAction(node,describe){actionHelpProviders.set(node,describe);node.setAttribute('data-action-help','true');return explainContext(node,describe());}
function explainContext(node,description){
  node.removeAttribute('title');node.setAttribute('tabindex','0');
  if(!actionHelpProviders.has(node))node.setAttribute('aria-label',node.textContent);node.setAttribute('data-metric-description',description);
  if(!metricHelpBound.has(node)){
    metricHelpBound.add(node);
    node.addEventListener('pointerenter',()=>{showMetricTooltip(node);metricAnchorHovered=true;});
    node.addEventListener('pointerleave',()=>{if(metricTooltipTarget===node){metricAnchorHovered=false;deferMetricTooltipHide();}});
    node.addEventListener('focus',()=>showMetricTooltip(node));
    node.addEventListener('blur',()=>{if(metricTooltipTarget===node){metricTooltipPinned=false;deferMetricTooltipHide();}});
    // Actions keep their native keys and clicks; plain metrics also support touch.
    if(!actionHelpProviders.has(node)&&node.tagName!=='BUTTON'&&node.tagName!=='A'){
      node.addEventListener('click',()=>{showMetricTooltip(node);metricTooltipPinned=true;});
      node.addEventListener('keydown',event=>{if(event.key==='Enter'||event.key===' '){event.preventDefault();showMetricTooltip(node);}});
    }
  }
  if(metricTooltipTarget===node)showMetricTooltip(node);
  return node;
}
function initMetricTooltips() {
  const tip=document.getElementById('metric-tooltip');tip.hidden=true;
  tip.addEventListener('pointerenter',()=>{metricTooltipHovered=true;clearTimeout(metricTooltipTimer);});
  tip.addEventListener('pointerleave',()=>{metricTooltipHovered=false;deferMetricTooltipHide();});
  document.addEventListener('pointerdown',event=>{if(metricTooltipTarget&&!metricTooltipTarget.contains(event.target)&&!tip.contains(event.target))hideMetricTooltip();},true);
  document.addEventListener('scroll',event=>{
    if(tip.contains(event.target))return;
    const target=metricTooltipTarget;
    if(target?.isConnected&&document.activeElement===target){
      const r=target.getBoundingClientRect(),clip=event.target.getBoundingClientRect?.()||{left:0,top:0,width:window.innerWidth,height:window.innerHeight||600};
      const left=Math.max(0,clip.left||0),top=Math.max(0,clip.top||0),right=Math.min(window.innerWidth,(clip.left||0)+clip.width),bottom=Math.min(window.innerHeight||600,(clip.top||0)+clip.height);
      if((r.left||0)+r.width>left&&(r.top||0)+r.height>top&&(r.left||0)<right&&(r.top||0)<bottom){showMetricTooltip(target);return;}
    }
    hideMetricTooltip();
  },true);
  window.addEventListener('blur',()=>hideMetricTooltip());
}
function bindActionHelp(action,help,describe){
  explainAction(action,describe);explainAction(help,describe);
  help.setAttribute('aria-label',t('Help: {action}','Hilfe: {action}',{action:action.getAttribute('aria-label')||action.textContent}));
  help.setAttribute('data-action-help-button','true');
  if(!actionHelpButtons.has(help)){actionHelpButtons.add(help);help.addEventListener('click',()=>{showMetricTooltip(help);metricTooltipPinned=true;});}
}
function actionWithHelp(action,describe){
  const row=element('span',undefined,'action-with-help'),help=button('?',()=>{},'context-help-trigger');
  bindActionHelp(action,help,describe);row.appendChild(action);row.appendChild(help);return row;
}
function contextHelp(label,description){
  const help=button('?',()=>{showMetricTooltip(help);metricTooltipPinned=true;},'context-help-trigger');
  help.setAttribute('aria-label',label);return explainAction(help,()=>description);
}
function refreshActionHelp(){
  const descriptions={
    'simulation-mode':()=>t('Try changes in the graph. Source files stay unchanged. Clear changes to return to the original graph.','Änderungen am Graphen ausprobieren. Quelldateien bleiben unverändert. Änderungen verwerfen führt zum ursprünglichen Graphen zurück.'),
    'repair-cycle':()=>t('Select a cycle, then compare proposed dependency cuts and their simulated impact before applying a plan.','Zyklus auswählen, vorgeschlagene Trennungen und ihre simulierten Auswirkungen vergleichen, dann einen Plan übernehmen.'),
    'scenario-open':()=>t('Save and compare graph simulations in this browser, or exchange them as JSON. Scenarios do not edit source files.','Graphsimulationen in diesem Browser speichern und vergleichen oder als JSON austauschen. Szenarien bearbeiten keine Quelldateien.'),
    'repository-open':()=>t('Open imports, calls and architecture for {project}. Nested projects stay outside this source scope.','Imports, Aufrufe und Architektur für {project} öffnen. Verschachtelte Projekte bleiben außerhalb dieses Quellumfangs.',{project:repositorySelection||t('the selected project','das ausgewählte Projekt')}),
    'workspace-analyze':()=>t('Analyze declared project dependencies together, including source and compiler evidence. Large workspaces can take longer.','Deklarierte Projektabhängigkeiten gemeinsam mit Quell- und Compilerbelegen analysieren. Große Projektverbünde können länger dauern.'),
    'services-kind':()=>document.getElementById('services-kind').value==='startup'?t('Startup order from depends_on. These arrows do not establish API calls. Cycles use this relation type only.','Startreihenfolge aus depends_on. Diese Pfeile belegen keine API-Aufrufe. Zyklen werden nur für diese Beziehungsart berechnet.'):t('Configured HTTP targets, not recorded runtime traffic. Cycles use these configured connections only.','Konfigurierte HTTP-Ziele, keine gemessenen Laufzeitaufrufe. Zyklen verwenden nur diese konfigurierten Verbindungen.'),
    'engineering-target':()=>engineeringUi.section==='tests'?t('Select changed code to find mapped tests and static dependents. Unmapped tests remain visible; other tests are not proven safe to skip.','Geänderten Code auswählen, um zugeordnete Tests und statische Abhängigkeiten zu finden. Tests ohne Zuordnung bleiben sichtbar; andere Tests sind nicht als entbehrlich nachgewiesen.'):t('Limit the displayed measurements to a package or file. Use the search to find a target; the recorded measurements stay unchanged.','Messwerte auf ein Paket oder eine Datei eingrenzen. Das Ziel lässt sich über die Suche finden; aufgezeichnete Messwerte bleiben unverändert.')
  };
  for(const [id,describe]of Object.entries(descriptions))bindActionHelp(document.getElementById(id),document.getElementById(id+'-help'),describe);
}
function button(text, callback, className = 'btn btn-secondary') {
  const node = element('button', text, className); node.type = 'button';
  node.addEventListener('click', callback); return node;
}
function selectedEdge() {
  const sel = ui.selection;
  return sel && sel.kind === 'edge' ? view.edges.find(e => nodeById(view,e.from).name === sel.from && nodeById(view,e.to).name === sel.to) : null;
}
function focusDefaultEdges() {
  if(ui.edgeModeAutomatic&&ui.edgeMode==='all')ui.edgeMode='both';
  ui.edgeModeAutomatic=false;
}
function selectNode(name, focus = false) {
  const restoreFocus=selectionFocusRestorer();
  treeRevealName=name;
  focusDefaultEdges();
  ui.selection = {kind:'node', name};
  const source=nodeByName(work,name);
  if(source)for(const id of ancestorsOf(work,source.id))explorerOpen.add(id);
  reconcileSelection();packageContext=expandedPackageContext(packageExpansions);exitCycleForSelection();pinSelectedGraphFiles();renderGraph(false); renderSidebar(); renderExplorer(); refreshSelection(false);restoreFocus();
  if(source?.kind==='file'&&ui.workspace==='graph'&&!inspectorIsOpen())openInspector();
  const node = renderedNodes.find(n => n.name === name);
  if (focus && node && fg) fg.centerAt(node.x || 0,node.y || 0,0);
}
function selectEdge(edge) {
  const restoreFocus=selectionFocusRestorer();
  // Reading evidence keeps the direction context of a visible connection.
  // Explicit selections outside the filter still use the endpoint-pair fallback.
  const reference=dependencyMatchesSelection(edge.from,edge.to)?directionReference():null;
  ui.selection = {kind:'edge',from:nodeById(view,edge.from).name,to:nodeById(view,edge.to).name,...(reference?{reference:reference.name}:{})};
  reconcileSelection();exitCycleForSelection();pinSelectedGraphFiles();renderGraph(false); renderSidebar(); refreshSelection(false);restoreFocus();
}
function reconcileSelection() {
  selectedId = null; selectedLinkKey = null; selectedNodeIds = new Set();
  if (!ui.selection) return;
  if (ui.selection.kind === 'node') {
    const node = view.nodes.find(n => n.name === ui.selection.name);
    if (node) {selectedId = node.id;selectedNodeIds.add(node.id);}
    else {
      // An opened package is represented by its descendants in the current
      // cut. Keep that package as the reference when selected in the Explorer.
      const source=nodeByName(work,ui.selection.name);
      if(source?.kind==='module'){
        const descendants=subtree(work,source.id);
        for(const child of view.nodes)if(descendants.has(child.src))selectedNodeIds.add(child.id);
      }
      if(!selectedNodeIds.size)ui.selection=null;
    }
  } else if (ui.selection.kind === 'edge') {
    const edge = selectedEdge();
    if (edge) selectedLinkKey = edge.from + ' ' + edge.to; else ui.selection = null;
  } else if (ui.selection.kind === 'cycle' && (!ui.cycle || ui.selection.key!==ui.cycle)) {
    ui.selection = null;
  }
}
function refreshSelection(renderGraphView=true) {
  const selected=ui.selection?.kind==='node'?nodeByName(work,ui.selection.name):null;
  for(const row of document.getElementById('module-list').querySelectorAll('button[data-node]')){
    row.setAttribute('aria-pressed',String(!!selected&&row.dataset.node===String(selected.id)));
  }
  if (fg) fg.nodeCanvasObject(drawNode);
  renderBreadcrumb();
  if(renderGraphView)renderGraphControls();
  if (ui.workspace === 'dsm') updateMatrixSelection();
}
function openInspector(focus=false) {setInspectorOpen(true,focus);}
function expandNode(name) {
  const node = nodeByName(work,name);
  if (!node || !childrenOf(work,node.id).length) return;
  treeLocation=null;
  focusDefaultEdges();
  // Opening a package explicitly reveals its direct contents even while
  // unrelated files remain hidden in the package overview.
  for (const child of work.nodes) if (child.parent===node.id && child.kind!=='module') revealedGraphFiles.add(child.id);
  for (const id of [node.id,...ancestorsOf(work,node.id)]) explorerOpen.add(id);
  trail.push(node.id); isolated = null; ui.cycle = null;
  recompute(); renderAll(); fitView();
}
function revealNode(name) {
  const node = nodeByName(work,name);
  if (!node) return;
  treeLocation=null;
  isolated = null; ui.cycle = null;
  for (const id of ancestorsOf(work,node.id).reverse()) if (!trail.includes(id)) trail.push(id);
  recompute(); renderAll(false); selectNode(name,true);
}
function navigateExplorerNode(id, opened = true) {
  const node=nodeById(work,id);
  if(!node)return;
  treeBaseCollapses.clear();treeLocation=null;
  focusDefaultEdges();
  // Explorer navigation can also collapse ancestors above the report's
  // initial depth. Rebuild this path from roots and retain sibling context.
  navigationAtRoot=true;
  trail=ancestorsOf(work,id).reverse();
  if(opened&&childrenOf(work,id).length)trail.push(id);
  else explorerOpen.delete(id);
  for(const parent of trail){
    explorerOpen.add(parent);
    for(const child of work.nodes)if(child.parent===parent&&child.kind!=='module')revealedGraphFiles.add(child.id);
  }
  if(node.kind!=='module')revealedGraphFiles.add(id);
  isolated=null;ui.cycle=null;ui.selection={kind:'node',name:node.name};
  showTab('graph');
  recompute();renderAll();
}
// Inline exploration changes only this subtree. Coarsening a context row
// above the report depth is replayed on both the working and baseline cuts.
function toggleTreeBranch(id, expanded) {
  const node=nodeById(work,id);
  if(!node||!childrenOf(work,id).length)return false;
  const descendants=subtree(work,id),opened=!visible.has(id)&&[...visible].some(source=>descendants.has(source));
  if(!opened&&!visible.has(id))return false;
  const open=expanded===undefined?!opened:!!expanded;
  if(open===opened)return false;
  focusDefaultEdges();treeLocation=id;
  if(open){
    trail.push(id);
    for(const ancestor of [id,...ancestorsOf(work,id)])explorerOpen.add(ancestor);
    for(const child of work.nodes)if(child.parent===id&&child.kind!=='module')revealedGraphFiles.add(child.id);
  }else{
    treeBaseCollapses.add(id);
    trail=trail.filter(source=>!descendants.has(source));
    for(const source of descendants)explorerOpen.delete(source);
  }
  isolated=null;ui.cycle=null;ui.selection={kind:'node',name:node.name};
  recompute();showTab('graph');renderAll(false);
  return true;
}
function treeBulkState() {
  const byId=new Map(work.nodes.map(node=>[node.id,node])),parents=new Set(work.nodes.map(node=>node.parent)),seen=new Set();
  let expand=visible.size>0&&ui.searchLimit<work.nodes.length,collapse=explorerOpen.size>0;
  for(const id of visible){
    if(parents.has(id))expand=true;
    let node=byId.get(id);if(node&&byId.has(node.parent))collapse=true;
    while(node&&!seen.has(node.id)){
      seen.add(node.id);if(parents.has(node.id)&&!explorerOpen.has(node.id))expand=true;
      node=byId.get(node.parent);
    }
  }
  return {expand:expand||!!currentFilter()||!!ui.cycle,collapse:collapse||!!currentFilter()};
}
function setAllTreeBranches(expanded) {
  if(!treeBulkState()[expanded?'expand':'collapse'])return false;
  const byId=new Map(work.nodes.map(node=>[node.id,node])),children=new Map();
  for(const node of work.nodes){if(!children.has(node.parent))children.set(node.parent,[]);children.get(node.parent).push(node.id);}
  const selectedName=ui.selection?.kind==='node'?ui.selection.name:ui.selection?.kind==='edge'?ui.selection.reference:null;
  let selected=selectedName?nodeByName(work,selectedName):null;
  if(expanded){
    const stack=[...visible].reverse(),seen=new Set();
    for(const id of visible){
      let parent=byId.get(byId.get(id)?.parent);const path=new Set();
      while(parent&&!path.has(parent.id)){path.add(parent.id);explorerOpen.add(parent.id);parent=byId.get(parent.parent);}
    }
    while(stack.length){
      const id=stack.pop();if(seen.has(id))continue;seen.add(id);
      const kids=children.get(id)||[];
      if(kids.length){trail.push(id);explorerOpen.add(id);stack.push(...kids.slice().reverse());}
      else if(byId.get(id)?.kind!=='module')revealedGraphFiles.add(id);
    }
    ui.searchLimit=Math.max(ui.searchLimit,work.nodes.length);
  }else{
    const roots=new Set();
    for(const id of visible){
      let node=byId.get(id);const seen=new Set();
      while(node&&byId.has(node.parent)&&!seen.has(node.id)){seen.add(node.id);node=byId.get(node.parent);}
      if(node)roots.add(node.id);
    }
    trail=[];treeBaseCollapses.clear();for(const id of roots)treeBaseCollapses.add(id);
    explorerOpen.clear();revealedGraphFiles.clear();
    const seen=new Set();
    while(selected&&!roots.has(selected.id)&&!seen.has(selected.id)){seen.add(selected.id);selected=byId.get(selected.parent);}
    if(selected&&!roots.has(selected.id))selected=null;
  }
  // Search deliberately opens matching ancestor paths. Clear it for a full
  // tree action, while retaining the user's dependency and cycle filters.
  document.getElementById('filter').value='';
  treeLocation=selected?.id??'project';treeLastClick=null;
  ui.selection=selected?{kind:'node',name:selected.name}:null;ui.cycle=null;isolated=null;
  if(!selected)ui.edgeMode='all';
  recompute();pinSelectedGraphFiles();ui.diagram='overview';
  if(ui.workspace!=='graph')showTab('graph');
  renderAll(false);
  return true;
}
function fitView() {
  if (!fg) return;
  pendingGraphZoom=null;
  pendingFit = true;
  fitWhenReady();
}
function fitWhenReady() {
  if (!pendingFit || !fg || !layoutReady || !graphLayout.bounds || ui.workspace!=='graph') return;
  const b=graphLayout.bounds,box=document.getElementById('tab-graph').getBoundingClientRect();
  if(box.width<=0||box.height<=0)return;
  pendingFit = false;
  const zoom=Math.min(1.5,Math.max(1,box.width-120)/Math.max(1,b.maxX-b.minX),Math.max(1,box.height-120)/Math.max(1,b.maxY-b.minY));
  fg.centerAt((b.minX+b.maxX)/2,(b.minY+b.maxY)/2,0);fg.zoom(zoom,0);
}
function resetView() { if(typeof scenarioBaseCut!=='undefined')scenarioBaseCut=null;treeBaseCollapses.clear();treeLocation=null;ui.diagram='overview';trail = []; navigationAtRoot=false; isolated = null; ui.selection = null; ui.cycle = null; explorerOpen.clear();revealedGraphFiles.clear();recompute(); renderAll(); fitView(); }
function setSimulation(enabled) {
  if(isRepositoryOverview()){enforceRepositoryScope();return;}
  ui.simulation=enabled;document.getElementById('simulation-mode').setAttribute('aria-pressed',String(enabled));
  renderSidebar();renderOpsStrip();feedback(enabled?t('Simulation enabled. Proposed actions require confirmation.','Simulation aktiviert. Vorgeschlagene Aktionen müssen bestätigt werden.'):t('Browse mode. Simulated changes are preserved until you undo or clear them.','Ansichtsmodus. Simulierte Änderungen bleiben erhalten, bis du sie rückgängig machst oder löschst.'));
}
const WORKSPACE_MENUS={'project-menu':'project-summary','export-menu':'export-summary','refactor-menu':'refactor-summary','settings-menu':'settings-summary','graph-display-menu':'graph-display-summary'};
function closeWorkspaceMenu(id,restoreFocus=false) {
  const menu=document.getElementById(id),wasOpen=menu.open;
  hideMetricTooltip(menu);menu.open=false;
  if(wasOpen&&restoreFocus)document.getElementById(WORKSPACE_MENUS[id]).focus({preventScroll:true});
  return wasOpen;
}
function askSimulation(op, description, repair=null) {
  if(isRepositoryOverview())return;
  if (!ui.simulation) { feedback(t('Enable Simulation to propose changes. Browsing never edits dependencies.','Aktiviere die Simulation, um Änderungen vorzuschlagen. Beim Erkunden werden keine Abhängigkeiten verändert.')); return; }
  ui.pending = op; pendingRepair=repair; ui.previousFocus = document.activeElement;
  document.getElementById('action-description').textContent = description + t(' This changes only the simulated graph; source files remain unchanged.',' Dies ändert nur den simulierten Graphen; Quelldateien bleiben unverändert.');
  const dialog = document.getElementById('action-dialog');
  dialog.showModal(); document.getElementById('cancel-action').focus();
}
function closeDialog() {
  if(typeof cancelScenarioAction==='function')cancelScenarioAction();
  document.getElementById('action-dialog').close(); ui.pending = null; pendingRepair=null;
  if (ui.previousFocus && ui.previousFocus.isConnected) ui.previousFocus.focus();
}
function copyText(text) {
  if (!navigator.clipboard || !navigator.clipboard.writeText) { feedback(t('Clipboard is unavailable. Select and copy the displayed text manually.','Die Zwischenablage ist nicht verfügbar. Markiere und kopiere den angezeigten Text manuell.')); return; }
  navigator.clipboard.writeText(text).then(() => feedback(t('Copied.','Kopiert.')), () => feedback(t('Copy failed. Select and copy the displayed text manually.','Kopieren fehlgeschlagen. Markiere und kopiere den angezeigten Text manuell.')));
}

// ── theme + palette (U01) ────────────────────────────────────────────────────
// ONE palette source of truth: the CSS custom properties defined in the
// template. The canvas colour object C is (re)read from them via
// getComputedStyle, so DOM and canvas can never drift apart — and a theme
// switch restyles panels (CSS) and graph (accessors re-read C every draw)
// together. Light/Dark/System is a user preference; System follows
// prefers-color-scheme live. The preference is stored in localStorage but
// storage is OPTIONAL: file:// reports or disabled storage fall back to an
// in-memory-only preference (safe default: system).
const mediaDark = window.matchMedia('(prefers-color-scheme: dark)');
const THEME_KEY = 'archguard-theme';
const CANVAS_VARS = [
  ['node', '--c-node', '#2563EB'], ['dim', '--c-dim', '#D7DEE9'],
  ['cycle', '--c-cycle', '#D92D20'], ['label', '--c-label', '#172033'],
  ['edge', '--c-edge', '#64748B'], ['resolved', '--c-resolved', '#1E8E3E'],
  ['incoming', '--c-incoming', '#0369A1'], ['outgoing', '--c-outgoing', '#7C3AED'],
  ['introduced', '--c-introduced', '#D50000'], ['panel', '--panel', '#FFFFFF'], ['border','--border','#DDE3ED'], ['muted','--muted','#5B6472'],
  ['packageFill','--c-package','#FFFAED'], ['packageTab','--c-package-tab','#F1D998'],
  ['packageBorder','--c-package-border','#A77D32'], ['packageSelected','--c-package-selected','#EDF4FF'],
  ['priority','--c-priority','#9A4D00'],['priorityFill','--c-priority-fill','#FFE0A3'],['priorityText','--c-priority-text','#502500'],
];
const C = {}; // canvas palette — populated by refreshPalette(), never hardcoded
function refreshPalette() {
  const cs = getComputedStyle(document.documentElement);
  for (const entry of CANVAS_VARS) {
    const v = (cs.getPropertyValue(entry[1]) || '').trim();
    C[entry[0]] = v || entry[2];
  }
}
let themePref = 'system';
function applyTheme(pref) {
  themePref = (pref === 'light' || pref === 'dark' || pref === 'system') ? pref : 'system';
  const resolved = themePref === 'system' ? (mediaDark.matches ? 'dark' : 'light') : themePref;
  document.documentElement.setAttribute('data-theme', resolved);
  try { localStorage.setItem(THEME_KEY, themePref); } catch (err) { /* optional */ }
  for (const b of document.querySelectorAll('#theme-switch button')) {
    b.setAttribute('aria-pressed', b.dataset.mode === themePref ? 'true' : 'false');
  }
  refreshPalette();
  buildLegend(); // legend strokes take the (new) edge colour
  if (fg) { fg.backgroundColor(getComputedStyle(document.documentElement).getPropertyValue('--bg').trim()); fg.nodeCanvasObject(drawNode); }
  if(ui.workspace==='graph'&&ui.diagram==='overview')renderPackageTree();
  if(ui.workspace==='services')renderServices();
  if(ui.workspace==='start')worldDraw();
}
function storedTheme() {
  try {
    const v = localStorage.getItem(THEME_KEY);
    if (v === 'light' || v === 'dark' || v === 'system') return v;
  } catch (err) { /* storage unavailable — fall back to system */ }
  return 'system';
}

// ── working model ──────────────────────────────────────────────────────────
// `work` is the mutated copy the ops are applied to, exactly like the
// `g.clone()` inside apply_ops (archguard-analysis/src/simulate.rs:127).
function baseline() {
  return {
    nodes: data.nodes.map(n => ({ id: n.id, name: n.name, kind: n.kind,
                                  parent: n.parent, lang: n.lang })),
    edges: data.edges.map(e => ({ from: e.from, to: e.to,
                                  refs: e.refs.map(r => ({ file: r.file,
                                                           line: r.line,
                                                           raw: r.raw })),
                                  // F12: exact total; null for legacy graphs.
                                  count: e.count === undefined ? null : e.count })),
  };
}
// F12: the shown weight is the exact total when known — otherwise the
// sample length as documented lower bound (mirrors Edge::weight,
// archguard-core/src/lib.rs).
function edgeWeight(e) {
  return (e.count === undefined || e.count === null) ? e.refs.length : e.count;
}
// A total without an exact count is a lower bound — label it as such.
function weightLabel(e) {
  return edgeWeight(e) + (e.count === undefined || e.count === null ? '+' : '');
}
let ops = [];        // recorded SimOps, applied in order (all-or-nothing)
let opBatches = [];  // one undo step per confirmed plan, including multi-edge plans
let repairComparison=null, pendingRepair=null, appliedRepair=null;
let work = baseline();
let replayedOps = null;
let moveSuggestions = null; // computed only for a selected package, keyed by source ID
const explorerOpen = new Set(); // source IDs survive renames, moves and undo
const revealedGraphFiles = new Set(); // explicit selections stay visible until Hide files/Reset
let trail = [];      // breadcrumb: ids expanded from the start cut
let navigationAtRoot = false; // explicit Explorer navigation starts above any report depth
const treeBaseCollapses=new Set(); // explicit coarsening of a report's initial cut
let treeLocation=null; // source ID, 'project' for bulk navigation, or null to follow the trail
let packageExpansions=[]; // current pre-expansion cuts for selection-only cycle context
let visible = new Set();
let view = { nodes: [], edges: [], srcToDense: new Map() };
let sccs = [];
let cyclicKeys = new Set();       // "from to" (view ids) currently cyclic
let resolvedKeys = new Set();     // cyclic in the original scope, acyclic now
let introducedKeys = new Set();   // newly cyclic relative to the original scope
let isolated = null;              // Set of view ids when a cycle is focused
let packageContext = null;        // display-only cycle context of the last opened package
let selectedId = null;
let selectedNodeIds = new Set();  // one visible node or the descendants of an opened package
let selectedLinkKey = null;       // F17: "srcId tgtId" of the selected link
let currentReview=null,reviewBySource=new Map();

// Graph helpers — mirror Graph::children_of (archguard-core/src/lib.rs:99)
// and Graph::ancestors_of (lib.rs:113, cycle-safe).
function nodeById(g, id) { return g.nodes.find(n => n.id === id); }
function childrenOf(g, id) {
  return g.nodes.filter(n => n.parent === id).map(n => n.id).sort((a, b) => a - b);
}
function ancestorsOf(g, id) {
  const out = [];
  const seen = new Set();
  const self = nodeById(g, id);
  let cur = self ? self.parent : null;
  while (cur !== null && cur !== undefined) {
    if (seen.has(cur)) break;
    seen.add(cur);
    out.push(cur);
    const p = nodeById(g, cur);
    cur = p ? p.parent : null;
  }
  return out;
}
// subtree(root) — mirror of simulate.rs subtree() (root plus descendants).
function subtree(g, id) {
  const out = new Set([id]);
  const stack = [id];
  while (stack.length) {
    for (const c of childrenOf(g, stack.pop())) {
      if (!out.has(c)) { out.add(c); stack.push(c); }
    }
  }
  return out;
}

// ── Cut mirror (archguard-core/src/view.rs) ───────────────────────────────────
// at_depth — mirrors Cut::at_depth (view.rs:74): every node at depth d
// plus every shallower childless node.
function atDepth(g, d) {
  const out = new Set();
  for (const n of g.nodes) {
    const depth = ancestorsOf(g, n.id).length;
    if (depth === d || (depth < d && childrenOf(g, n.id).length === 0)) out.add(n.id);
  }
  return out;
}
// expand — mirrors Cut::expand (view.rs:90): replace a visible node by
// its children; no-op for invisible or childless nodes.
function cutExpand(g, cut, id) {
  if (!cut.has(id)) return cut;
  const kids = childrenOf(g, id);
  if (kids.length === 0) return cut;
  const out = new Set(cut);
  out.delete(id);
  for (const k of kids) out.add(k);
  return out;
}
// nearest_visible — mirrors nearest_visible (view.rs:264).
function nearestVisible(g, cut, id, byId) {
  const seen=new Set();
  while(id!==null&&id!==undefined&&!seen.has(id)){
    if(cut.has(id))return id;
    seen.add(id);id=(byId?byId.get(id):nodeById(g,id))?.parent;
  }
  return undefined;
}

// ── aggregate mirror (archguard-core/src/view.rs:184-252) ─────────────────────
// Same semantics as the Rust aggregate(): endpoints mapped to their
// nearest visible ancestor-or-self, self-edges dropped, parallel edges
// merged with refs summed (then sorted by (file,line,raw), deduped,
// capped at MAX_SAMPLE_REFS = 10) and F12 totals summed with
// None-poisoning (a contributor without a total poisons the sum —
// never recounted from the capped sample), nodes sorted by name and
// renumbered densely, edges sorted by (from, to).
function aggregate(g, cut) {
  const MAX_SAMPLE_REFS = 10; // view.rs:20
  // Replayed tree cuts share many edge endpoints. Index the current graph
  // once per aggregation, and cache this cut's nearest visible endpoint.
  const byId=new Map(g.nodes.map(n=>[n.id,n])),nearest=new Map();
  const endpoint=id=>{if(!nearest.has(id))nearest.set(id,nearestVisible(g,cut,id,byId));return nearest.get(id);};
  const vis = g.nodes.filter(n => cut.has(n.id))
    .slice()
    .sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : a.id - b.id));
  const idMap = new Map();
  const srcToDense = new Map();
  const nodes = [];
  vis.forEach((n, i) => {
    idMap.set(n.id, i);
    srcToDense.set(n.id, i);
    nodes.push({ id: i, src: n.id, name: n.name, kind: n.kind, lang: n.lang });
  });
  const merged = new Map(); // "from to" → { count, refs } accumulator
  for (const e of g.edges) {
    const f = endpoint(e.from);
    const t = endpoint(e.to);
    if (f === undefined || t === undefined) continue; // broken cut → no edge
    const fi = idMap.get(f);
    const ti = idMap.get(t);
    if (fi === ti) continue;                           // self-edge dropped
    const key = fi + ' ' + ti;
    let slot = merged.get(key);
    if (!slot) { slot = { count: undefined, refs: [] }; merged.set(key, slot); }
    // F12: totals are SUMMED with None-poisoning (mirrors view.rs) — a
    // contributor without a total poisons the sum, never the (capped)
    // sample length. undefined = no contributor yet, null = poisoned.
    const c = (e.count === undefined || e.count === null) ? null : e.count;
    slot.count = (slot.count === null || c === null)
      ? null
      : (slot.count === undefined ? c : slot.count + c);
    for (const r of e.refs) slot.refs.push({ file: r.file, line: r.line, raw: r.raw });
  }
  const keys = [...merged.keys()].sort((ka, kb) => {
    const a = ka.split(' ');
    const b = kb.split(' ');
    return (+a[0] - +b[0]) || (+a[1] - +b[1]);
  });
  const edges = keys.map(k => {
    const { count, refs } = merged.get(k);
    refs.sort((x, y) => (x.file < y.file ? -1 : x.file > y.file ? 1
      : (x.line - y.line) || (x.raw < y.raw ? -1 : x.raw > y.raw ? 1 : 0)));
    const dedup = [];
    for (const r of refs) {
      const last = dedup[dedup.length - 1];
      if (!last || last.file !== r.file || last.line !== r.line || last.raw !== r.raw) {
        dedup.push(r);
      }
    }
    const [from, to] = k.split(' ').map(Number);
    return { from: from, to: to, refs: dedup.slice(0, MAX_SAMPLE_REFS), count: count };
  });
  return { nodes: nodes, edges: edges, srcToDense: srcToDense };
}

// ── Tarjan mirror (archguard-analysis/src/scc.rs find_sccs) ───────────────────
// Iterative Tarjan; only components with >1 member or a self-loop count
// (scc.rs:61); members sorted by (name, id), components by smallest
// member name then members (scc.rs:66-74); ids scc-N are CLIENT-side and
// may differ from the server finding ids (documented simplification).
// internal edges of one SCC — mirror of scc.rs internal_edges().
function internalEdges(v, comp) {
  const members = new Set(comp.members);
  const out = [];
  for (const e of v.edges) {
    if (members.has(e.from) && members.has(e.to)) out.push([e.from, e.to]);
  }
  return out;
}

// ── lakos mirror (archguard-analysis/src/lakos.rs:46, view level only) ────────
// condense (client SCCs + singletons) → Kahn topo → memoized reachable
// sets → CCD = Σ comp_size × reached, ACD = CCD / n (lakos.rs:119-128).
function viewAcd(v) {
  const n = v.nodes.length;
  if (n === 0) return 0.0;
  const compOf = new Map();
  const compSize = [];
  for (const c of findSccs(v)) {
    const id = compSize.length;
    compSize.push(c.members.length);
    for (const m of c.members) compOf.set(m, id);
  }
  for (const nd of v.nodes) {
    if (!compOf.has(nd.id)) { compOf.set(nd.id, compSize.length); compSize.push(1); }
  }
  const succs = compSize.map(() => new Set());
  for (const e of v.edges) {
    const f = compOf.get(e.from);
    const t = compOf.get(e.to);
    if (f === undefined || t === undefined) continue;
    if (f !== t) succs[f].add(t);
  }
  const indeg = compSize.map(() => 0);
  succs.forEach(s => s.forEach(t => { indeg[t] += 1; }));
  const queue = [];
  for (let c = 0; c < compSize.length; c += 1) if (indeg[c] === 0) queue.push(c);
  const topo = [];
  while (queue.length) {
    const c = queue.shift();
    topo.push(c);
    succs[c].forEach(s2 => { indeg[s2] -= 1; if (indeg[s2] === 0) queue.push(s2); });
  }
  const reach = compSize.map(() => new Set());
  for (let i = topo.length - 1; i >= 0; i -= 1) {
    const c = topo[i];
    const set = new Set();
    succs[c].forEach(s2 => { set.add(s2); reach[s2].forEach(x => set.add(x)); });
    reach[c] = set;
  }
  let ccd = 0;
  for (let c = 0; c < compSize.length; c += 1) {
    let reached = compSize[c];
    reach[c].forEach(s2 => { reached += compSize[s2]; });
    ccd += reached * compSize[c];
  }
  return ccd / n;
}

// ── SimOps: T37 encoder + apply_ops mirror ─────────────────────────────────
// Frozen wire format (crates/archguard-analysis/src/simulate.rs, externally
// tagged snake_case enum): the Export SimOps button downloads exactly
// this array; it feeds `archguard simulate --ops` / MCP simulate. The
// viewer UI records move_node and cut_edge; merge shares the array
// shape but has no viewer UI (T39).
function encodeCut(from, to) { return { cut_edge: { from: from, to: to } }; }
function encodeMove(node, newParent) { return { move_node: { node: node, new_parent: newParent } }; }

function nodeByName(g, name) { return g.nodes.find(n => n.name === name); }

// applyOp — mirrors apply_op/cut_edge/move_node (simulate.rs:176-225);
// failures throw, mirroring the all-or-nothing ArchguardError::Analysis.
function applyOp(g, op) {
  if (op.cut_edge) {
    const from = op.cut_edge.from;
    const to = op.cut_edge.to;
    const f = nodeByName(g, from);
    const t = nodeByName(g, to);
    if (!f) throw new Error('simulate: unknown node "' + from + '"');
    if (!t) throw new Error('simulate: unknown node "' + to + '"');
    const fs = subtree(g, f.id);
    const ts = subtree(g, t.id);
    // subtree-inclusion rule, simulate.rs:192.
    g.edges = g.edges.filter(e => !(fs.has(e.from) && ts.has(e.to)));
  } else if (op.move_node) {
    const node = op.move_node.node;
    const newParent = op.move_node.new_parent;
    const m = nodeByName(g, node);
    const p = nodeByName(g, newParent);
    if (!m) throw new Error('simulate: unknown node "' + node + '"');
    if (!p) throw new Error('simulate: unknown node "' + newParent + '"');
    if (p.kind !== 'module') {
      throw new Error('simulate: move target "' + newParent + '" is not a module');
    }
    const sub = subtree(g, m.id);
    if (sub.has(p.id)) {
      throw new Error('simulate: cannot move "' + node + '" into itself or its own subtree');
    }
    const base = node.split('/').pop();
    const newBase = p.name + '/' + base; // simulate.rs:218
    if (g.nodes.some(x => !sub.has(x.id) && x.name === newBase)) {
      throw new Error('simulate: name "' + newBase + '" already exists');
    }
    // rename_subtree with keep_root=true (simulate.rs:341): the moved
    // node takes newBase, descendants keep their path below it.
    for (const x of g.nodes) {
      if (!sub.has(x.id)) continue;
      if (x.id === m.id) x.name = newBase;
      else if (x.name.indexOf(node + '/') === 0) x.name = newBase + x.name.slice(node.length);
    }
    m.parent = p.id; // node ids survive — edges follow for free
  } else {
    throw new Error('simulate: unknown op');
  }
}
function moveTargetAllowed(g, node, parent) {
  if (!node || parent.kind !== 'module' || parent.id === node.parent) return false;
  try {
    // Reuse the full SimOp validation. A move mutates only node names/parents;
    // its probe can share the untouched import edges with the working graph.
    applyOp({nodes:g.nodes.map(n=>({...n})),edges:g.edges},encodeMove(node.name,parent.name));
    return true;
  } catch (_error) { return false; }
}
function packageMoveSuggestions(g) {
  const byId=new Map(g.nodes.map(n=>[n.id,n])),chains=new Map(),affinities=new Map();
  for(const node of g.nodes){
    const chain=[],seen=new Set();let current=node;
    while(current&&!seen.has(current.id)){
      seen.add(current.id);if(current.kind==='module')chain.push(current);
      current=byId.get(current.parent);
    }
    chains.set(node.id,chain.reverse());
  }
  // Only sibling packages compete. Each import belongs to the pair at the
  // first divergence of its endpoint ancestry; imports inside the source
  // subtree never count, and outer containers cannot win by containing it.
  for(const edge of g.edges){
    const from=chains.get(edge.from)||[],to=chains.get(edge.to)||[];
    let i=0;while(i<from.length&&i<to.length&&from[i].id===to[i].id)i++;
    const a=from[i],b=to[i];if(!a||!b||a.parent!==b.parent)continue;
    for(const [source,target,direction] of [[a,b,'outgoing'],[b,a,'incoming']]){
      if(!affinities.has(source.id))affinities.set(source.id,new Map());
      const targets=affinities.get(source.id);
      if(!targets.has(target.id))targets.set(target.id,{target:target.name,targetId:target.id,outgoing:0,incoming:0,outgoingLowerBound:false,incomingLowerBound:false,total:0,lowerBound:false});
      const affinity=targets.get(target.id);
      affinity[direction]+=edgeWeight(edge);
      affinity[direction+'LowerBound'] ||= edge.count==null;
      affinity.lowerBound ||= edge.count==null;
      affinity.total=affinity.outgoing+affinity.incoming;
    }
  }
  const suggestions=new Map();
  for(const [source,targets] of affinities){
    const ranked=[...targets.values()].filter(a=>a.total>0).sort((a,b)=>b.total-a.total||compareNames(a.target,b.target));
    const best=ranked.find(candidate=>moveTargetAllowed(g,byId.get(source),byId.get(candidate.targetId)));
    if(best)suggestions.set(source,{...best,rankingLowerBound:ranked.some(a=>a.lowerBound)});
  }
  return suggestions;
}
function referenceAmount(value, lowerBound) { return (lowerBound?t('at least ','mindestens '):'')+value; }
// replayOps — mirrors apply_ops (simulate.rs:127): pristine clone, then
// every op in order. Valid ops are guaranteed by recordOp's probe check.
function replayOps() {
  const key=JSON.stringify(ops);
  if(key===replayedOps)return false;
  const next=baseline();
  for (const op of ops) applyOp(next, op);
  work=next;replayedOps=key;
  return true;
}

// ── state recomputation ────────────────────────────────────────────────────
// One path for every state change: replay ops on the clone, derive the
// cut (initial visible or roots + trail replay through the Cut::expand mirror), then
// aggregate + client Tarjan + static cycle diff against the same baseline scope.
function startCut(graph) {
  let cut=typeof scenarioBaseCut!=='undefined'&&scenarioBaseCut?new Set(scenarioBaseCut):navigationAtRoot?atDepth(graph,0):data.view.visible?new Set(data.view.visible):atDepth(graph,data.view.depth);
  // A saved post-move cut can contain a node beneath another selected node
  // after Undo or when comparing the baseline. Its ancestor already represents
  // the complete subtree; keep that coverage without counting the child twice.
  cut=new Set([...cut].filter(id=>!ancestorsOf(graph,id).some(parent=>cut.has(parent))));
  // Parent collapses dominate child collapses. Only already represented
  // descendants are replaced, so a focused report gains no unrelated roots.
  const collapsed=[...treeBaseCollapses].sort((a,b)=>ancestorsOf(graph,a).length-ancestorsOf(graph,b).length);
  for(const id of collapsed){
    if(!nodeById(graph,id))continue;
    const descendants=subtree(graph,id);
    if([...cut].some(source=>source!==id&&descendants.has(source))){
      cut=new Set([...cut].filter(source=>!descendants.has(source)));cut.add(id);
    }
  }
  return cut;
}
function recompute() {
  enforceRepositoryScope();
  if(replayOps())moveSuggestions=null;
  let cut = startCut(work);
  // Replay the trail entry by entry; an entry that changes nothing (e.g.
  // invalidated by a recorded op) drops out of the breadcrumb.
  const kept = [], expansions = [];
  for (const id of trail) {
    const next = cutExpand(work, cut, id);
    if (next !== cut) {kept.push(id);expansions.push({id,cut});}
    cut = next;
  }
  trail = kept;
  visible = cut;
  view = aggregate(work, visible);
  sccs = findSccs(view);
  cyclicKeys = new Set();
  for (const c of sccs) {
    for (const p of internalEdges(view, c)) cyclicKeys.add(p[0] + ' ' + p[1]);
  }
  // Compare the same expansion scope before/after, using original source IDs.
  // Dense view IDs may change after a move or zoom and cannot identify an edge.
  resolvedKeys = new Set(); introducedKeys = new Set();
  if(ops.length){
    const original = baseline();
    let originalCut = startCut(original);
    for (const id of trail) originalCut = cutExpand(original, originalCut, id);
    const originalView = aggregate(original, originalCut), originalCycles = findSccs(originalView);
    baselineStats = { acd: viewAcd(originalView), cycles: originalCycles.length };
    const originalPairs = new Set();
    for (const cycle of originalCycles) for (const [from,to] of internalEdges(originalView,cycle)) {
      originalPairs.add(nodeById(originalView,from).src+' '+nodeById(originalView,to).src);
    }
    for (const edge of view.edges) {
      const key=edge.from+' '+edge.to;
      const wasCyclic=originalPairs.has(nodeById(view,edge.from).src+' '+nodeById(view,edge.to).src);
      if (wasCyclic&&!cyclicKeys.has(key)) resolvedKeys.add(key);
      if (!wasCyclic&&cyclicKeys.has(key)) introducedKeys.add(key);
    }
  }
  packageExpansions=expansions;
  packageContext = expandedPackageContext(packageExpansions);
  if (ui.cycle) {
    const cycle = cyclesForView().find(c => cycleKey(c) === ui.cycle);
    isolated = cycle ? new Set(view.nodes.filter(n => cycle.nodes.some(m => n.name === m || n.name.startsWith(m + '/') || m.startsWith(n.name + '/'))).map(n => n.id)) : null;
    if (!cycle) { ui.cycle = null; if (ui.selection && ui.selection.kind === 'cycle') ui.selection = null; }
  } else isolated = null;
  reconcileSelection();
  currentReview=computeReview(work,view,data.reviewContext||{});
  currentAcd = currentReview.summary.acd;
  if(!ops.length)baselineStats={acd:currentAcd,cycles:sccs.length};
  reviewBySource=new Map(currentReview.nodes.filter(row=>row.source_id!==null).map(row=>[row.source_id,row]));
  viewRevision += 1; ui.matrixPage = 0;
}

// Preserve cycle context across a drill-down without changing the analytical
// cut. A cycle between packages need not be a cycle between their files: keep
// its real projected edges as context, never add them to cyclicKeys or metrics.
function expandedPackageContext(expansions) {
  const reference=ui.selection?.kind==='node'?ui.selection.name:ui.selection?.kind==='edge'?ui.selection.reference:null;
  const selected=reference&&nodeByName(work,reference),path=selected?[selected.id,...ancestorsOf(work,selected.id)]:[];
  if(!selected&&treeLocation==='project')return null;
  const active=selected?path.map(id=>expansions.find(entry=>entry.id===id)).find(Boolean):expansions[expansions.length-1];
  if(!active)return null;
  const node=nodeById(work,active.id);
  if(!node||node.kind!=='module')return null;
  const descendants=subtree(work,node.id),ids=new Set(view.nodes.filter(n=>descendants.has(n.src)).map(n=>n.id));
  if(!ids.size)return null;
  const context={name:node.name,ids,nodes:new Set(),edges:new Set(),levels:[]};
  const add=(from,to)=>{context.edges.add(from+' '+to);context.nodes.add(from);context.nodes.add(to);};
  for(const cycle of sccs)if(cycle.members.some(id=>ids.has(id))&&cycle.members.some(id=>!ids.has(id))){
    for(const [from,to] of internalEdges(view,cycle))add(from,to);
  }
  const ancestors=new Set([node.id,...ancestorsOf(work,node.id)]),seen=new Set();
  const sourceNodes=new Map(work.nodes.map(n=>[n.id,n]));
  let watched=null;
  for(let index=0;index<expansions.length;index++){
    const expansion=expansions[index];
    // A sibling's earlier cut can contain the relevant coarse cycle, so
    // inspect cut membership rather than filtering by the expanded branch.
    if(![...ancestors].some(id=>expansion.cut.has(id)))continue;
    // Refining a node outside this SCC cannot change its return paths. Revisit
    // only when a member is expanded; without a cycle, watch the ancestor
    // itself because its children may reveal a previously internal cycle.
    if(watched&&!watched.has(expansions[index-1].id))continue;
    const coarse=aggregate(work,expansion.cut),opened=coarse.nodes.find(n=>ancestors.has(n.src));
    const cycle=opened&&findSccs(coarse).find(c=>c.members.includes(opened.id));
    watched=new Set(cycle?cycle.members.map(id=>coarse.nodes[id].src):opened?[opened.src]:[]);
    if(!cycle)continue;
    const pairs=new Set(internalEdges(coarse,cycle).map(([from,to])=>from+' '+to));
    const dense=new Map(coarse.nodes.map(n=>[n.src,n.id]));
    const projected=new Map(view.nodes.map(n=>[n.id,dense.get(nearestVisible(work,expansion.cut,n.src,sourceNodes))]));
    const edges=view.edges.filter(e=>pairs.has(projected.get(e.from)+' '+projected.get(e.to)));
    // Select the whole return path, not just edges incident to the package.
    if(!edges.some(e=>ids.has(e.from)!==ids.has(e.to)))continue;
    for(const edge of edges)add(edge.from,edge.to);
    const entry={id:'Package-level cycle · '+opened.name,nodes:cycle.members.map(id=>nodeById(coarse,id).name),edge_count:pairs.size,suggested_cuts:[],origin:'package'};
    const key=cycleKey(entry);
    if(!seen.has(key)){context.levels.push(entry);seen.add(key);}
  }
  return context;
}

// ── rendering ──────────────────────────────────────────────────────────────

let renderedNodes = [];
let renderedLinks = [];
let pendingFit = false;
let pendingGraphZoom = null;
let layoutReady = false;
let graphLayout = {nodes:[],edges:[],positions:new Map(),bounds:null};
let treeSignature = '';

function hasPosition(node) { return !!node && Number.isFinite(node.x) && Number.isFinite(node.y); }
function radiusOf(n) { const box = nodeBox(n); return Math.hypot(box.w,box.h) / 2; }
function nodeBox(n) {
  const label = n.shortLabel || n.name.split('/').pop() || n.name;
  if (n.kind === 'module') {
    const w = Math.min(240,Math.max(164,label.length * 8 + 32));
    return {w,h:84,tabWidth:Math.min(96,w*.45),tabHeight:16};
  }
  return {w:Math.min(180,Math.max(88,label.length * 6.6 + 28)),h:32};
}
// Containment alone determines the layout. Dependency edges (including cycles)
// never become tree edges or influence coordinates. Context stays outside view.
function containmentLayout(g, visibleIds, projectName='Project') {
  const visible=new Set(visibleIds),byId=new Map(g.nodes.map(n=>[n.id,n])),included=new Set();
  for(const id of visible){
    let node=byId.get(id);const seen=new Set();
    while(node&&!seen.has(node.id)){seen.add(node.id);included.add(node.id);node=byId.get(node.parent);}
  }
  if(!included.size)return {nodes:[],edges:[],positions:new Map(),bounds:null};
  const sourceChildren=new Map();
  for(const id of included){
    const node=byId.get(id),parent=included.has(node.parent)?node.parent:null;
    if(!sourceChildren.has(parent))sourceChildren.set(parent,[]);
    sourceChildren.get(parent).push(id);
  }
  for(const siblings of sourceChildren.values())siblings.sort((a,b)=>{
    const x=byId.get(a),y=byId.get(b);return (x.kind==='module'?0:1)-(y.kind==='module'?0:1)||compareNames(x.name,y.name)||a-b;
  });
  const children=new Map(),nodesById=new Map(),order=[null];
  nodesById.set(null,{src:null,name:projectName,kind:'project',context:true,project:true,sourceIds:[],contextNames:[],w:Math.min(320,Math.max(220,projectName.length*7+70)),h:34});
  const pending=(sourceChildren.get(null)||[]).map(id=>({id,parent:null}));
  for(let i=0;i<pending.length;i++){
    const {id,parent}=pending[i],chain=[id];let last=id;
    // Only context is compressed. Every visible package/file retains its exact
    // source identity and gets its own card/position, even along a unary chain.
    while(!visible.has(last)){
      const kids=sourceChildren.get(last)||[];
      if(kids.length!==1||visible.has(kids[0])||chain.includes(kids[0]))break;
      last=kids[0];chain.push(last);
    }
    const source=byId.get(last),context=!visible.has(last);
    const box=context?{w:Math.min(360,Math.max(180,source.name.length*6+32)),h:34}:nodeBox({...source,shortLabel:source.name});
    nodesById.set(last,{src:last,name:source.name,kind:source.kind,context,project:false,sourceIds:chain,contextNames:context?chain.map(id=>byId.get(id).name):[],w:box.w,h:box.h});
    order.push(last);
    if(!children.has(parent))children.set(parent,[]);children.get(parent).push(last);
    for(const child of sourceChildren.get(last)||[])pending.push({id:child,parent:last});
  }
  const packs=new Map(),gapX=28,gapY=32,headerGap=24;
  for(const id of order.slice().reverse()){
    const node=nodesById.get(id),kids=children.get(id)||[];
    if(!kids.length){packs.set(id,{w:node.w,h:node.h,rows:[],gutter:0,gridWidth:0});continue;}
    const gutter=kids.length>1?24:0;
    let best=null;
    // Try a bounded set of row lengths, preserving sorted sibling order.
    // Whole subtrees occupy cells, so wrapping cannot overlap descendants.
    const candidates=Math.min(kids.length,Math.max(4,Math.ceil(Math.sqrt(kids.length)*2)));
    for(let columns=1;columns<=candidates;columns++){
      const rows=[];
      for(let start=0;start<kids.length;start+=columns){
        const ids=kids.slice(start,start+columns);
        rows.push({ids,w:ids.reduce((sum,child)=>sum+packs.get(child).w,0)+(ids.length-1)*gapX,h:Math.max(...ids.map(child=>packs.get(child).h))});
      }
      const gridWidth=Math.max(...rows.map(row=>row.w));
      const w=Math.max(node.w,gridWidth+gutter),h=node.h+headerGap+rows.reduce((sum,row)=>sum+row.h,0)+(rows.length-1)*gapY;
      const score=Math.abs(Math.log(w/h/1.5));
      if(!best||score<best.score)best={w,h,rows,gutter,gridWidth,score};
    }
    packs.set(id,best);
  }
  const root=packs.get(null),nodes=[],edges=[],positions=new Map(),queue=[{id:null,left:-root.w/2,top:0}];
  for(let i=0;i<queue.length;i++){
    const {id,left,top}=queue[i],node=nodesById.get(id),pack=packs.get(id);
    node.x=left+pack.w/2;node.y=top+node.h/2;nodes.push(node);
    if(id!==null)positions.set(id,{x:node.x,y:node.y});
    const areaLeft=left+(pack.w-pack.gridWidth-pack.gutter)/2;
    let rowTop=top+node.h+headerGap;
    for(const row of pack.rows){
      let childLeft=areaLeft+pack.gutter+(pack.gridWidth-row.w)/2;
      for(const child of row.ids){
        // Later rows branch from an outside rail, never through earlier cards.
        edges.push({from:id,to:child,railX:pack.gutter?areaLeft+pack.gutter/2:null,rowTop});
        queue.push({id:child,left:childLeft,top:rowTop});childLeft+=packs.get(child).w+gapX;
      }
      rowTop+=row.h+gapY;
    }
  }
  const bounds={minX:-root.w/2,maxX:root.w/2,minY:0,maxY:root.h};
  return {nodes,edges,positions,bounds};
}
function drawContainment(ctx,scale) {
  if(!graphLayout.nodes.length)return;
  const byId=new Map(graphLayout.nodes.map(n=>[n.src,n]));
  ctx.save();ctx.strokeStyle=C.muted;ctx.lineWidth=1/scale;ctx.globalAlpha=.55;
  for(const edge of graphLayout.edges){
    const from=byId.get(edge.from),to=byId.get(edge.to),b=to.context?to:nodeBox(to);
    const start=from.y+from.h/2,end=to.y-b.h/2+(!to.context&&to.kind==='module'?b.tabHeight:0),bus=edge.rowTop-12;
    ctx.beginPath();ctx.moveTo(from.x,start);
    if(edge.railX!==null){ctx.lineTo(from.x,start+8);ctx.lineTo(edge.railX,start+8);ctx.lineTo(edge.railX,bus);}
    else ctx.lineTo(from.x,bus);
    ctx.lineTo(to.x,bus);ctx.lineTo(to.x,end);ctx.stroke();
  }
  ctx.globalAlpha=1;
  for(const node of graphLayout.nodes.filter(n=>n.context)){
    ctx.save();if(!node.project)ctx.setLineDash([4/scale,3/scale]);
    ctx.beginPath();ctx.roundRect(node.x-node.w/2,node.y-node.h/2,node.w,node.h,6);ctx.fillStyle=C.panel;ctx.fill();ctx.strokeStyle=C.border;ctx.stroke();
    ctx.font=(node.project?'600 12px':'11px')+' system-ui';ctx.fillStyle=node.project?C.label:C.muted;ctx.textAlign='center';ctx.textBaseline='middle';
    ctx.fillText((node.project?t('PROJECT · ','PROJEKT · '):'')+node.name,node.x,node.y,node.w-20);ctx.restore();
  }
  ctx.restore();
}
function hitNode(n,x,y) {
  if (!hasPosition(n)) return false;
  const box=nodeBox(n),left=n.x-box.w/2,top=n.y-box.h/2;
  if (x<left||x>left+box.w||y<top||y>top+box.h) return false;
  return n.kind!=='module'||y>=top+box.tabHeight||x<=left+box.tabWidth;
}
// The same package contour is painted visibly and into the library's hit map.
function nodeOutline(n,ctx) {
  const box=nodeBox(n),left=n.x-box.w/2,top=n.y-box.h/2;
  ctx.beginPath();
  if(n.kind==='module'){
    ctx.moveTo(left,top);ctx.lineTo(left+box.tabWidth,top);
    ctx.lineTo(left+box.tabWidth,top+box.tabHeight);
    ctx.lineTo(left+box.w,top+box.tabHeight);
    ctx.lineTo(left+box.w,top+box.h);ctx.lineTo(left,top+box.h);ctx.closePath();
  }else ctx.roundRect(left,top,box.w,box.h,8);
}
function curvePoint(l,t) {
  const cp = [(l.source.x+l.target.x)/2+CURVATURE*(l.target.y-l.source.y), (l.source.y+l.target.y)/2-CURVATURE*(l.target.x-l.source.x)];
  return {x:(1-t)*(1-t)*l.source.x+2*(1-t)*t*cp[0]+t*t*l.target.x,y:(1-t)*(1-t)*l.source.y+2*(1-t)*t*cp[1]+t*t*l.target.y};
}
function nodeRectangles(node) {
  const box=nodeBox(node),left=node.x-box.w/2,top=node.y-box.h/2;
  return node.kind==='module'
    ? [[left,top,left+box.tabWidth,top+box.tabHeight],[left,top+box.tabHeight,left+box.w,top+box.h]]
    : [[left,top,left+box.w,top+box.h]];
}
function arrowGeometry(l, gap=ARROW_GAP) {
  // A curve can cross the tab, its cutout and then the body. Clip at the
  // FIRST entry into either rectangle, so a gap cannot put the arrow back
  // inside the tab. Solve the quadratic intersections instead of sampling
  // past a narrow tab on a long edge.
  if(hitNode(l.target,l.source.x,l.source.y))return null;
  const rectangles=nodeRectangles(l.target);
  const start=curvePoint(l,0),middle=curvePoint(l,.5),end=curvePoint(l,1);
  const a={x:2*(start.x+end.x-2*middle.x),y:2*(start.y+end.y-2*middle.y)};
  const b={x:end.x-start.x-a.x,y:end.y-start.y-a.y};
  const crossings=[];
  function intersect(axis,boundary,min,max){
    const c=start[axis]-boundary,other=axis==='x'?'y':'x';
    let roots=[];
    if(Math.abs(a[axis])<1e-10){if(Math.abs(b[axis])>1e-10)roots=[-c/b[axis]];}
    else{
      const discriminant=b[axis]*b[axis]-4*a[axis]*c;
      if(discriminant>=0){const d=Math.sqrt(discriminant);roots=[(-b[axis]-d)/(2*a[axis]),(-b[axis]+d)/(2*a[axis])];}
    }
    for(const t of roots)if(t>0&&t<=1){
      const point=curvePoint(l,t);
      if(point[other]>=min-1e-7&&point[other]<=max+1e-7)crossings.push(t);
    }
  }
  for(const [x0,y0,x1,y1] of rectangles){
    intersect('x',x0,y0,y1);intersect('x',x1,y0,y1);
    intersect('y',y0,x0,x1);intersect('y',y1,x0,x1);
  }
  if(!crossings.length)return null;
  const entry=Math.min(...crossings);
  const velocity={x:2*a.x*entry+b.x,y:2*a.y*entry+b.y};
  const t=Math.max(0,entry-gap/Math.max(1,Math.hypot(velocity.x,velocity.y)));
  const tip=curvePoint(l,t),angle=Math.atan2(2*a.y*t+b.y,2*a.x*t+b.x);
  return {x:tip.x,y:tip.y,angle};
}
function arrowFits(l, arrow, size, margin) {
  // A circle enclosing the entire triangle and its outline keeps even the
  // wings clear of both cards, using the same package tab/body as clipping.
  const x=arrow.x-Math.cos(arrow.angle)*size*.65,y=arrow.y-Math.sin(arrow.angle)*size*.65;
  const radius=size*.7+margin;
  return [l.source,l.target].every(node=>nodeRectangles(node).every(([left,top,right,bottom])=>{
    const dx=Math.max(0,left-x,x-right),dy=Math.max(0,top-y,y-bottom);
    return Math.hypot(dx,dy)>=radius;
  }));
}
// F17 width scale: more import references of the SAME direction ⇒ thicker
// line. Always the F12 total (edgeWeight), never the capped evidence
// sample; the log2 scale is stable and bounded at 8 px so the layout stays
// readable. force-graph divides linkWidth by globalScale at draw time, so
// the mapping survives zoom unchanged; filtering never rescales it either.
function widthFor(count) {
  return Math.min(8, 1.5 + Math.log2(Math.max(1, count)));
}
// F17 routing: one constant positive curvature for every link. force-graph
// 1.52 places the control point at midpoint + dist·k·perp(θ); reversing
// the endpoints flips θ by π, so A→B and B→A (kept as SEPARATE directed
// links by the aggregate mirror) become two mirrored curved paths —
// deterministic, no per-link state, never a merged or invented edge.
const CURVATURE = 0.25;
const ARROW_GAP = 3; // screen px; converted to graph units by the painter
function isCyclic(l) { return cyclicKeys.has(l.source.id + ' ' + l.target.id); }
function linkState(l) {
  const k = l.source.id + ' ' + l.target.id;
  if (resolvedKeys.has(k)) return 'resolved';
  if (introducedKeys.has(k)) return 'introduced';
  if (cyclicKeys.has(k)) return 'cyclic';
  return null;
}
function packageCycleContextHelp() { return t('Package-level cycle context; not cyclic in the current detail scope.','Kontext eines Paketzyklus; im aktuellen Detailumfang nicht zyklisch.'); }
function isPackageContextOnly(l) {
  return !linkState(l)&&!!packageContext?.edges.has(l.source.id+' '+l.target.id);
}
function linkStrokeColor(l,side) {
  // Tree highlights only right-hand cycle edges. Network has no side axis.
  // Coarser package context remains visible without inventing detail cycles.
  const state=linkState(l);
  if(state==='resolved')return C.resolved;
  if(side!=='left'){
    if(state==='introduced')return C.introduced;
    if(state==='cyclic')return C.cycle;
  }
  // Direction is relative to a node or opened package, including the retained
  // reference while reading edge evidence. Unrelated acyclic links stay neutral.
  if(typeof patternEdgeMarked==='function'&&patternEdgeMarked(l))return C.node;
  const reference=directionReference();
  if(reference){
    const from=reference.ids.has(l.source.id),to=reference.ids.has(l.target.id);
    if(to&&!from)return C.incoming;
    if(from&&!to)return C.outgoing;
  }
  return C.edge;
}
function linkArrowColor(l,side) {
  // Keep arrowheads and the full line in the same cycle/status/direction color.
  return linkStrokeColor(l,side);
}

function drawNode(n, ctx, scale) {
  if (!hasPosition(n)) return;
  const box=nodeBox(n), selected=selectedId===n.id,isPackage=n.kind==='module';
  const review=reviewBySource.get(n.src),badge=ui.highlightPriorities&&review?.priority!=='low'&&review;
  const left=n.x-box.w/2,top=n.y-box.h/2;
  ctx.globalAlpha=n.dimmed ? 0.2 : 1;
  nodeOutline(n,ctx);
  ctx.fillStyle=isPackage?(selected?C.packageSelected:C.packageFill):C.panel;ctx.fill();
  if(isPackage){
    ctx.fillStyle=C.packageTab;
    ctx.fillRect(left+.5,top+.5,box.tabWidth-1,box.tabHeight-.5);
  }
  ctx.strokeStyle=selected?C.node:n.inCycle?C.cycle:isPackage?C.packageBorder:C.border;
  ctx.lineWidth=(selected?2.5:1.2)/scale;ctx.stroke();
  if(typeof patternNodeMarked==='function'&&patternNodeMarked(n)){ctx.strokeStyle=C.node;ctx.lineWidth=3/scale;ctx.stroke();}
  const fontSize=Math.max(isPackage?14:12,11/Math.max(.2,scale));
  ctx.font=(isPackage?'600 ':'')+fontSize+'px -apple-system,system-ui,sans-serif';
  ctx.textAlign=isPackage?'left':'center';ctx.textBaseline='middle';ctx.fillStyle=C.label;
  const label=n.shortLabel||n.name;
  // Keep names painted at overview zoom, instead of reducing packages to dots.
  ctx.fillText(label.length>28?label.slice(0,26)+'…':label,isPackage?left+12:n.x-(badge?14:0),isPackage?top+box.tabHeight+21:n.y,box.w-24-(badge?32:0));
  if(badge){
    const x=left+box.w-30,y=isPackage?top+box.tabHeight+13:n.y-8;
    ctx.fillStyle=C.priorityFill;ctx.fillRect(x,y,24,16);
    ctx.fillStyle=C.priorityText;ctx.textAlign='center';ctx.font='600 10px system-ui';
    ctx.fillText(review.priority==='high'?'P1':'P2',x+12,y+8,22);
  }
  if(isPackage&&scale>=.5){
    ctx.textAlign='left';ctx.font='600 9px system-ui';ctx.fillStyle=C.label;
    ctx.fillText(t('PACKAGE','PAKET'),left+8,top+box.tabHeight/2,box.tabWidth-16);
    const files=n.fileCount||0,packages=n.packageCount||0;
    const summary=files+' '+(files===1?t('file','Datei'):t('files','Dateien'))+(packages?' · '+packages+' '+(packages===1?t('package','Paket'):t('packages','Pakete')):'')+(n.inCycle?t(' · CYCLE',' · ZYKLUS'):'');
    ctx.beginPath();ctx.moveTo(left+12,top+box.tabHeight+39);ctx.lineTo(left+box.w-12,top+box.tabHeight+39);
    ctx.strokeStyle=C.packageBorder;ctx.lineWidth=.65/scale;ctx.stroke();
    ctx.font=Math.max(10,9/scale)+'px system-ui';ctx.fillStyle=n.inCycle?C.cycle:C.muted;
    ctx.fillText(summary,left+12,top+box.tabHeight+54,box.w-24);
  }
  ctx.globalAlpha=1;
}

// Substituted from the pinned Rust constant when exporting the report. The
// default SVG tree never waits for an external script or creates a canvas.
const GRAPH_LIBRARY_URL='__CDN__';
let fg=null,networkLibraryState='idle',networkLibraryRequest=null;
function initializeNetworkGraph() {
  if(fg||isRepositoryOverview()||typeof ForceGraph!=='function')return;
  fg=ForceGraph()(document.getElementById('graph'))
  .enableNodeDrag(false)
  .minZoom(0.000001)
  .onRenderFramePre(drawContainment)
  .nodeLabel(n => tooltipEl([n.name
    + (n.inCycle ? t(' (in cycle)',' (im Zyklus)') : '')
    + (n.kind === 'module' ? t(' [module]',' [Modul]') : '')]
    .concat(reviewBySource.has(n.src)?[reviewPriorityText(reviewBySource.get(n.src)),...reviewBySource.get(n.src).reasons.map(reviewReasonText)]:[])
    .concat(n.kind==='module'&&childrenOf(work,n.src).length?[t('Double-click to open package contents.','Doppelklick öffnet den Paketinhalt.')]:[])))
  .nodeCanvasObject(drawNode)
  .nodePointerAreaPaint((n, color, ctx) => {
    if (!hasPosition(n)) return;
    nodeOutline(n,ctx);
    ctx.fillStyle = color;
    ctx.fill();
  })
  .linkColor(linkStrokeColor)
  .linkVisibility(dependencyVisible)
  .linkLineDash(l => isPackageContextOnly(l) ? [6,4] : null)
  // F17: width carries dependency strength (total import references of
  // THIS direction). A selected node gets incoming/outgoing line colors;
  // arrowhead colors keep the cycle/simulation state visible.
  .linkWidth(l => widthFor(edgeWeight(l)))
  // The library paints/hit-tests the curve; our after-painter clips the
  // arrowhead against the rectangular node rather than a circle radius.
  .linkCurvature(() => CURVATURE)
  .linkDirectionalArrowLength(0) // arrowGeometry clips heads against the actual rounded-node bounds
  .linkDirectionalArrowRelPos(1)
  .nodeRelSize(4)
  .nodeVal(n => Math.pow((radiusOf(n) + ARROW_GAP) / 4, 2))
  .linkLabel(l => tooltipEl([l.source.name + ' → ' + l.target.name + ': '
    + weightLabel(l) + ' '+graphEvidenceUnit()]
    .concat(isPackageContextOnly(l)?[packageCycleContextHelp()]:[])
    .concat(l.refs.slice(0, 10)
      .map(r => r.file + ':' + r.line + '  ' + r.raw))))
  .onNodeClick(n => selectNode(n.name))
  // F17 selection: clicking any link selects it — per DIRECTION, since
  // A→B and B→A are separate links with separate pointer areas — and its
  // exact total is painted beside the path (linkCanvasObject below). For
  // Simulation exposes a separate confirmed cut action in the Inspector.
  .onLinkClick(l => selectEdge({from:l.source.id,to:l.target.id}))
  .onBackgroundClick(() => {ui.selection=null;reconcileSelection();renderSidebar();refreshSelection();})
  .linkCanvasObjectMode(() => 'after')
  .linkCanvasObject((l,ctx,scale) => {
    if(!dependencyVisible(l))return;
    // graphData schedules endpoint resolution in force-graph. An animation
    // frame may arrive first, while source/target are IDs or lack positions.
    if (!hasPosition(l.source) || !hasPosition(l.target) || !Number.isFinite(scale) || scale<=0) return;
    const pixels=Math.max(10,widthFor(edgeWeight(l))+6),size=pixels/scale,markers=[];
    let endpoint=null;
    // Keep the analytic contour intersection; increase the screen-space gap
    // only when a larger arrow's wing would be hidden behind either card.
    const maxGap=Math.max(ARROW_GAP+pixels*2,Math.hypot(l.target.x-l.source.x,l.target.y-l.source.y)*scale*.35);
    for(let gap=ARROW_GAP;gap<=maxGap;gap+=Math.max(2,gap*.25)){
      const arrow=arrowGeometry(l,gap/scale);
      if(!arrow)break;
      if(arrowFits(l,arrow,size,1.5/scale)){endpoint=arrow;markers.push(arrow);break;}
    }
    const middle={...curvePoint(l,.5),angle:Math.atan2(l.target.y-l.source.y,l.target.x-l.source.x)};
    if(arrowFits(l,middle,size,1.5/scale)&&(!endpoint||Math.hypot(middle.x-endpoint.x,middle.y-endpoint.y)>=size*2))markers.push(middle);
    for(const arrow of markers){
      ctx.save();ctx.translate(arrow.x,arrow.y);ctx.rotate(arrow.angle);
      ctx.beginPath();ctx.moveTo(0,0);ctx.lineTo(-size,-size*.6);ctx.lineTo(-size,size*.6);ctx.closePath();
      ctx.lineJoin='round';ctx.lineWidth=2/scale;ctx.strokeStyle=C.panel;ctx.stroke();
      ctx.fillStyle=linkArrowColor(l);ctx.fill();ctx.restore();
    }
    if(l.source.id+' '+l.target.id===selectedLinkKey){
      const p=curvePoint(l,.5);ctx.font=(12/scale)+'px system-ui';ctx.textAlign='center';ctx.textBaseline='bottom';
      ctx.fillStyle=C.label;ctx.fillText(weightLabel(l),p.x,p.y-size*1.2-4/scale);
    }
  });

// double-click = zoom in: replace the node by its children (the
// Cut::expand mirror), recorded on the breadcrumb trail. force-graph 1.52
// has no onNodeDoubleClick accessor, so dblclick is wired on the canvas
// directly (the library disables d3-zoom's own dblclick.zoom there).
if (fg) {
  const canvas=document.getElementById('graph').querySelector('canvas');
  let lastPointer=null;
  // A pan drag must not drill into whatever the pointer happens to cross.
  const trackPointer=ev=>{lastPointer=ev.buttons?null:{x:ev.offsetX,y:ev.offsetY};};
  canvas.addEventListener('pointermove',hoverContextTooltip);
  canvas.addEventListener('pointermove',trackPointer);
  canvas.addEventListener('pointerleave',()=>{hideContextTooltip();lastPointer=null;});
  canvas.addEventListener('pointerdown',hideContextTooltip);
  // Reading a package at deep zoom continues the exploration: crossing the
  // threshold opens it like a double-click, and the fit then frames its contents.
  const DRILL_ZOOM=8;let drillArmed=true;
  fg.onZoom(()=>{hideContextTooltip();
    if(fg.zoom()<DRILL_ZOOM){drillArmed=true;return;}
    if(!drillArmed||!lastPointer)return;
    const point=fg.screen2GraphCoords(lastPointer.x,lastPointer.y);
    const node=renderedNodes.find(n=>n.kind==='module'&&hitNode(n,point.x,point.y));
    if(node&&childrenOf(work,node.src).length){drillArmed=false;expandNode(node.name);}
  });
  canvas.addEventListener('dblclick', ev => {
    const point=fg.screen2GraphCoords(ev.offsetX,ev.offsetY);
    const node=renderedNodes.find(n=>hitNode(n,point.x,point.y));
    if(node && node.kind==='module') {ev.preventDefault();expandNode(node.name);}
  });
  fg.cooldownTicks(1);
  for(const force of ['charge','link','center'])fg.d3Force(force,null);
  // The library may set an automatic zoom during its deferred data update.
  // Finish a requested fit on the first subsequent tick, after that update.
  fg.onEngineTick(()=>{layoutReady=true;if(pendingGraphZoom!==null){fg.zoom(pendingGraphZoom,0);pendingGraphZoom=null;}fitWhenReady();});
  fg.onEngineStop(fitWhenReady);
}
}
// Embedded/preloaded copies remain usable without a network request.
if(typeof ForceGraph==='function')initializeNetworkGraph();

function renderNetworkState() {
  const state=document.getElementById('graph-state');
  const message=networkLibraryState==='loading'?t('Loading network view… Tree and Dependencies remain available.','Netzwerkansicht wird geladen… Baum und Abhängigkeiten bleiben verfügbar.'):
    networkLibraryState==='failed'?t('Network library could not load. Tree, Dependencies, Explorer and Matrix remain available.','Netzwerkbibliothek konnte nicht geladen werden. Baum, Abhängigkeiten, Explorer und Matrix bleiben verfügbar.'):
    fg&&!view.nodes.length?t('No visible nodes. Reset the view or check the source configuration.','Keine sichtbaren Knoten. Ansicht zurücksetzen oder Quellkonfiguration prüfen.'):'';
  state.textContent=message;
  if(networkLibraryState==='failed')state.appendChild(button(t('Retry network load','Erneut laden'),requestNetworkLibrary));
  state.hidden=ui.workspace!=='graph'||ui.diagram!=='network'||!message;
}
function requestNetworkLibrary() {
  if(fg||networkLibraryRequest||isRepositoryOverview())return;
  if(typeof ForceGraph==='function'){
    try{initializeNetworkGraph();networkLibraryState='ready';}catch(_error){fg=null;networkLibraryState='failed';}
    renderNetworkState();return;
  }
  networkLibraryState='loading';
  const script=document.createElement('script'),request={script,timer:null};
  networkLibraryRequest=request;script.async=true;script.src=GRAPH_LIBRARY_URL;
  const finish=failed=>{
    if(networkLibraryRequest!==request)return;
    networkLibraryRequest=null;clearTimeout(request.timer);script.remove();
    try{
      if(failed||typeof ForceGraph!=='function')throw Error('Network library unavailable');
      initializeNetworkGraph();networkLibraryState='ready';
    }catch(_error){fg=null;networkLibraryState='failed';}
    // A late response must neither change the workspace nor replay an old
    // scope/selection. Render the current model only if Network is still open.
    if(ui.workspace==='graph'&&ui.diagram==='network'){renderAll(false);setWidth();fitView();}
    else renderNetworkState();
  };
  script.onload=()=>finish(false);script.onerror=()=>finish(true);
  request.timer=setTimeout(()=>finish(true),15000);
  document.head.appendChild(script);renderNetworkState();
}

function recordOp(op, repair=null) {
  if(isRepositoryOverview())return;
  if (!ui.simulation) return;
  const additions=Array.isArray(op)?op:[op];
  if(!additions.length)return;
  try {
    // Validate the FULL sequence like apply_ops (all-or-nothing): the
    // candidate must survive replay on the pristine baseline plus every
    // already-recorded op, so replayOps can never throw afterwards.
    const probe = baseline();
    for (const o of ops) applyOp(probe, o);
    for(const addition of additions)applyOp(probe, addition);
  } catch (err) {
    feedback(String(err.message));
    return;
  }
  ops.push(...additions);opBatches.push(additions.length);appliedRepair=repair;
  // Drop the cycle isolation: re-aggregation renumbers the dense view
  // ids, so the old isolation set could highlight the wrong nodes.
  isolated = null; ui.cycle = null;
  recompute();
  renderAll();
  document.getElementById('sidebar').focus();
}

function currentFilter() {
  return document.getElementById('filter').value.trim().toLowerCase();
}

function pinSelectedGraphFiles() {
  const names=ui.selection?.kind==='node'?[ui.selection.name]:ui.selection?.kind==='edge'?[ui.selection.from,ui.selection.to]:[];
  const ids=new Set(view.nodes.filter(n=>names.includes(n.name)).map(n=>n.id)),reference=directionReference()?.ids;
  // A package's incoming/outgoing edges can terminate at direct files hidden
  // by the mixed package/file display. Reveal those endpoints with the selection.
  if(reference)for(const edge of view.edges)if(reference.has(edge.from)||reference.has(edge.to)){ids.add(edge.from);ids.add(edge.to);}
  for(const n of view.nodes)if(n.kind!=='module'&&ids.has(n.id))revealedGraphFiles.add(n.src);
}
function exitCycleForSelection() {
  if(!isolated||!ui.cycle)return;
  const edge=selectedEdge(),ids=ui.selection?.kind==='node'?[...selectedNodeIds]:edge?[edge.from,edge.to]:[];
  if(ids.some(id=>id!==null&&!isolated.has(id))){ui.cycle=null;isolated=null;renderCyclesPanel();}
}
function directionReference() {
  const name=ui.selection?.kind==='node'?ui.selection.name:ui.selection?.kind==='edge'?ui.selection.reference:!ui.selection&&!ui.cycle?packageContext?.name:null;
  if(typeof name!=='string')return null;
  const cached=directionReference.cache;
  if(cached?.view===view&&cached.work===work&&cached.name===name)return cached.value;
  const node=view.nodes.find(n=>n.name===name);
  // A retained package reference is projected again after expansion or a
  // simulated move. Cache only this immutable view/work pair, since every
  // painted edge asks for the same reference and subtree traversal is costly.
  let ids=node?new Set([node.id]):null;
  if(!node){
    const source=nodeByName(work,name);
    if(source?.kind==='module'){
      const descendants=subtree(work,source.id);
      ids=new Set(view.nodes.filter(n=>descendants.has(n.src)).map(n=>n.id));
    }
  }
  const value=ids?.size?{name,ids}:null;
  directionReference.cache={view,work,name,value};
  return value;
}
function edgeReference() {
  const reference=directionReference();if(reference)return reference;
  if(ui.selection?.kind==='edge'&&selectedLinkKey)return {name:ui.selection.from+' ↔ '+ui.selection.to,ids:new Set(selectedLinkKey.split(' ').map(Number))};
  if(ui.cycle&&isolated)return {name:t('Focused cycle','Fokussierter Zyklus'),ids:isolated};
  return null;
}
function dependencyMatchesSelection(from,to,mode=ui.edgeMode) {
  // Keep the projected package-cycle evidence without reclassifying it as a
  // finer-grained SCC. This shared predicate also drives counts and hints.
  if(ui.cyclesOnly&&!cyclicKeys.has(from+' '+to)&&!packageContext?.edges.has(from+' '+to))return false;
  if(mode==='all')return true;
  if(ui.selection?.kind==='cycle')return !!isolated&&isolated.has(from)&&isolated.has(to);
  if(!ui.cycle&&mode==='both'&&packageContext?.edges.has(from+' '+to))return true;
  const selected=edgeReference()?.ids;
  // A direction may be chosen before its reference; keep context visible until selection.
  if(!selected)return true;
  const fromSelected=selected.has(from),toSelected=selected.has(to);
  // An expanded package is represented by several nodes. Connections between
  // those nodes are internal, not incoming/outgoing across the package boundary.
  // Edge-only inspection retains its endpoint-pair fallback until a node is selected.
  if(directionReference()){
    if(mode==='incoming')return toSelected&&!fromSelected;
    if(mode==='outgoing')return fromSelected&&!toSelected;
  }
  return (mode!=='incoming'&&fromSelected)||(mode!=='outgoing'&&toSelected);
}
function dependencyVisible(link) {
  return dependencyMatchesSelection(typeof link.source==='object'?link.source.id:link.source,typeof link.target==='object'?link.target.id:link.target);
}
function displayedGraphNodes() {
  const candidates=view.nodes.filter(n=>isolated===null||isolated.has(n.id)),packages=candidates.some(n=>n.kind==='module');
  return candidates.filter(n=>ui.showFiles||!!ui.cycle||!packages||n.kind==='module'||revealedGraphFiles.has(n.src)||(!ui.cycle&&packageContext?.nodes.has(n.id)));
}
function renderGraphPresentation() {
  const network=ui.diagram==='network';
  for(const id of ['graph','edge-legend','graph-controls'])document.getElementById(id).hidden=!network;
  document.getElementById('dependency-browser').hidden=ui.diagram!=='dependencies';
  document.getElementById('package-tree').hidden=ui.diagram!=='overview';
  document.getElementById('graph-state').hidden=!network||!document.getElementById('graph-state').textContent;
  for(const mode of ['overview','dependencies','network'])document.getElementById('diagram-'+mode).setAttribute('aria-pressed',String(ui.diagram===mode));
  document.getElementById('edge-modes').hidden=false;
  for(const id of ['edge-reference','graph-visibility','graph-hint'])document.getElementById(id).hidden=false;
  if(ui.workspace!=='graph')return;
  renderWorkspaceDescription();
  if(ui.diagram==='overview')renderPackageTree();
  else if(ui.diagram==='dependencies')renderDependencyBrowser();
}
function setDiagram(mode) {
  ui.diagram=mode;
  if(mode==='network')requestNetworkLibrary();
  if(!edgeReference()&&ui.edgeModeAutomatic)ui.edgeMode='all';
  hideContextTooltip();renderAll(false);setWidth();
  if(mode==='network')fitView();
}
function renderGraphControls() {
  buildLegend();
  document.getElementById('toggle-cycles').setAttribute('aria-pressed',String(ui.cyclesOnly));
  document.getElementById('toggle-priorities').setAttribute('aria-pressed',String(ui.highlightPriorities));
  const candidates=view.nodes.filter(n=>isolated===null||isolated.has(n.id)),scopeIds=new Set(candidates.map(n=>n.id)),shown=new Set(displayedGraphNodes().map(n=>n.id));
  const displayedEdges=view.edges.filter(e=>shown.has(e.from)&&shown.has(e.to));
  const files=candidates.filter(n=>n.kind!=='module'),hiddenFiles=files.filter(n=>!shown.has(n.id)).length;
  const packages=candidates.some(n=>n.kind==='module'),toggle=document.getElementById('toggle-files');
  toggle.disabled=!packages||!files.length||!!ui.cycle;toggle.setAttribute('aria-pressed',String(ui.showFiles||!packages||!!ui.cycle));
  toggle.textContent=ui.cycle?t('Cycle files shown','Zyklusdateien angezeigt'):!packages?t('Files shown','Dateien angezeigt'):ui.showFiles?t('Hide files','Dateien ausblenden'):t('Show files ({count} hidden)','Dateien anzeigen ({count} ausgeblendet)',{count:hiddenFiles});
  toggle.title=t('Display filter only. Hide files clears previously revealed files; selected dependencies and files needed for external cycle context stay visible. Reset view clears revealed files.','Reiner Anzeigefilter. „Dateien ausblenden“ blendet zuvor eingeblendete Dateien wieder aus; Dateien der ausgewählten Beziehungen und für externe Zyklen benötigte Dateien bleiben sichtbar. „Ansicht zurücksetzen“ setzt eingeblendete Dateien zurück.');
  const reference=edgeReference();
  document.getElementById('edge-reference').textContent=reference?t('Reference: {name}','Bezug: {name}',{name:reference.name})+(ui.selection?.kind==='node'&&selectedId===null?t(' (package contents)',' (Paketinhalt)'):''):ui.edgeMode!=='all'?t('Select a package or file to apply the chosen direction.','Paket oder Datei auswählen, um die gewählte Richtung anzuwenden.'):t('Select a package or file for incoming/outgoing dependencies.','Für eingehende/ausgehende Beziehungen ein Paket oder eine Datei auswählen.');
  document.getElementById('edge-reference').title=document.getElementById('edge-reference').textContent;
  const diagramLabel={overview:t('Tree','Baum'),dependencies:t('Dependencies','Abhängigkeiten'),network:t('Network','Netzwerk')}[ui.diagram];
  document.getElementById('graph-display-summary').textContent=t('Display: {name}','Anzeige: {name}',{name:diagramLabel});
  const modeLabels={incoming:t('Incoming','Eingehend'),outgoing:t('Outgoing','Ausgehend'),both:t('Both','Beide'),all:t('All','Alle')};
  for(const b of document.querySelectorAll('#edge-modes button')){
    const mode=b.dataset.edges,count=mode!=='all'&&!reference?'—':displayedEdges.filter(e=>dependencyMatchesSelection(e.from,e.to,mode)).length;
    b.setAttribute('aria-pressed',String(mode===ui.edgeMode));
    b.disabled=false;
    b.textContent=modeLabels[mode]+' ('+count+')';
    b.title=(mode==='all'?t('All connections between displayed nodes, including unrelated connections.','Alle Verbindungen zwischen angezeigten Knoten, auch ohne Bezug zur Auswahl.'):!reference?t('Choose this direction, then select a package or file. Until then, connections remain visible.','Richtung wählen, dann ein Paket oder eine Datei auswählen. Bis dahin bleiben die Verbindungen sichtbar.'):(mode==='incoming'?t('External connections into {name}','Verbindungen von außerhalb nach {name}',{name:reference.name}):mode==='outgoing'?t('Connections from {name} to outside','Verbindungen von {name} nach außerhalb',{name:reference.name}):t('Connections in both directions and inside {name}','Verbindungen in beide Richtungen und innerhalb von {name}',{name:reference.name}))+(mode==='both'&&packageContext&&!ui.cycle?t('; includes external cycle context.','; einschließlich externem Zykluskontext.'):'.'))+(ui.cyclesOnly?t(' Limited to cycles and package-level cycle context.',' Auf Zyklen und Zykluskontext auf Paketebene begrenzt.'):'');
    b.setAttribute('aria-label',b.textContent+'. '+b.title);
  }
  const visibleEdges=displayedEdges.filter(e=>dependencyMatchesSelection(e.from,e.to)).length;
  document.getElementById('graph-status-brief').textContent=t('{shownNodes}/{nodes} nodes · {shownEdges}/{edges} dependencies','{shownNodes}/{nodes} Knoten · {shownEdges}/{edges} Abhängigkeiten',{shownNodes:shown.size,nodes:view.nodes.length,shownEdges:visibleEdges,edges:view.edges.length});
  const emptyFeedback=document.getElementById('graph-filter-feedback');
  emptyFeedback.hidden=!view.nodes.length||!!visibleEdges;
  emptyFeedback.textContent=!view.edges.length?t('No dependencies in this scope. Open a package to inspect its files.','Keine Abhängigkeiten in diesem Ausschnitt. Ein Paket öffnen, um seine Dateien zu untersuchen.'):t('No matching dependencies. Check the direction, cycle and file filters.','Keine passenden Abhängigkeiten. Richtungs-, Zyklus- und Dateifilter prüfen.');
  const hiddenRelated=view.edges.filter(e=>scopeIds.has(e.from)&&scopeIds.has(e.to)&&dependencyMatchesSelection(e.from,e.to)&&(!shown.has(e.from)||!shown.has(e.to))).length;
  document.getElementById('graph-visibility').textContent=t('{shownNodes} / {nodes} nodes shown · {shownEdges} / {edges} dependencies shown in analysis scope · {hiddenNodes} nodes and {hiddenEdges} dependencies hidden','{shownNodes} / {nodes} Knoten angezeigt · {shownEdges} / {edges} Abhängigkeiten im Analysebereich angezeigt · {hiddenNodes} Knoten und {hiddenEdges} Abhängigkeiten ausgeblendet',{shownNodes:shown.size,nodes:view.nodes.length,shownEdges:visibleEdges,edges:view.edges.length,hiddenNodes:view.nodes.length-shown.size,hiddenEdges:view.edges.length-visibleEdges});
  let hint='';
  if(!view.edges.length)hint=t('No dependencies in this analysis scope. Expand a package to inspect its files.','Keine Abhängigkeiten in diesem Analysebereich. Ein Paket aufklappen, um seine Dateien zu untersuchen.');
  else if(ui.edgeMode!=='all'&&!ui.selection&&!packageContext)hint=t('Select a package or file to show its dependencies, or choose All.','Ein Paket oder eine Datei auswählen, um die Abhängigkeiten anzuzeigen, oder „Alle“ wählen.');
  else if(hiddenRelated)hint=hiddenRelated===1?t('1 related dependency is hidden by the file filter. Choose Show files to include their endpoints.','1 zugehörige Abhängigkeit wird durch den Dateifilter ausgeblendet. „Dateien anzeigen“ wählen, um ihre Endpunkte einzubeziehen.'):t('{count} related dependencies are hidden by the file filter. Choose Show files to include their endpoints.','{count} zugehörige Abhängigkeiten werden durch den Dateifilter ausgeblendet. „Dateien anzeigen“ wählen, um ihre Endpunkte einzubeziehen.',{count:hiddenRelated});
  else if(!visibleEdges){
    const direction={both:t('matching','passenden'),incoming:t('incoming','eingehenden'),outgoing:t('outgoing','ausgehenden'),all:t('all','')};
    hint=ui.cyclesOnly?t('No cycle dependencies match the current direction and file filters.','Keine Zyklusabhängigkeiten passen zu den aktuellen Richtungs- und Dateifiltern.'):ui.edgeMode==='all'?t('No all dependencies for this selection in the shown scope.','Keine Abhängigkeiten für diese Auswahl im angezeigten Ausschnitt.'):t('No {direction} dependencies for this selection in the shown scope.','Keine {direction} Abhängigkeiten für diese Auswahl im angezeigten Ausschnitt.',{direction:direction[ui.edgeMode]});
  }
  if(packageContext&&!ui.cycle){
    const context=t('Inside {name}. ','Innerhalb von {name}. ',{name:packageContext.name})+(packageContext.edges.size?t('Both keeps external cycle context visible. ','„Beide“ hält den externen Zykluskontext sichtbar. '):'')+(packageContext.levels.length?t('Package-level cycle context is included without changing cycles in the current detail scope. ','Zykluskontext auf Paketebene wird einbezogen, ohne Zyklen im aktuellen Detailausschnitt zu verändern. '):'');
    hint=context+hint;
  }
  document.getElementById('graph-hint').textContent=(ui.cyclesOnly?t('Cycles only: current-scope cycles and package-level cycle context. ','Nur Zyklen: Zyklen im aktuellen Ausschnitt und Zykluskontext auf Paketebene. '):'')+hint;
  document.getElementById('graph-scope-details').hidden=false;
  renderGraphPresentation();
}
function renderGraph(fitOnLayoutChange=true) {
  if(isRepositoryOverview())return;
  hideContextTooltip();
  if (!fg) {renderGraphControls();return;}
  const q=currentFilter();
  const chosen=displayedGraphNodes();
  const keep=new Set(chosen.map(n=>n.id));
  const edges=view.edges.filter(e=>keep.has(e.from)&&keep.has(e.to));
  graphLayout=containmentLayout(work,chosen.map(n=>n.src),rootName);
  const layoutSignature=JSON.stringify(graphLayout.nodes.map(n=>[n.src,n.name,n.x,n.y]));
  const layoutChanged=layoutSignature!==treeSignature;treeSignature=layoutSignature;
  const signature=JSON.stringify([layoutSignature,chosen.map(n=>[n.src,n.id,n.name]),edges.map(e=>[e.from,e.to,e.count,e.refs.length])]);
  const byId=new Map(work.nodes.map(n=>[n.id,n]));
  const counts=new Map(work.nodes.filter(n=>n.kind==='module').map(n=>[n.id,{files:0,packages:0}]));
  for(const node of work.nodes){
    if(node.kind==='module'&&counts.has(node.parent))counts.get(node.parent).packages++;
    if(node.kind==='file'){
      const seen=new Set();let parent=node.parent;
      while(parent!=null&&!seen.has(parent)){
        seen.add(parent);if(counts.has(parent))counts.get(parent).files++;
        parent=byId.get(parent)?.parent;
      }
    }
  }
  for(const n of graphLayout.nodes){n.fileCount=counts.get(n.src)?.files||0;n.packageCount=counts.get(n.src)?.packages||0;}
  if(signature===graphSignature){
    for(const n of renderedNodes){n.dimmed=q!==''&&!n.name.toLowerCase().includes(q);n.fileCount=counts.get(n.src)?.files||0;n.packageCount=counts.get(n.src)?.packages||0;n.inCycle=sccs.some(c=>c.members.includes(n.id));}
    refreshSelection();return;
  }
  graphSignature=signature;
  const basenames=new Map();for(const n of chosen){const base=n.name.split('/').pop();basenames.set(base,(basenames.get(base)||0)+1);}
  renderedNodes=chosen.map(n=>{
    const position=graphLayout.positions.get(n.src),node={...n,...position,fx:position.x,fy:position.y},base=n.name.split('/').pop();
    // A package is identified by its path; the bare basename alone is not a
    // name, especially when no cycles put the other path halves on screen.
    node.shortLabel=n.kind==='module'?n.name:(basenames.get(base)>1?n.name.split('/').slice(-2).join('/'):base);
    node.fileCount=counts.get(n.src)?.files||0;node.packageCount=counts.get(n.src)?.packages||0;
    node.inCycle=sccs.some(c=>c.members.includes(n.id));node.dimmed=q!==''&&!n.name.toLowerCase().includes(q);return node;
  });
  // Without d3's link force, force-graph does not resolve ID endpoints.
  const renderedById=new Map(renderedNodes.map(n=>[n.id,n]));
  renderedLinks=edges.map(e=>({source:renderedById.get(e.from),target:renderedById.get(e.to),refs:e.refs,count:e.count}));
  if(!fitOnLayoutChange){pendingFit=false;pendingGraphZoom=fg.zoom();}
  layoutReady=false;
  fg.graphData({nodes:renderedNodes,links:renderedLinks});
  if(layoutChanged&&fitOnLayoutChange)fitView();
  renderGraphControls();
}

function renderBreadcrumb() {
  const bar = document.getElementById('breadcrumbs');
  bar.textContent = '';
  if(isRepositoryOverview()){bar.appendChild(element('span',rootName));return;}
  // A navigation path follows containment, not the history of opened packages.
  // A selected child refines the location; an unrelated selection must not
  // replace the package most recently opened in the graph.
  let target=nodeById(work,treeLocation??trail[trail.length-1]);
  const selected=ui.selection?.kind==='node'?nodeByName(work,ui.selection.name):null;
  if(selected&&(!target||selected.id===target.id||ancestorsOf(work,selected.id).includes(target.id)))target=selected;
  const path=target?[...ancestorsOf(work,target.id).reverse(),target.id]:[];
  const root = button(rootName,() => {
    showTab('graph');resetView();
    bar.querySelector('button')?.focus();
  });
  root.title = t('Restore the original report scope and depth','Ursprünglichen Berichtsumfang und Tiefe wiederherstellen');
  root.setAttribute('aria-label',rootName+' · '+root.title);
  if(!path.length)root.setAttribute('aria-current','location');
  bar.appendChild(root);
  path.forEach((id, i) => {
    const n = nodeById(work, id);
    if (!n) return;
    const separator=element('span',' › ');separator.setAttribute('aria-hidden','true');bar.appendChild(separator);
    const b = button(n.name.slice(n.name.lastIndexOf('/')+1),() => {
      for(const descendant of path.slice(i+1))explorerOpen.delete(descendant);
      navigateExplorerNode(id);
      [...bar.querySelectorAll('button[data-node]')].find(button=>button.dataset.node===String(id))?.focus();
    });
    b.title=n.name;b.setAttribute('aria-label',n.name);b.setAttribute('data-node',String(id));
    if(i===path.length-1)b.setAttribute('aria-current','location');
    bar.appendChild(b);
  });
}

// fan_in / fan_out / instability — mirror of archguard-core ModuleMetrics
// (lib.rs:192-198): I = fan_out/(fan_in+fan_out), 0.0 without edges.
function viewMetrics() {
  const fanIn = new Map();
  const fanOut = new Map();
  for (const e of view.edges) {
    fanOut.set(e.from, (fanOut.get(e.from) || 0) + 1);
    fanIn.set(e.to, (fanIn.get(e.to) || 0) + 1);
  }
  return view.nodes.map(n => {
    const fi = fanIn.get(n.id) || 0;
    const fo = fanOut.get(n.id) || 0;
    return { node: n, fanIn: fi, fanOut: fo,
             instability: fi + fo === 0 ? 0 : fo / (fi + fo) };
  });
}

function esc(s) {
  return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;').replace(/'/g, '&#39;');
}

// F01: force-graph inserts a STRING label into its tooltip via innerHTML
// — node names, paths and raw imports are untrusted source-code content,
// so both label accessors hand the library a pre-built element instead
// (1.52 appends HTMLElement labels as-is): every line becomes a text node
// through createTextNode, no markup of the payload is ever parsed and no
// injected element or event handler can materialise.
function tooltipEl(lines) {
  const el = document.createElement('div');
  for (const line of lines) {
    if (el.childNodes.length) el.appendChild(document.createElement('br'));
    el.appendChild(document.createTextNode(String(line)));
  }
  return el;
}

function hideContextTooltip() {
  document.getElementById('graph-context-tooltip').hidden=true;
}

// Context headers are painted outside graphData, so they have no library hit
// target. Use the same laid-out bounds in graph coordinates for their labels.
function hoverContextTooltip(event) {
  hideContextTooltip();
  if(event.buttons||ui.workspace!=='graph'||ui.diagram!=='network')return;
  const point=fg.screen2GraphCoords(event.offsetX,event.offsetY);
  const node=graphLayout.nodes.find(n=>n.context&&Math.abs(point.x-n.x)<=n.w/2&&Math.abs(point.y-n.y)<=n.h/2);
  if(!node)return;
  const tip=document.getElementById('graph-context-tooltip');
  tip.textContent='';
  tip.appendChild(tooltipEl([
    node.project?(data.root||node.name):node.name,
    node.project?t('Project hierarchy','Projekthierarchie'):t('Package context · {files} files · {packages} direct packages','Paketkontext · {files} Dateien · {packages} direkte Pakete',{files:node.fileCount,packages:node.packageCount}),
    t('Tree lines show containment; curved arrows show dependencies.','Baumlinien zeigen die Zugehörigkeit; gebogene Pfeile zeigen Abhängigkeiten.')
  ]));
  tip.style.left='0px';tip.style.top='0px';tip.hidden=false;
  const area=document.getElementById('tab-graph').getBoundingClientRect(),box=tip.getBoundingClientRect();
  const left=event.offsetX+12+box.width<=area.width?event.offsetX+12:event.offsetX-box.width-12;
  const top=event.offsetY+12+box.height<=area.height?event.offsetY+12:event.offsetY-box.height-12;
  tip.style.left=Math.max(8,Math.min(left,area.width-box.width-8))+'px';
  tip.style.top=Math.max(8,Math.min(top,area.height-box.height-8))+'px';
}

function renderDependencyMetrics(container,incoming,outgoing){
    const fanIn=new Set(incoming.map(e=>e.from)).size,fanOut=new Set(outgoing.map(e=>e.to)).size;
    const metrics={fanIn,fanOut,instability:fanIn+fanOut?fanOut/(fanIn+fanOut):0};
    const metricLine=element('div',undefined,'inspector-metrics');metricLine.setAttribute('data-inspector-metrics','true');
    for(const [key,value] of [['incoming',metrics.fanIn],['outgoing',metrics.fanOut],['instability',metrics.instability.toFixed(2)]]){
      const card=element('div',undefined,'inspector-metric');card.appendChild(element('strong',key==='instability'?String(value):connectionCountLabel(value)));card.appendChild(element('span',{incoming:t('incoming','Eingehend'),outgoing:t('outgoing','Ausgehend'),instability:t('instability','Instabilität')}[key]));
      metricLine.appendChild(explainMetric(card,key));
    }
    container.appendChild(metricLine);
}
function renderSidebar() {
  enforceRepositoryScope();
  hideMetricTooltip(document.getElementById('sidebar'));
  if(ui.workspace==='services'){renderServiceEvidence();return;}
  if(ui.workspace==='calls'){renderCallInspector();return;}
  if(ui.workspace==='security'){renderSecurityInspector();return;}
  if(ui.workspace==='review-dashboard'&&typeof renderReviewDashboardSidebar==='function'){renderReviewDashboardSidebar();return;}
  const sb=document.getElementById('sidebar');sb.textContent='';
  if(['quality','architecture','assessment'].includes(ui.workspace))return;
  const sel=ui.selection;
  if(!sel){
    sb.appendChild(element('h2',packageContext?t("Inside ","Innerhalb von ")+packageContext.name:t('Details','Details')));
    if(packageContext)sb.appendChild(element('p',t("Both keeps external cycle paths visible, including sibling packages and parent-level dependencies. Select an edge to inspect its source evidence.","„Beide“ zeigt auch externe Zykluspfade, einschließlich benachbarter Pakete und Abhängigkeiten auf übergeordneten Ebenen. Wähle eine Kante, um ihre Quellbelege zu untersuchen."),'muted'));
    sb.appendChild(element('p',t('Select a node or connection.','Knoten oder Verbindung auswählen.'),'empty'));return;
  }
  if(sel.kind==='node'){
    const visibleNode=view.nodes.find(n=>n.name===sel.name),source=nodeByName(work,sel.name);
    const n=visibleNode||(source?{...source,src:source.id}:null);if(!n)return;
    sb.appendChild(element('h2',n.name));sb.appendChild(element('span',(n.kind==='module'?t('module','Modul'):n.kind==='file'?t('file','Datei'):n.kind)+(n.lang?' · '+n.lang:'')+(!visibleNode?t(" · expanded package contents"," · aufgeklappter Paketinhalt"):''),'badge'));
    const incoming=view.edges.filter(e=>selectedNodeIds.has(e.to)&&!selectedNodeIds.has(e.from)),outgoing=view.edges.filter(e=>selectedNodeIds.has(e.from)&&!selectedNodeIds.has(e.to));
    if(visibleNode)renderNodeDetailGraph(sb,n,incoming,outgoing);
    renderDependencyMetrics(sb,incoming,outgoing);
    const actions=element('div',undefined,'context-actions');
    actions.appendChild(button(t("Copy path","Pfad kopieren"),()=>copyText(n.name)));
    if((callModel.classes||[]).length)actions.appendChild(button(t("Inspect calls","Aufrufe untersuchen"),()=>openCallsForNode(n.name)));
    if(visibleNode&&childrenOf(work,n.src).length)actions.appendChild(button(t("Expand module","Modul aufklappen"),()=>expandNode(n.name),'btn btn-primary'));
    sb.appendChild(actions);
    if(n.kind==='module'&&moveSuggestions===null)moveSuggestions=packageMoveSuggestions(work);
    const suggestion=moveSuggestions?.get(n.src),advanced=element('details',undefined,'expert-simulation');
    advanced.appendChild(element('summary',t("Advanced: manual grouping","Erweitert: manuelle Gruppierung")));
    if(n.kind==='module'){
      advanced.appendChild(element('h3',t("Suggested package move","Vorgeschlagene Paketverschiebung")));
      if(suggestion){
        advanced.appendChild(element('p',t("Move under ","Verschieben unter ")+suggestion.target,'path'));
        advanced.appendChild(element('p',referenceAmount(suggestion.outgoing,suggestion.outgoingLowerBound)+t(" outgoing + "," ausgehende + ")+referenceAmount(suggestion.incoming,suggestion.incomingLowerBound)+t(" incoming "," eingehende ")+graphEvidenceUnit()+t(" with this target."," mit diesem Ziel."),'muted'));
        advanced.appendChild(element('p',(data.semantic?t("Grouping heuristic: strongest recorded dependency affinity among sibling packages. Source dependencies and file-level cycles stay unchanged.","Gruppierungsheuristik: stärkste erfasste Abhängigkeitsverbindung zwischen benachbarten Paketen. Quellabhängigkeiten und Zyklen auf Dateiebene bleiben unverändert."):t("Grouping heuristic: strongest recorded import affinity among sibling packages. Imports and file-level cycles stay unchanged.","Gruppierungsheuristik: stärkste erfasste Importverbindung zwischen benachbarten Paketen. Imports und Zyklen auf Dateiebene bleiben unverändert.")),'muted'));
        if(suggestion.rankingLowerBound)advanced.appendChild(element('p',t("Some totals are unknown; candidates are ranked using sample lower bounds.","Einige Gesamtzahlen sind unbekannt; Kandidaten werden anhand der Untergrenzen aus Stichproben geordnet."),'muted'));
        advanced.appendChild(button(t("Simulate suggested move","Vorgeschlagene Verschiebung simulieren"),()=>{
          setSimulation(true);document.getElementById('sidebar').focus();
          askSimulation(encodeMove(n.name,suggestion.target),t('Move {name} under {target}?','{name} unter {target} verschieben?',{name:n.name,target:suggestion.target}));
        },'btn btn-primary'));
      }else advanced.appendChild(element('p',t("No supported move among sibling packages: no dependency evidence supports a valid target.","Keine belegte Verschiebung zwischen benachbarten Paketen: Für ein gültiges Ziel fehlen Abhängigkeitsbelege."),'muted'));
    }
    if(ui.simulation){
      const label=element('label',t("Move under module","Unter Modul verschieben"));const select=element('select');select.setAttribute('data-manual-move-target','true');select.setAttribute('aria-label',t("Move target module","Zielmodul der Verschiebung"));
      for(const parent of work.nodes.filter(x=>x.kind==='module'&&moveTargetAllowed(work,nodeById(work,n.src),x))) {const option=element('option',parent.name);option.value=parent.name;select.appendChild(option);}
      select.value=suggestion?suggestion.target:select.children[0]?.value||'';
      label.appendChild(select);advanced.appendChild(label);
      const move=button(t("Propose move","Verschiebung vorschlagen"),()=>askSimulation(encodeMove(n.name,select.value),t('Move {name} under {target}?','{name} unter {target} verschieben?',{name:n.name,target:select.value})));move.disabled=!select.children.length;advanced.appendChild(move);
    }
    for(const direction of ['out','in']){
      const edges=direction==='out'?outgoing:incoming;
      const section=element('details',undefined,'inspector-section');section.setAttribute('data-inspector-neighbors',direction);section.open=false;
      section.appendChild(element('summary',(direction==='out'?t("Depends on","Abhängig von"):t("Used by","Verwendet von"))+' · '+edges.length));
      const neighbors=element('div',undefined,'inspector-neighbors');section.appendChild(neighbors);
      if(!edges.length)neighbors.appendChild(element('p',t("No recorded dependencies in this direction.","Keine erfassten Abhängigkeiten in dieser Richtung."),'muted'));
      for(const edge of edges.slice(0,100)){
        const other=nodeById(view,direction==='out'?edge.to:edge.from);
        const row=element('div',undefined,'inspector-neighbor'),node=button(other.name.split('/').pop(),()=>selectNode(other.name,true),'btn btn-quiet neighbor');
        node.title=other.name;node.setAttribute('aria-label',t("Inspect ","Untersuchen: ")+other.name);row.appendChild(node);
        const evidence=button(weightLabel(edge)+t(" refs"," Referenzen"),()=>selectEdge(edge),'btn btn-quiet');evidence.setAttribute('aria-label',t("Inspect dependency evidence: ","Abhängigkeitsbelege untersuchen: ")+(direction==='out'?n.name+' → '+other.name:other.name+' → '+n.name));row.appendChild(evidence);
        neighbors.appendChild(row);
      }
      if(edges.length>100)neighbors.appendChild(element('p',t("Showing 100 of ","Anzeige von 100 aus ")+edges.length+t(" neighbors. Use search to inspect others."," Nachbarn. Verwende die Suche, um weitere zu untersuchen."),'muted'));
      sb.appendChild(section);
    }
    renderQualityInspector(sb,n.src);renderTestCoverageDetails(sb,n.src);renderReviewDetails(sb,n.src);renderPerformanceDetails(sb,n.src);
    if(n.kind==='module'||ui.simulation)sb.appendChild(advanced);
  } else if(sel.kind==='edge'){
    const edge=selectedEdge();if(!edge)return;
    detailHeading(sb,[sel.from,sel.to]);
    renderDependencyDetailGraph(sb,sel.from,sel.to);
    sb.appendChild(element('p',weightLabel(edge)+' '+graphEvidenceUnit()+' · '+edge.refs.length+t(" evidence samples"," Belegstichproben"),'badge'));
    sb.appendChild(element('p',edge.count==null?t("Legacy total is unknown; the displayed count is a lower bound.","Die Gesamtzahl des Altberichts ist unbekannt; die angezeigte Anzahl ist eine Untergrenze."):edge.count>edge.refs.length?t("Evidence is sampled; the total count includes every recorded dependency reference.","Die Belege sind Stichproben; die Gesamtzahl umfasst alle erfassten Abhängigkeitsbelege."):t("All recorded dependency evidence is shown.","Alle erfassten Abhängigkeitsbelege werden angezeigt."),'muted'));
    const evidence=edge.refs.map(r=>r.file+':'+r.line+'  '+r.raw).join('\n');
    const actions=element('div',undefined,'context-actions');actions.appendChild(button(t("Copy evidence","Belege kopieren"),()=>copyText(evidence)));
    actions.appendChild(button(t("Source","Quelle"),()=>selectNode(sel.from,true)));actions.appendChild(button(t("Target","Ziel"),()=>selectNode(sel.to,true)));
    if(ui.simulation)actions.appendChild(button(t("Propose cut","Trennung vorschlagen"),()=>askSimulation(encodeCut(sel.from,sel.to),t('Cut {from} → {to}?','Kante {from} → {to} trennen?',{from:sel.from,to:sel.to})),'btn btn-primary'));
    sb.appendChild(actions);
    const sourceDetails=element('details',undefined,'inspector-section');sourceDetails.setAttribute('data-source-evidence','true');sourceDetails.appendChild(element('summary',t('Source evidence','Quellbelege')+' · '+edge.refs.length));sb.appendChild(sourceDetails);
    for(const ref of edge.refs){const row=element('div',undefined,'evidence');row.appendChild(element('div',ref.file+':'+ref.line,'path'));row.appendChild(element('pre',ref.raw));sourceDetails.appendChild(row);}
    if(!edge.refs.length)sb.appendChild(element('p',t("No source evidence is available for this edge.","Für diese Kante sind keine Quellcode-Belege verfügbar."),'empty'));
  } else if(sel.kind==='cycle'){
    const cycle=cyclesForView().find(c=>cycleKey(c)===sel.key);if(!cycle){ui.selection=null;renderSidebar();return;}
    sb.appendChild(element('h2',cycle.id));sb.appendChild(element('p',cycle.origin==='package'?t("Package-level cycle context","Zykluskontext auf Paketebene"):ops.length?t("Recomputed simulated cycle","Neu berechneter simulierter Zyklus"):cycle.origin==='current'?t("Cycle in current detail scope","Zyklus im aktuellen Detailbereich"):t("Recorded report finding","Erfasster Berichtsbefund"),'badge'));
    if(cycle.origin==='package')sb.appendChild(element('p',t("This cycle exists at a coarser package level. Its real dependencies are shown in the current detail view; they need not form a file-level cycle. Red arrows and scope metrics refer only to the current detail scope.","Dieser Zyklus besteht auf einer übergeordneten Paketebene. Seine tatsächlichen Abhängigkeiten werden in der aktuellen Detailansicht gezeigt; sie müssen keinen Zyklus auf Dateiebene bilden. Rote Pfeile und Bereichsmetriken beziehen sich nur auf den aktuellen Detailbereich."),'muted'));
    sb.appendChild(element('p',cycle.nodes.length+t(" members · "," Mitglieder · ")+cycle.edge_count+t(" directed edges"," gerichtete Kanten"),'muted'));
    if(cycle.sampling&&!cycle.sampling.complete)sb.appendChild(element('p',t("Cycle sampling is incomplete: searched from ","Die Zyklusstichprobe ist unvollständig: Suche ab ")+(cycle.sampling.searched_from||t("no root","keinem Wurzelknoten"))+', '+cycle.sampling.explored_edges+t(" edges explored. SCC detection still establishes the cycle."," Kanten untersucht. Die SCC-Erkennung belegt den Zyklus weiterhin."),'muted'));
    const cycleGraph=detailScopeGraph(cycle.nodes);
    renderDetailGraph(sb,{...cycleGraph,label:t('Cycle connections','Verbindungen der Zyklusgruppe'),onNode:n=>revealNode(n.name)});
    const members=element('details',undefined,'inspector-section');members.appendChild(element('summary',t('All members','Alle Mitglieder')+' · '+cycle.nodes.length));
    for(const name of cycle.nodes)members.appendChild(button(name,()=>revealNode(name),'btn btn-quiet neighbor'));sb.appendChild(members);
    sb.appendChild(button(t("Compare solutions","Lösungen vergleichen"),()=>compareCycleSolutions(cycle),'btn btn-primary'));
    sb.appendChild(element('p',t("Compare dependency changes ranked by affected source files, dependency evidence and number of dependencies. Each candidate is checked against the graph.","Vergleiche Abhängigkeitsänderungen, geordnet nach betroffenen Quelldateien, Abhängigkeitsbelegen und Anzahl der Abhängigkeiten. Jeder Kandidat wird anhand des Graphen geprüft."),'muted'));
    renderRepairComparison(sb,cycle);
    sb.appendChild(button(t("Return to overview","Zur Übersicht zurück"),clearCycle));
  }
}
function renderExplorer() {
  document.getElementById('clear-search').hidden=!document.getElementById('filter').value;
  const list=document.getElementById('module-list');list.textContent='';
  const bulk=treeBulkState();
  for(const action of ['expand','collapse'])document.getElementById('explorer-'+action+'-all').disabled=!bulk[action];
  const q=currentFilter(),matches=work.nodes.filter(n=>!q||n.name.toLowerCase().includes(q));
  const byId=new Map(work.nodes.map(n=>[n.id,n])),children=new Map(),included=new Set();
  // The tree includes ancestors outside the current analytical cut. Summarize
  // its ranked descendants without attributing their score to the parent.
  const subtreePriorities=new Map();
  if(ui.highlightPriorities)for(const review of currentReview?.nodes||[]){
    if(review.source_id===null||review.priority==='low')continue;
    for(const id of ancestorsOf(work,review.source_id)){
      const summary=subtreePriorities.get(id);
      if(summary)summary.count++;
      else subtreePriorities.set(id,{review,count:1}); // rows are highest score first
    }
  }
  // Parent IDs establish the hierarchy; path strings are only display labels.
  for(const node of work.nodes){
    const parent=byId.has(node.parent)?node.parent:null;
    if(!children.has(parent))children.set(parent,[]);children.get(parent).push(node);
  }
  for(const siblings of children.values())siblings.sort((a,b)=>(a.kind==='module'?0:1)-(b.kind==='module'?0:1)||compareNames(a.name,b.name));
  if(q)for(const node of matches){included.add(node.id);for(const id of ancestorsOf(work,node.id))included.add(id);}
  const rows=[],seen=new Set(),stack=(children.get(null)||[]).slice().reverse();
  while(stack.length){
    const node=stack.pop();if(seen.has(node.id)||(q&&!included.has(node.id)))continue;
    seen.add(node.id);rows.push(node);
    if(explorerOpen.has(node.id)||q)for(const child of (children.get(node.id)||[]).slice().reverse())stack.push(child);
  }
  // Keep an explicitly selected path visible without rendering every preceding
  // sibling. The remaining rows retain their normal preorder and page budget.
  const selected=ui.selection?.kind==='node'?rows.find(n=>n.name===ui.selection.name):null;
  const shown=new Set(selected?[selected.id,...ancestorsOf(work,selected.id)]:[]);
  const limit=Math.max(ui.searchLimit,shown.size);
  for(const node of rows){if(shown.size>=limit)break;shown.add(node.id);}
  const displayed=rows.filter(n=>shown.has(n.id)),containers=new Map([[null,list]]),controls=new Map();
  function focusNode(id){
    if(!controls.has(id))return;
    navigateExplorerNode(id,explorerOpen.has(id));
    [...list.querySelectorAll('button[data-node]')].find(b=>b.dataset.node===String(id))?.focus();
  }
  function expand(id,opened,toggleFocus=false){
    navigateExplorerNode(id,opened);
    const attr=toggleFocus?'data-toggle':'data-node';
    [...list.querySelectorAll('button['+attr+']')].find(b=>b.getAttribute(attr)===String(id))?.focus();
  }
  function navigate(event,node){
    if(!['ArrowUp','ArrowDown','ArrowLeft','ArrowRight','Home','End'].includes(event.key))return;
    event.preventDefault();const index=displayed.indexOf(node),kids=children.get(node.id)||[],opened=!!q||explorerOpen.has(node.id);
    if(event.key==='ArrowUp')focusNode(displayed[Math.max(0,index-1)]?.id);
    if(event.key==='ArrowDown')focusNode(displayed[Math.min(displayed.length-1,index+1)]?.id);
    if(event.key==='Home')focusNode(displayed[0]?.id);
    if(event.key==='End')focusNode(displayed[displayed.length-1]?.id);
    if(event.key==='ArrowRight'&&kids.length){if(!opened)expand(node.id,true);else focusNode(kids.find(n=>controls.has(n.id))?.id);}
    if(event.key==='ArrowLeft'){if(kids.length&&opened&&!q)expand(node.id,false);else focusNode(node.parent);}
  }
  for(const node of displayed){
    const item=element('li',undefined,'explorer-item'),row=element('div',undefined,'explorer-tree-row');item.appendChild(row);
    (containers.get(node.parent)||list).appendChild(item);
    const kids=children.get(node.id)||[],opened=!!q||explorerOpen.has(node.id);
    if(kids.length){
      const toggle=button(opened?'▾':'▸',()=>expand(node.id,!opened,true),'explorer-toggle');
      toggle.setAttribute('data-toggle',String(node.id));toggle.setAttribute('aria-expanded',String(opened));toggle.setAttribute('aria-controls','explorer-children-'+node.id);
      toggle.setAttribute('aria-label',opened?t('Collapse {name}','{name} zuklappen',{name:node.name}):t('Expand {name}','{name} aufklappen',{name:node.name}));toggle.disabled=!!q;
      toggle.title=q?t('Search temporarily expands matching branches. Clear search to collapse.','Die Suche klappt passende Zweige vorübergehend auf. Zum Zuklappen die Suche löschen.'):(opened?t('Collapse {name}','{name} zuklappen',{name:node.name}):t('Expand {name}','{name} aufklappen',{name:node.name}));
      toggle.addEventListener('keydown',event=>navigate(event,node));row.appendChild(toggle);
    }else{const spacer=element('span',undefined,'explorer-toggle-space');spacer.setAttribute('aria-hidden','true');row.appendChild(spacer);}
    const select=button(undefined,()=>{
      navigateExplorerNode(node.id);
      [...list.querySelectorAll('button[data-node]')].find(b=>b.dataset.node===String(node.id))?.focus();
    },'explorer-row explorer-node');select.setAttribute('data-node',String(node.id));select.title=node.name;
    select.setAttribute('aria-pressed',String(!!ui.selection&&ui.selection.kind==='node'&&ui.selection.name===node.name));
    select.setAttribute('aria-label',node.name+' · '+(node.kind==='module'?t('package','Paket'):t('file','Datei')));
    const icon=element('span',undefined,'explorer-icon '+(node.kind==='module'?'package-icon':'file-icon'));icon.setAttribute('aria-hidden','true');select.appendChild(icon);
    const text=element('span',undefined,'explorer-text');text.appendChild(element('span',node.name.slice(node.name.lastIndexOf('/')+1),'explorer-name'));

    select.appendChild(text);
    const qualityCount=qualityNodeCount(node.id);
    if(qualityCount){const badge=element('span',qualityBadgeText(qualityCount),'quality-badge');badge.setAttribute('data-quality-count',String(qualityCount));badge.title=qualityBadgeDescription(qualityCount);select.appendChild(badge);select.setAttribute('aria-label',select.getAttribute('aria-label')+' · '+badge.title);}
    const direct=reviewBySource.get(node.id),contained=subtreePriorities.get(node.id),review=direct||contained?.review;
    if(ui.highlightPriorities&&review&&review.priority!=='low'){
      const label=review.priority==='high'?'P1':'P2';
      const scope=direct?t('Current graph scope','Aktueller Graph-Ausschnitt'):contained.count===1?t('1 prioritized descendant in current graph scope. Highest: {name}','1 priorisierter Nachfahre im aktuellen Graph-Ausschnitt. Höchste Priorität: {name}',{name:review.name}):t('{count} prioritized descendants in current graph scope. Highest: {name}','{count} priorisierte Nachfahren im aktuellen Graph-Ausschnitt. Höchste Priorität: {name}',{count:contained.count,name:review.name});
      const description=scope+'\n'+reviewPriorityText(review)+'\n'+review.reasons.map(reviewReasonText).join('\n')+
        (review.changes==null?t('\nGit unavailable for this node; structural evidence only.','\nGit ist für diesen Knoten nicht verfügbar; nur strukturelle Belege.'):currentReview.git.shallow||currentReview.git.truncated?t('\nGit change touches are lower bounds.','\nDie Git-Änderungszahlen sind Untergrenzen.'):'');
      const badge=element('span',label+(direct?'':' ↓'),'explorer-priority');
      badge.setAttribute('data-tree-priority',review.priority);badge.setAttribute('data-priority-scope',direct?'node':'subtree');
      badge.setAttribute('title',description+'\n'+metricHelp('priority'));badge.setAttribute('aria-hidden','true');select.appendChild(badge);
      select.title=node.name+'\n'+description;
      select.setAttribute('aria-label',select.getAttribute('aria-label')+' · '+description);
    }
    select.addEventListener('keydown',event=>{if(event.key==='Enter'){event.preventDefault();event.stopPropagation();selectNode(node.name);openInspector(true);}else navigate(event,node);});row.appendChild(select);controls.set(node.id,select);
    if(kids.length){const nested=element('ul',undefined,'explorer-children');nested.setAttribute('id','explorer-children-'+node.id);nested.hidden=!opened;item.appendChild(nested);containers.set(node.id,nested);}
  }
  document.getElementById('search-count').textContent=q?t('{count} results for “{query}”','{count} Ergebnisse für „{query}“',{count:matches.length,query:document.getElementById('filter').value}):t('{shown} of {total} items shown','{shown} von {total} Einträgen angezeigt',{shown:displayed.length,total:work.nodes.length});
  if(!matches.length)list.appendChild(element('li',q?t('No matching modules or files. Clear search to see the project.','Keine passenden Module oder Dateien. Suche löschen, um das Projekt anzuzeigen.'):t('No source files were extracted. Check the project path, languages and exclusions.','Keine Quelldateien extrahiert. Projektpfad, Sprachen und Ausschlüsse prüfen.'),'empty'));
  if(rows.length>displayed.length){
    const more=element('li'),next=rows.find(n=>!shown.has(n.id));
    more.appendChild(button(t('Show next 100 ({count} remaining)','Nächste 100 anzeigen ({count} verbleibend)',{count:rows.length-displayed.length}),()=>{ui.searchLimit+=100;renderExplorer();[...list.querySelectorAll('button[data-node]')].find(b=>b.dataset.node===String(next.id))?.focus();}));list.appendChild(more);
  }
}
function renderSummary() {
  const strip=document.getElementById('summary');hideMetricTooltip(strip);strip.textContent='';
  strip.hidden=isRepositoryOverview();if(strip.hidden)return;
  for(const [key,label,value] of [['scope',t('Scope nodes','Knoten im Umfang'),view.nodes.length],['dependencies',t('Dependencies','Abhängigkeiten'),view.edges.length],['cycles',t('Cyclic groups','Zyklusgruppen'),sccs.length],['acd','ACD',currentAcd.toFixed(2)]]){
    const metric=element('div',undefined,'metric');metric.appendChild(element('strong',String(value)));metric.appendChild(document.createTextNode(label));
    explainMetric(metric,key,key==='acd'?t('Current value: each node reaches about {value} nodes on average, including itself.','Aktueller Wert: Jeder Knoten erreicht durchschnittlich etwa {value} Knoten, einschließlich sich selbst.',{value}):'');
    const hint=element('span',' ⓘ','metric-help-marker');hint.setAttribute('aria-hidden','true');metric.appendChild(hint);strip.appendChild(metric);
  }
  if(currentReview){
    const metrics=currentReview.summary,details=element('details',undefined,'kpi-details');
    details.appendChild(element('summary',t('More metrics','Weitere Kennzahlen')));
    const grid=element('div',undefined,'kpi-grid');
    for(const [key,label,value] of [['ccd','CCD',metrics.ccd],['nccd','NCCD',metrics.nccd.toFixed(2)],['largestScc',t('Largest cyclic group','Größte Zyklusgruppe'),metrics.largest_scc],['cyclicNodes',t('Nodes in cycles','Knoten in Zyklen'),metrics.cyclic_nodes],['priority',t('High priority','Hohe Priorität'),metrics.high_priority],['orphans',t('Orphan candidates','Verwaiste Kandidaten'),metrics.orphan_candidates],['sourceLines',t('Source lines','Quellzeilen'),metrics.lines??t('Unavailable','Nicht verfügbar')],['sourceChanges',t('Git file touches','Git-Dateiänderungen'),reviewChangesText(metrics.changes)],['coverage',t('Test line coverage','Zeilen-Testabdeckung'),(()=>{const summary=coverageScopeSummary();return summary?coverageCounterText(summary):'—';})()]]){
      const metric=element('div',undefined,'metric');metric.appendChild(element('strong',String(value)));metric.appendChild(document.createTextNode(label));grid.appendChild(explainMetric(metric,key));
    }
    details.appendChild(grid);details.appendChild(element('p',t('All KPIs describe the current analysis scope, including filtered files. Source lines and Git evidence describe the scanned files; simulation does not change those measurements.','Alle Kennzahlen beschreiben den aktuellen Analyseumfang einschließlich ausgefilterter Dateien. Quellzeilen und Git-Belege beziehen sich auf die gescannten Dateien; Simulationen ändern diese Messwerte nicht.'),'muted'));appendCoverageSummary(details);appendQualitySummary(details);strip.appendChild(details);
  }
  const errors=(data.findings||[]).filter(f=>f.severity==='error').length;
  strip.appendChild(explainMetric(element('div',t('Report: {errors} error findings','Bericht: {errors} Fehlerbefunde',{errors})+(ops.length?t(' · view is simulated',' · simulierte Ansicht'):''),'report-status'),'report'));
  if(data.semantic){
    const coverage=explainContext(element('span',t('Analysis: {status} ⓘ','Analyse: {status} ⓘ',{status:callCoverageLabel(data.semantic.status)}),'graph-coverage muted'),graphCoverageDescription()+(!sccs.length&&graphCycleCoverageWarning()?' '+graphCycleCoverageWarning():''));
    coverage.setAttribute('data-graph-coverage','true');strip.appendChild(coverage);
  }
  renderNetworkState();
}

function reviewPriorityText(row){
  const priority={high:t('High','Hohe'),medium:t('Medium','Mittlere'),low:t('Low','Niedrige')}[row.priority]||row.priority.charAt(0).toUpperCase()+row.priority.slice(1);
  return t('{priority} review priority · {score}/100','{priority} Prüfpriorität · {score}/100',{priority,score:row.score});
}
function reviewChangesText(value){return value==null?t('Unavailable','Nicht verfügbar'):String(value)+(currentReview?.git.shallow||currentReview?.git.truncated?t(' (lower bound)',' (Untergrenze)'):'');}
// Translate only the known structural reasons generated by computeReview.
// The cached analysis and unknown/source-supplied messages remain unchanged.
function reviewReasonText(reason){
  let match;
  if((match=/^Cyclic group: (\d+) nodes \(\+(\d+)\)$/.exec(reason)))return t('Cyclic group: {nodes} nodes (+{score})','Zyklische Gruppe: {nodes} Knoten (+{score})',{nodes:match[1],score:match[2]});
  if((match=/^Direct coupling: (\d+) incoming \+ (\d+) outgoing \(\+(\d+)\)$/.exec(reason)))return t('Direct coupling: {incoming} incoming + {outgoing} outgoing (+{score})','Direkte Kopplung: {incoming} eingehend + {outgoing} ausgehend (+{score})',{incoming:match[1],outgoing:match[2],score:match[3]});
  if((match=/^Transitive dependents: (\d+) \(\+(\d+)\)$/.exec(reason)))return t('Transitive dependents: {count} (+{score})','Transitiv abhängige Knoten: {count} (+{score})',{count:match[1],score:match[2]});
  if((match=/^Git change touches × coupling: (\d+) × (\d+) \(\+(\d+)\)$/.exec(reason)))return t('Git change touches × coupling: {changes} × {coupling} (+{score})','Git-Dateiänderungen × Kopplung: {changes} × {coupling} (+{score})',{changes:match[1],coupling:match[2],score:match[3]});
  if(reason==='No incoming imports in this scope; check entry points and dynamic consumers before removing code.')return t('No incoming dependencies in this scope; check entry points and dynamic consumers before removing code.','Keine eingehenden Abhängigkeiten in diesem Analysebereich; vor dem Entfernen Einstiegspunkte und dynamische Aufrufer prüfen.');
  return reason;
}
function renderReviewDetails(container,sourceId){
  const row=reviewBySource.get(sourceId);if(!row)return;
  const section=element('section',undefined,'review-details');section.appendChild(explainMetric(element('h3',reviewPriorityText(row),'review-score'),'priority'));
  const reasons=element('ul');for(const reason of row.reasons)reasons.appendChild(element('li',reviewReasonText(reason)));section.appendChild(reasons);
  if(!row.reasons.length)section.appendChild(element('p',t('No priority signals in this scope. This does not establish absence of defects.','Keine Prioritätssignale in diesem Analysebereich. Das belegt keine Fehlerfreiheit.'),'muted'));
  const metrics=element('dl');
  for(const [label,value] of [[t('Transitive dependencies','Transitive Abhängigkeiten'),row.dependencies],[t('Transitive dependents','Transitiv abhängige Knoten'),row.dependents],[t('Cyclic group size','Größe der zyklischen Gruppe'),row.scc_size],[t('Source files','Quelldateien'),row.file_count],[t('Mapped source files','Zugeordnete Quelldateien'),row.mapped_files+' / '+row.file_count],[t('Source lines','Quellcodezeilen'),row.lines??t('Unavailable','Nicht verfügbar')],[t('Git file touches','Git-Dateiänderungen'),reviewChangesText(row.changes)]]){metrics.appendChild(element('dt',label));metrics.appendChild(element('dd',String(value)));}
  section.appendChild(metrics);
  if(row.entry_point)section.appendChild(element('p',t('Contains a configured entry point; excluded from orphan candidates.','Enthält einen konfigurierten Einstiegspunkt und ist daher kein Kandidat für ungenutzten Code.'),'muted'));
  if(row.orphan)section.appendChild(explainMetric(element('p',(row.orphan==='isolated'?t('Isolated','Isolierter'):t('Unreferenced','Unreferenzierter'))+t(' candidate. Check external or dynamic consumers before deleting code.',' Kandidat. Vor dem Löschen externe oder dynamische Aufrufer prüfen.'),'muted'),'orphans'));
  if(row.changes==null)section.appendChild(element('p',t('Git contribution is unavailable for this node. Its priority uses structural evidence only.','Für diesen Knoten fehlen Git-Daten. Seine Priorität basiert ausschließlich auf Strukturbelegen.'),'muted'));
  if(ops.length)section.appendChild(element('p',t('Structure and priority reflect the simulation; source measurements remain from the original scan.','Struktur und Priorität entsprechen der Simulation; Quellcode-Messwerte stammen weiterhin aus dem ursprünglichen Scan.'),'muted'));
  container.appendChild(section);
}
function renderReviewPanel(){
  renderQualityFindings();
  renderTestCoveragePanel();
  if(!currentReview)return;
  const list=document.getElementById('review-list');list.textContent='';
  const q=currentFilter(),rows=currentReview.nodes.filter(row=>(!q||row.name.toLowerCase().includes(q))&&(!ui.reviewOrphans||row.orphan));
  document.getElementById('review-status').textContent=t('Current scope · {nodes} nodes · {high} high priority. Scores are a review order, not defect probabilities.','Aktueller Analysebereich · {nodes} Knoten · {high} mit hoher Priorität. Die Bewertung bestimmt eine Prüfreihenfolge, keine Fehlerwahrscheinlichkeit.',{nodes:currentReview.nodes.length,high:currentReview.summary.high_priority})+(ops.length?t(' Simulated structure.',' Simulierte Struktur.'):'');
  const git=currentReview.git;
  const gitMessage=data.reviewContext?.sources?.git?(git.message||t('no history evidence','keine Historienbelege')):t(git.message,'Keine Git-Historie mitgeliefert');
  document.getElementById('review-git').textContent=git.status==='available'?t('Git window: {since} · {commits} commits scanned','Git-Zeitraum: {since} · {commits} Commits analysiert',{since:git.since,commits:git.scanned_commits})+(git.shallow||git.truncated?t(' · incomplete history: change counts and scores are lower bounds.',' · unvollständige Historie: Änderungszahlen und Bewertungen sind Untergrenzen.'):'.'):t('Git unavailable: {message}. Priorities use structural evidence.','Git nicht verfügbar: {message}. Prioritäten basieren auf Strukturbelegen.',{message:gitMessage});
  document.getElementById('review-orphans').setAttribute('aria-pressed',String(ui.reviewOrphans));
  for(const row of rows.slice(0,ui.reviewLimit)){
    const item=element('li'),action=button(undefined,()=>{showTab('graph');revealNode(row.name);},'review-row');
    action.setAttribute('data-review-node',String(row.source_id));action.setAttribute('data-priority',row.priority);action.title=row.name+'\n'+row.reasons.map(reviewReasonText).join('\n');
    action.appendChild(element('strong','#'+row.rank+' '+row.name));action.appendChild(element('span',reviewPriorityText(row),'review-score'));
    action.appendChild(element('small',row.reasons.length?reviewReasonText(row.reasons[0]):t('No scored structural signals.','Keine bewerteten Struktursignale.')));
    if(row.orphan)action.appendChild(element('small',(row.orphan==='isolated'?t('Isolated','Isolierter'):t('Unreferenced','Unreferenzierter'))+t(' candidate · verify consumers',' Kandidat · Aufrufer prüfen')));
    item.appendChild(action);list.appendChild(item);
  }
  if(!rows.length)list.appendChild(element('li',ui.reviewOrphans?t('No orphan candidates match this scope/search.','Keine Kandidaten für ungenutzten Code in diesem Analysebereich bzw. dieser Suche.'):t('No review candidates match this scope/search.','Keine Prüfkandidaten in diesem Analysebereich bzw. dieser Suche.'),'empty'));
  if(rows.length>ui.reviewLimit){const more=element('li');more.appendChild(button(t('Show next 20 ({count} remaining)','Nächste 20 anzeigen ({count} verbleibend)',{count:rows.length-ui.reviewLimit}),()=>{ui.reviewLimit+=20;renderReviewPanel();}));list.appendChild(more);}
  renderPerformancePanel();
  renderReviewInsights();
}

function renderReviewInsights(){
  const panel=document.getElementById('review-insights');panel.textContent='';const insights=data.insights;if(!insights)return;
  const section=(title)=>{const details=element('details');details.appendChild(element('summary',title));details.appendChild(element('p',t("Original source/history evidence","Ursprüngliche Quellcode- und Historienbelege")+(ops.length?t(" · before simulation"," · vor der Simulation"):''),'muted'));panel.appendChild(details);return details;};
  const co=insights.co_change;
  if(co){
    const details=section(t("Co-change candidates","Kandidaten für gemeinsame Änderungen"));
    if(co.git?.status!=='available')details.appendChild(element('p',t("Git unavailable: ","Git nicht verfügbar: ")+(co.git?.message||t("no history evidence","keine Historienbelege")),'muted'));
    else{
      details.appendChild(element('p',co.considered_commits+t(" commits considered · "," Commits berücksichtigt · ")+co.skipped_bulk_commits+t(" bulk commits excluded · "," Massenänderungs-Commits ausgeschlossen · ")+co.total_candidates+t(" candidates without an observed import edge"," Kandidaten ohne beobachtete Importkante"),'muted'));
      if(co.git.shallow||co.git.truncated||co.pair_search_truncated||co.results_truncated)details.appendChild(element('p',t("History, pair search or results are incomplete; inspect coverage before interpreting these ratios.","Historie, Paarsuche oder Ergebnisse sind unvollständig; vor der Interpretation dieser Verhältnisse die Abdeckung prüfen."),'muted'));
      for(const pair of (co.pairs||[]).slice(0,20)){
        details.appendChild(element('h4',pair.left.name+' ↔ '+pair.right.name));
        details.appendChild(element('p',pair.shared_commits+t(" shared commits · "," gemeinsame Commits · ")+Math.round(pair.left_to_right*100)+t("% of left changes accompany right · ","% der Änderungen links treten auch rechts auf · ")+Math.round(pair.right_to_left*100)+t("% of right changes accompany left","% der Änderungen rechts treten auch links auf"),'muted'));
        for(const file of [pair.left,pair.right])details.appendChild(button(t("Inspect ","Untersuchen: ")+file.name,()=>openSourceEvidence(file.node_id,file.name),'btn btn-quiet neighbor'));
        details.appendChild(element('p',t("Commit evidence: ","Commit-Belege: ")+pair.evidence_commits.join(', '),'path'));
      }
      if(!(co.pairs||[]).length)details.appendChild(element('p',t("No pairs meet the configured co-change thresholds in the available history.","Keine Paare erfüllen die konfigurierten Schwellenwerte für gemeinsame Änderungen in der verfügbaren Historie."),'muted'));
      if((co.pairs||[]).length>20)details.appendChild(element('p',t("Showing 20 of ","Anzeige von 20 aus ")+co.pairs.length+t(" reported pairs."," gemeldeten Paaren."),'muted'));
    }
    for(const warning of (co.warnings||[]).slice(0,5))details.appendChild(element('p',warning,'muted'));
  }
  const ownership=insights.ownership;
  if(ownership){
    const details=section(t("Team boundaries · CODEOWNERS","Teamgrenzen · CODEOWNERS"));
    if(ownership.status!=='available')details.appendChild(element('p',t("Ownership unavailable; no complete local CODEOWNERS evidence.","Zuständigkeit nicht verfügbar; keine vollständigen lokalen CODEOWNERS-Belege."),'muted'));
    else{
      details.appendChild(element('p',ownership.codeowners_path+' · '+ownership.owned_files+' / '+ownership.total_files+t(" files assigned · "," Dateien zugeordnet · ")+ownership.unowned_files+t(" unowned"," ohne Zuständigkeit")+(ownership.complete?'':t(" · incomplete coverage"," · unvollständige Abdeckung")),'muted'));
      details.appendChild(element('h4',t("Cross-owner dependencies: ","Abhängigkeiten zwischen Zuständigen: ")+ownership.cross_owner_dependencies));
      const dependencies=(ownership.dependencies||[]).filter(row=>row.cross_owner);
      for(const row of dependencies.slice(0,20)){
        details.appendChild(element('p',row.from_owners.join(', ')+' → '+row.to_owners.join(', ')+' · '+row.count+(row.count_is_lower_bound?'+':'')+' '+graphEvidenceUnit(),'path'));
        details.appendChild(button(t("Inspect ","Untersuchen: ")+row.from_name,()=>openSourceEvidence(row.from,row.from_name),'btn btn-quiet neighbor'));
        details.appendChild(button(t("Inspect ","Untersuchen: ")+row.to_name,()=>openSourceEvidence(row.to,row.to_name),'btn btn-quiet neighbor'));
        for(const ref of row.evidence.slice(0,3))details.appendChild(element('pre',ref.file+':'+ref.line+' '+ref.raw));
      }
      for(const finding of (ownership.findings||[]).slice(0,10))details.appendChild(element('p',finding.severity+': '+finding.id+' · '+finding.nodes.join(' → '),'muted'));
      if(dependencies.length>20)details.appendChild(element('p',t("Showing 20 of ","Anzeige von 20 aus ")+dependencies.length+t(" cross-owner dependencies."," Abhängigkeiten zwischen Zuständigen."),'muted'));
    }
    for(const warning of (ownership.warnings||[]).slice(0,5))details.appendChild(element('p',warning,'muted'));
  }
  const architecture=insights.architecture;
  if(architecture){
    const details=section(t("Architecture intent & trade-offs","Architekturziele und Abwägungen")),intent=architecture.intent||{};
    for(const goal of intent.goals||[]){details.appendChild(element('h4',goal.name));details.appendChild(element('p',t("User priority: ","Benutzerpriorität: ")+(goal.priority||t("unspecified","nicht angegeben"))+' · '+goal.rationale,'muted'));}
    for(const constraint of intent.constraints||[]){details.appendChild(element('h4',t("Constraint: ","Randbedingung: ")+constraint.name));details.appendChild(element('p',constraint.rationale,'muted'));}
    for(const decision of intent.decisions||[]){
      details.appendChild(element('h4',decision.id+' · '+decision.title));details.appendChild(element('p',decision.status+' · '+decision.chosen_option,'muted'));
      if(decision.alternatives.length)details.appendChild(element('p',t("Alternatives: ","Alternativen: ")+decision.alternatives.join('; '),'muted'));
      if(decision.trade_offs.length)details.appendChild(element('p',t("Accepted trade-offs: ","Akzeptierte Kompromisse: ")+decision.trade_offs.join('; '),'muted'));
      for(const reference of decision.links)details.appendChild(element('p',t("Reference: ","Referenz: ")+reference,'path'));
    }
    if(!(intent.goals||[]).length&&!(intent.decisions||[]).length)details.appendChild(element('p',t("No explicit goals or decisions were supplied. Priorities are not inferred from the graph.","Es wurden keine ausdrücklichen Ziele oder Entscheidungen angegeben. Prioritäten werden nicht aus dem Graphen abgeleitet."),'muted'));
    for(const question of architecture.questions||[])details.appendChild(element('p',question,'muted'));
  }
}

function performanceScopeLabel(){return t("Original report scope","Ursprünglicher Berichtsumfang")+(ops.length?t(" · before simulation"," · vor der Simulation"):'')+t("; source measurements are not recomputed by navigation.","; Quellcode-Messwerte werden bei der Navigation nicht neu berechnet.");}
function openSourceEvidence(id,name){
  const original=id==null?data.nodes.find(node=>node.name===name):null;
  const node=nodeById(work,id??original?.id);if(node)navigateExplorerNode(node.id,false);
}
function renderPerformancePanel(){
  const panel=document.getElementById('performance-panel');hideMetricTooltip(panel);panel.textContent='';
  const report=data.performance;if(!report)return;
  const details=element('details');details.appendChild(element('summary',t("Source & build indicators","Quellcode- und Build-Indikatoren")));
  details.appendChild(element('p',performanceScopeLabel(),'muted'));
  details.appendChild(element('p',t("Structural exposure and source evidence; no runtime or build-time measurements.","Strukturelle Belastung und Quellcode-Belege; keine Messung von Laufzeit oder Build-Dauer."),'muted'));
  for(const entry of (report.entries||[]).slice(0,20)){
    details.appendChild(explainMetric(button(entry.name+' · '+entry.reachable_files+t(" reachable files"," erreichbare Dateien"),()=>openSourceEvidence(entry.node_id,entry.name),'btn btn-quiet neighbor'),'closure'));
    details.appendChild(element('p',entry.longest_component_path+t(" components on longest path · "," Komponenten auf dem längsten Pfad · ")+entry.cyclic_components+t(" cyclic components · "," zyklische Komponenten · ")+(entry.lines??t("unknown","unbekannt"))+t(" source lines"," Quellcodezeilen"),'muted'));
  }
  if((report.entries||[]).length>20)details.appendChild(element('p',t("Showing 20 of ","Anzeige von 20 aus ")+report.entries.length+t(" entry points."," Einstiegspunkten."),'muted'));
  details.appendChild(element('h3',t("Cyclic build structure","Zyklische Build-Struktur")));
  for(const group of (report.cyclic_groups||[]).slice(0,10))details.appendChild(element('p',group.nodes.length+t(" scope nodes · "," Knoten im Analysebereich · ")+group.source_files+t(" source files · "," Quelldateien · ")+(group.lines??t("unknown","unbekannt"))+t(" lines: "," Zeilen: ")+group.nodes.join(', '),'path'));
  if(!(report.cyclic_groups||[]).length)details.appendChild(element('p',t("No cyclic groups in the original report scope.","Keine zyklischen Gruppen im ursprünglichen Berichtsumfang."),'muted'));
  details.appendChild(element('h3',t("Normalized Main Sequence Distance","Normalisierter Abstand zur Main Sequence")));
  const abstractions=(report.abstractions||[]).filter(row=>row.coverage==='complete'&&row.distance!=null).slice().sort((a,b)=>b.distance-a.distance||compareNames(a.name,b.name));
  for(const row of abstractions.slice(0,10)){
    const value=explainMetric(element('p',(row.scope==='scc'?t("Cyclic group ","Zyklische Gruppe "):'')+row.name+' · A '+row.abstractness.toFixed(2)+' · I '+row.instability.toFixed(2)+' · D′ '+row.distance.toFixed(2),'path'),'distance');details.appendChild(value);
  }
  if(!abstractions.length)details.appendChild(element('p',t("Unavailable: no complete compiler-backed type coverage for this report scope.","Nicht verfügbar: keine vollständige compilerbasierte Typabdeckung für diesen Berichtsumfang."),'muted'));
  details.appendChild(element('p',(report.source_findings||[]).filter(row=>row.kind==='heavy_import').length+t(" eager-import hints · "," Hinweise auf sofortige Imports · ")+(report.source_findings||[]).filter(row=>row.kind==='barrel').length+t(" barrel candidates · "," Barrel-Kandidaten · ")+(report.contract_candidates||[]).length+t(" concrete-call review candidates"," Prüfkandidaten für Aufrufe konkreter Typen")+(report.contract_candidates_truncated?t(" (limited)"," (begrenzt)"):''),'muted'));
  for(const finding of (report.source_findings||[]).slice(0,20))details.appendChild(button((finding.kind==='heavy_import'?t('heavy import','aufwendiger Import'):finding.kind==='barrel'?t('barrel','Barrel'):finding.kind.replaceAll('_',' '))+' · '+finding.file+':'+finding.line,()=>openSourceEvidence(finding.node_id,finding.name),'btn btn-quiet neighbor'));
  for(const diagnostic of (report.diagnostics||[]).slice(0,10))details.appendChild(element('p',diagnostic,'muted'));
  panel.appendChild(details);
}
function renderPerformanceDetails(container,sourceId,graph=work){
  const report=data.performance;if(!report)return;
  const ids=subtree(graph,sourceId),originalNames=new Set(data.nodes.filter(node=>ids.has(node.id)).map(node=>node.name));
  const findings=(report.source_findings||[]).filter(finding=>ids.has(finding.node_id));
  const originalName=data.nodes.find(node=>node.id===sourceId)?.name;
  const metric=(report.abstractions||[]).find(row=>row.scope==='node'&&row.name===originalName);
  const contracts=(report.contract_candidates||[]).filter(row=>originalNames.has(row.from_node)||originalNames.has(row.to_node));
  if(!findings.length&&!metric&&!contracts.length)return;
  const section=element('details',undefined,'review-details inspector-section');section.setAttribute('data-source-evidence','true');section.appendChild(element('summary',t("Source cost evidence · ","Quellcode-Kostenbelege · ")+(findings.length+contracts.length)+t(" findings"," Befunde")));
  section.appendChild(element('p',performanceScopeLabel(),'muted'));
  if(metric){
    if(metric.coverage==='complete'&&metric.abstractness!=null&&metric.distance!=null){
      section.appendChild(explainMetric(element('p','A '+metric.abstractness.toFixed(2)+' · '+metric.abstract_types+' / '+metric.types+t(" abstract types"," abstrakte Typen")),'abstractness'));
      section.appendChild(explainMetric(element('p','I '+metric.instability.toFixed(2)+t(" · normalized D′ "," · normalisiertes D′ ")+metric.distance.toFixed(2)),'distance'));
    }else section.appendChild(element('p',t("A / D′ unavailable: ","A / D′ nicht verfügbar: ")+(metric.reason||t("incomplete compiler coverage","unvollständige Compiler-Abdeckung")),'muted'));
  }
  for(const finding of findings.slice(0,20)){
    section.appendChild(element('h4',finding.kind==='barrel'?t("Barrel candidate · ","Barrel-Kandidat · ")+finding.reexports+' / '+finding.code_lines+t(" re-export lines"," Re-Export-Zeilen"):t("Eager import · ","Sofortiger Import · ")+finding.library));
    section.appendChild(element('p',finding.message,'muted'));section.appendChild(element('p',finding.file+':'+finding.line,'path'));section.appendChild(element('pre',finding.raw));
  }
  for(const candidate of contracts.slice(0,10)){
    section.appendChild(element('h4',callClassLabel(candidate.from_class)+' → '+callClassLabel(candidate.to_class)));section.appendChild(element('p',candidate.message,'muted'));
    if(candidate.interfaces.length)section.appendChild(element('p',t("Existing interface candidates: ","Vorhandene Interface-Kandidaten: ")+candidate.interfaces.map(callClassLabel).join(', '),'path'));
    for(const evidence of candidate.evidence)section.appendChild(element('pre',evidence.file+':'+evidence.line+' '+evidence.raw));
  }
  container.appendChild(section);
}

// The overview uses server findings; drill-down/simulation use current SCCs
// plus explicitly labelled coarser package-cycle context. Focusing a cycle
// includes view nodes that are ancestors or descendants of its members.
function cycleKey(cycle) { return JSON.stringify(cycle.nodes.slice().sort()); }
function compareNames(a,b) { return a<b?-1:a>b?1:0; }
function compareCycleSolutions(cycle) {
  const result=planCycleRepairs(work,cycle.nodes);
  repairComparison={key:cycleKey(cycle),revision:viewRevision,result};
  renderSidebar();
}
function repairPlanDocument(result,plan) {
  prepareCallModel();
  const steps=plan.ops.map(op=>t('Replace the dependency {from} → {to}. Inspect its source evidence before choosing a code change.','Die Abhängigkeit {from} → {to} ersetzen. Vor der Auswahl einer Codeänderung die zugehörigen Quellbelege prüfen.',{from:op.cut_edge.from,to:op.cut_edge.to}));
  // Calls are an independent original-source snapshot. Map stable source IDs
  // back to that snapshot even when the current model contains package moves.
  const originalById=new Map(data.nodes.map(n=>[n.id,n.name]));
  const calls=[];
  for(const op of plan.ops){
    const from=nodeByName(work,op.cut_edge.from),to=nodeByName(work,op.cut_edge.to);
    const fromNames=new Set([...subtree(work,from.id)].map(id=>originalById.get(id)));
    const toNames=new Set([...subtree(work,to.id)].map(id=>originalById.get(id)));
    const sites=(callModel.calls||[]).filter(c=>c.resolution==='project'&&fromNames.has(callClasses.get(c.from_class)?.graph_node)&&toNames.has(callClasses.get(c.to_class)?.graph_node));
    const methods=[...new Set(sites.filter(c=>c.dispatch==='instance'||c.dispatch==='interface').map(c=>callMethodLabel(c.to_method)))].sort(compareNames);
    if(methods.length)steps.push(t('Review a narrow contract owned by {from} for these observed operations: {methods}{more}. Check parameter and return types for dependencies back to {to}.','Einen schmalen Vertrag im Verantwortungsbereich von {from} für diese beobachteten Operationen prüfen: {methods}{more}. Parameter- und Rückgabetypen auf Rückabhängigkeiten zu {to} prüfen.',{from:op.cut_edge.from,to:op.cut_edge.to,methods:methods.slice(0,8).join('; '),more:methods.length>8?t(' (more operations in Calls)',' (weitere Operationen unter Aufrufe)'):''}));
    if(sites.some(c=>c.dispatch==='constructor'||c.dispatch==='static'))steps.push(t('Review construction/static calls across {from} → {to}; an instance interface alone cannot replace them. Check a factory or composition boundary and object lifetime.','Konstruktoraufrufe und statische Aufrufe über {from} → {to} prüfen; ein Instanz-Interface allein kann sie nicht ersetzen. Eine Factory oder Kompositionsgrenze sowie die Objektlebensdauer prüfen.',{from:op.cut_edge.from,to:op.cut_edge.to}));
    for(const site of sites)if(calls.length<20)calls.push({file:site.file,line:site.line,code:site.code,dispatch:site.dispatch,target:callMethodLabel(site.to_method)});
  }
  steps.push(t('Check responsibility, access modifiers and public API compatibility. Implement the selected change, run the compiler and tests, then rescan imports and calls to verify the cycle is actually removed.','Verantwortlichkeiten, Zugriffsmodifikatoren und Kompatibilität der öffentlichen API prüfen. Die gewählte Änderung umsetzen, Compiler und Tests ausführen und anschließend Imports und Aufrufe erneut analysieren, um die Auflösung des Zyklus zu bestätigen.'));
  const assumptions=[
    t('This is a dependency-removal hypothesis, not an implemented source-code refactoring. Every listed dependency must be removed or redirected by a valid code change.','Dieser Plan ist eine Hypothese zum Entfernen von Abhängigkeiten, keine umgesetzte Quellcode-Änderung. Jede aufgeführte Abhängigkeit muss durch eine gültige Codeänderung entfernt oder umgeleitet werden.'),
    t('Ranking uses affected source files, dependency evidence and dependency edits. It does not measure implementation effort or prove a minimum source-code change.','Die Rangfolge berücksichtigt betroffene Quelldateien, Abhängigkeitsbelege und geänderte Abhängigkeiten. Sie misst keinen Umsetzungsaufwand und beweist keine minimale Quellcode-Änderung.'),
    t('File counts cover importers whose modeled dependencies change. Implementing a contract or moving responsibility may require changes to additional files.','Die Dateizahl umfasst importierende Dateien, deren modellierte Abhängigkeiten sich ändern. Ein Vertrag oder eine verschobene Verantwortung kann Änderungen an weiteren Dateien erfordern.'),
    t('Layer-rule compliance and API compatibility are not evaluated in this standalone report.','Die Einhaltung von Schichtenregeln und API-Kompatibilität werden in diesem eigenständigen Bericht nicht bewertet.'),
    t('Call evidence comes from the original source snapshot; it is supporting evidence, not the simulated call graph. Coverage: {status}.','Aufrufbelege stammen aus dem ursprünglichen Quellcode-Stand; sie ergänzen die Analyse und bilden keinen simulierten Aufrufgraphen ab. Abdeckung: {status}.',{status:callCoverageLabel(callModel.status)})
  ];
  if(plan.referenceLowerBound)assumptions.push(t('Some reference totals are unknown; the reported import count is a lower bound.','Einige Gesamtzahlen der Verweise sind unbekannt; die angegebene Importzahl ist eine Untergrenze.'));
  if(result.rankingLowerBound)assumptions.push(t('Candidate ranking uses incomplete reference totals; resolving missing counts may change the order.','Die Rangfolge verwendet unvollständige Gesamtzahlen der Verweise; ergänzte Zahlen können die Reihenfolge ändern.'));
  if(ops.length)assumptions.push(t('This plan builds on {count} earlier simulated changes. Replay baseOps before plan.ops when reproducing the reported outcome.','Dieser Plan baut auf {count} früheren simulierten Änderungen auf. Zum Reproduzieren des Ergebnisses zuerst baseOps und dann plan.ops anwenden.',{count:ops.length}));
  return {schema:'archguard-cycle-repair/v1',project:data.root,
    sourceGraph:{root:data.root,nodes:data.nodes,edges:data.edges},baseOps:JSON.parse(JSON.stringify(ops)),
    scope:result.scopeNames,before:result.before,sourceBefore:result.sourceBefore,search:result.search,plan,steps,assumptions,calls};
}
function repairStats(label,before,after) {
  return t('{label}: {nodesBefore} → {nodesAfter} cyclic nodes · {edgesBefore} → {edgesAfter} cyclic dependencies · largest group {largestBefore} → {largestAfter}','{label}: {nodesBefore} → {nodesAfter} zyklische Knoten · {edgesBefore} → {edgesAfter} zyklische Abhängigkeiten · größte Gruppe {largestBefore} → {largestAfter}',{label,nodesBefore:before.cyclicNodes,nodesAfter:after.cyclicNodes,edgesBefore:before.cyclicEdges,edgesAfter:after.cyclicEdges,largestBefore:before.largestScc,largestAfter:after.largestScc});
}
function renderRepairComparison(box,cycle) {
  if(!repairComparison||repairComparison.key!==cycleKey(cycle)||repairComparison.revision!==viewRevision)return;
  const result=repairComparison.result;
  box.appendChild(element('h3',t('Repair alternatives','Lösungsalternativen')));
  if(result.error){
    let message=result.error;
    for(const [en,de] of [['Cycle member not found: ','Zyklusmitglied nicht gefunden: '],['Cycle member scopes overlap: ','Bereiche der Zyklusmitglieder überlappen: ']])if(message.startsWith(en)){message=t(en,de)+message.slice(en.length);break;}
    box.appendChild(element('p',message,'empty'));return;
  }
  box.appendChild(element('p',(result.search.complete?t('Candidate search completed. ','Kandidatensuche abgeschlossen. '):t('Bounded candidate search; additional alternatives may exist. ','Begrenzte Kandidatensuche; weitere Alternativen können existieren. '))+t('{count} candidates checked. Ranking is not a proof of minimum code changes.','{count} Kandidaten geprüft. Die Rangfolge beweist keine minimale Codeänderung.',{count:result.search.examined}),'muted'));
  if(result.rankingLowerBound)box.appendChild(element('p',t('Candidate order is uncertain because some import totals are only lower bounds.','Die Reihenfolge ist unsicher, weil einige Importzahlen nur Untergrenzen sind.'),'muted'));
  if(!result.plans.length){box.appendChild(element('p',t('No supported dependency-change plan was found for this scope. Inspect its source evidence or select a current cycle.','Für diesen Ausschnitt wurde kein unterstützter Plan zur Änderung von Abhängigkeiten gefunden. Quellbelege prüfen oder einen aktuellen Zyklus auswählen.'),'empty'));return;}
  for(const [index,plan] of result.plans.entries()){
    const document=repairPlanDocument(result,plan),card=element('article',undefined,'repair-plan');
    card.appendChild(element('h4','Option '+(index+1)+' · '+(plan.complete?t('target cycle removed in model','Zielzyklus im Modell aufgelöst'):t('partial improvement','teilweise Verbesserung'))));
    card.appendChild(element('p',t('{files} affected source files · {refs} dependency evidence records · {changes} dependency changes','{files} betroffene Quelldateien · {refs} Abhängigkeitsbelege · {changes} geänderte Abhängigkeiten',{files:plan.affectedFiles.length,refs:referenceAmount(plan.referenceCount,plan.referenceLowerBound),changes:plan.ops.length}),'badge'));
    for(const op of plan.ops)card.appendChild(element('p',op.cut_edge.from+' → '+op.cut_edge.to,'path'));
    card.appendChild(element('p',repairStats(t('Selected scope','Gewählter Ausschnitt'),result.before,plan.after)));
    card.appendChild(element('p',repairStats(t('Source dependency graph','Abhängigkeitsgraph des Quellcodes'),result.sourceBefore,plan.sourceAfter)));
    card.appendChild(element('p',t('Layer rules and public API compatibility require review.','Schichtenregeln und Kompatibilität der öffentlichen API müssen geprüft werden.'),'muted'));
    const details=element('details');details.appendChild(element('summary',t('Implementation steps and evidence','Umsetzungsschritte und Belege')));
    const steps=element('ol');for(const step of document.steps)steps.appendChild(element('li',step));details.appendChild(steps);
    details.appendChild(element('p',t('Affected files: ','Betroffene Dateien: ')+plan.affectedFiles.join(', '),'path'));
    for(const ref of plan.evidence.slice(0,10))details.appendChild(element('pre',ref.file+':'+ref.line+'  '+ref.raw));
    details.appendChild(element('p',t('Dependency evidence is sampled; reference totals and file counts describe the modeled change.','Abhängigkeitsbelege sind stichprobenartig; Gesamtzahlen der Verweise und Dateien beschreiben die modellierte Änderung.'),'muted'));
    for(const call of document.calls.slice(0,5))details.appendChild(element('pre',call.file+':'+call.line+'  '+call.target+'\n'+call.code));
    for(const assumption of document.assumptions)details.appendChild(element('p',assumption,'muted'));
    card.appendChild(details);
    const actions=element('div',undefined,'context-actions');
    actions.appendChild(button(t('Preview this plan','Plan als Vorschau anwenden'),()=>{setSimulation(true);askSimulation(plan.ops,t('Preview option {option}: replace {changes} dependencies across {files} source files? A valid source change is still required.','Option {option} als Vorschau: {changes} Abhängigkeiten in {files} Quelldateien ersetzen? Eine gültige Quellcode-Änderung ist weiterhin erforderlich.',{option:index+1,changes:plan.ops.length,files:plan.affectedFiles.length}),document);},'btn btn-primary'));
    actions.appendChild(button(t('Export repair plan','Lösungsplan exportieren'),()=>{
      const blob=new Blob([JSON.stringify(document,null,2)],{type:'application/json'}),a=element('a');a.href=URL.createObjectURL(blob);a.download='archguard-repair-plan.json';a.click();URL.revokeObjectURL(a.href);
      feedback(t('Exported the repair plan with evidence, assumptions and validation steps.','Lösungsplan mit Belegen, Annahmen und Prüfschritten exportiert.'));
    }));
    card.appendChild(actions);box.appendChild(card);
  }
}
function cyclesForView() {
  const cycles=ops.length||trail.length||navigationAtRoot||treeBaseCollapses.size?sccs.map(c=>({id:(ops.length?'Simulated ':'Current ')+c.id,nodes:c.members.map(id=>nodeById(view,id).name),edge_count:internalEdges(view,c).length,suggested_cuts:[],origin:'current'})):(data.findings||[]).filter(f=>f.kind==='cycle');
  for(const cycle of packageContext?.levels||[])if(!cycles.some(c=>cycleKey(c)===cycleKey(cycle)))cycles.push(cycle);
  return cycles.slice().sort((a,b)=>b.nodes.length-a.nodes.length||b.edge_count-a.edge_count||compareNames(cycleKey(a),cycleKey(b)));
}
function focusCycle(cycle) {
  ui.cycle=cycleKey(cycle);ui.selection={kind:'cycle',key:ui.cycle};
  for(const name of cycle.nodes){const node=nodeByName(work,name);if(node)for(const id of ancestorsOf(work,node.id).reverse())if(!trail.includes(id))trail.push(id);}
  recompute();showTab('graph');openInspector();renderAll();fitView();
  document.getElementById('sidebar').focus();
}
function clearCycle() {ui.cycle=null;isolated=null;ui.selection=null;reconcileSelection();renderAll();fitView();}
function renderCyclesPanel() {
  const ul=document.getElementById('cycles-panel');ul.textContent='';
  const q=currentFilter(),cycles=cyclesForView().filter(c=>!q||c.nodes.some(n=>n.toLowerCase().includes(q)));
  for(const cycle of cycles.slice(0,100)){
    const li=element('li');const row=button(cycle.id,()=>focusCycle(cycle),'explorer-row');row.setAttribute('aria-pressed',String(ui.cycle===cycleKey(cycle)));
    row.appendChild(element('small',t('{members} members · {edges} edges','{members} Mitglieder · {edges} Kanten',{members:cycle.nodes.length,edges:cycle.edge_count})));
    row.appendChild(element('small',cycle.nodes.slice(0,3).join(', ')+(cycle.nodes.length>3?'…':'')));
    li.appendChild(row);ul.appendChild(li);
  }
  if(!cycles.length)ul.appendChild(element('li',q?t('No cycles match your search.','Keine Zyklen passen zur Suche.'):ops.length?t('No cyclic groups remain in this simulated view.','In dieser simulierten Ansicht verbleiben keine zyklischen Gruppen.'):trail.length||navigationAtRoot||treeBaseCollapses.size?t('No cyclic groups in this detail scope.','Keine zyklischen Gruppen in diesem Detailausschnitt.'):t('No cycle findings in this report.','Keine Zyklusbefunde in diesem Bericht.'),'empty'));
  if(!cycles.length&&!sccs.length&&graphCycleCoverageWarning())ul.appendChild(element('li',graphCycleCoverageWarning(),'muted'));
  if(cycles.length>100)ul.appendChild(element('li',t('Showing the 100 largest groups. Narrow the search.','Die 100 größten Gruppen werden angezeigt. Suche eingrenzen.'),'muted'));
}

// Source hierarchy on the row axis; stable package buckets on the target axis.
const treeMatrixUi={scope:null,expanded:new Set(),cycle:null,cycleOnly:false,query:'',rowPage:0,cache:null,revision:null,evidence:null};
function buildTreeMatrix(graph,scope=null){
  const nodes=new Map(graph.nodes.map(n=>[n.id,n])),children=new Map(),chains=new Map();
  for(const n of graph.nodes){const parent=nodes.has(n.parent)?n.parent:null;if(!children.has(parent))children.set(parent,[]);children.get(parent).push(n);}
  for(const list of children.values())list.sort((a,b)=>(a.kind==='module'?0:1)-(b.kind==='module'?0:1)||compareNames(a.name,b.name));
  for(const n of graph.nodes){const chain=[],seen=new Set();let current=n;while(current&&!seen.has(current.id)){seen.add(current.id);chain.push(current.id);current=nodes.get(current.parent);}chains.set(n.id,chain);}
  let base=nodes.has(scope)?scope:null,roots=children.get(base)||[];
  if(scope==null){const overview=[];for(let root of roots){while(root.kind==='module'&&(children.get(root.id)||[]).length===1&&(children.get(root.id)||[])[0].kind==='module')root=children.get(root.id)[0];const kids=children.get(root.id)||[];overview.push(...(root.kind==='module'&&kids.some(n=>n.kind==='module')?kids:[root]));}roots=overview;}
  const packages=roots.filter(n=>n.kind==='module'),direct=roots.filter(n=>n.kind!=='module');
  const columns=packages.slice();if(direct.length)columns.push({id:'@direct',name:(nodes.get(base)?.name||rootName)+' / '+t('direct files','direkte Dateien'),kind:'files'});
  const packageIds=new Set(packages.map(n=>n.id)),directIds=new Set(direct.map(n=>n.id)),owner=new Map(),included=new Set();
  for(const n of graph.nodes){const chain=chains.get(n.id),id=chain.find(id=>packageIds.has(id));if(id!==undefined){owner.set(n.id,id);included.add(n.id);}else if(directIds.has(n.id)){owner.set(n.id,'@direct');included.add(n.id);}}
  const cells=new Map(),files=graph.nodes.filter(n=>included.has(n.id)&&n.kind==='file'),fileIds=new Set(files.map(n=>n.id));let total=0,totalExact=true,outside=false;
  for(const edge of graph.edges){if(!included.has(edge.from)||!nodes.has(edge.to))continue;const target=owner.get(edge.to)??'@outside';if(target==='@outside')outside=true;total+=edgeWeight(edge);totalExact&&=edge.count!=null;
    for(const id of chains.get(edge.from)){if(!included.has(id))break;if(!cells.has(id))cells.set(id,new Map());const row=cells.get(id);if(!row.has(target))row.set(target,{count:0,exact:true,edges:[]});const cell=row.get(target);cell.count+=edgeWeight(edge);cell.exact&&=edge.count!=null;cell.edges.push(edge);}
  }
  if(outside)columns.push({id:'@outside',name:t('Outside this scope','Außerhalb des Bereichs'),kind:'context'});
  const fileEdges=graph.edges.filter(e=>fileIds.has(e.from)&&fileIds.has(e.to)),adj=new Map(files.map(n=>[n.id,[]]));for(const e of fileEdges)adj.get(e.from).push(e.to);
  const cycleOf=new Map(),badges=new Map(),cycles=findSccs({nodes:files,edges:fileEdges}).map((group,i)=>{
    const members=new Set(group.members),start=group.members[0],next=adj.get(start).find(id=>members.has(id)&&id!==start)??start;let path=[start,start];
    if(next!==start){const parents=new Map([[next,null]]),queue=[next];for(let k=0;k<queue.length&&!parents.has(start);k++)for(const target of adj.get(queue[k]))if(members.has(target)&&!parents.has(target)){parents.set(target,queue[k]);queue.push(target);}
      const tail=[];for(let id=start;id!=null;id=parents.get(id))tail.push(id);path=[start,...tail.reverse()];}
    const cycle={id:'C'+(i+1),members:group.members,path};
    for(const id of group.members){cycleOf.set(id,cycle.id);for(const parent of chains.get(id)){if(!included.has(parent))break;if(!badges.has(parent))badges.set(parent,new Set());badges.get(parent).add(cycle.id);}}
    return cycle;
  });
  return {nodes,children,chains,roots,columns,owner,included,cells,files,total,totalExact,cycles,cycleOf,badges};
}
function treeMatrixRows(model){
  const query=treeMatrixUi.query.trim().toLowerCase(),cycleFilter=treeMatrixUi.cycleOnly&&treeMatrixUi.cycle,filtered=!!query||!!cycleFilter,keep=new Set();
  if(filtered)for(const id of model.included){const node=model.nodes.get(id);if((!query||node.name.toLowerCase().includes(query))&&(!cycleFilter||model.badges.get(id)?.has(cycleFilter)))for(const ancestor of model.chains.get(id))if(model.included.has(ancestor))keep.add(ancestor);}
  const prioritise=list=>list.filter(n=>!filtered||keep.has(n.id)).sort((a,b)=>Number(model.badges.get(b.id)?.has(treeMatrixUi.cycle)||false)-Number(model.badges.get(a.id)?.has(treeMatrixUi.cycle)||false));
  const rows=[],stack=prioritise(model.roots).reverse().map(node=>({node,depth:0}));
  while(stack.length){const row=stack.pop();rows.push(row);if(filtered||treeMatrixUi.expanded.has(row.node.id))for(const node of prioritise(model.children.get(row.node.id)||[]).reverse())stack.push({node,depth:row.depth+1});}
  return rows;
}
function highlightTreeMatrix(box,column,row){
  for(const node of box.querySelectorAll('[data-matrix-column]'))node.classList.toggle('matrix-column-active',node.getAttribute('data-matrix-column')===column);
  for(const node of box.querySelectorAll('tr[data-matrix-row]'))node.classList.toggle('matrix-row-active',node.getAttribute('data-matrix-row')===row);
}

function closeTreeMatrixEvidence(){
  const selection=treeMatrixUi.evidence;treeMatrixUi.evidence=null;renderTreeMatrix();
  if(selection)[...document.getElementById('dsm-tab').querySelectorAll('button[data-matrix-cell]')].find(b=>b.getAttribute('data-matrix-cell')===JSON.stringify([selection.row,selection.column]))?.focus({preventScroll:true});
}
function renderTreeMatrix(){
  const box=document.getElementById('dsm-tab'),state=treeMatrixUi,top=box.scrollTop,left=box.scrollLeft,active=document.activeElement;
  const focusAttribute=['data-matrix-search','data-matrix-toggle','data-matrix-cycle-select','data-matrix-focus','data-matrix-target','data-matrix-cell'].find(key=>active?.getAttribute(key)!=null),focusValue=focusAttribute?active.getAttribute(focusAttribute):null,cursor=active?.selectionStart;
  const revision=viewRevision+':'+state.scope+':'+viewerLanguage;
  if(state.revision!==revision){if(state.sourceRevision!=null&&state.sourceRevision!==viewRevision){state.cycle=null;state.cycleOnly=false;}state.sourceRevision=viewRevision;state.cache=buildTreeMatrix(work,state.scope);state.revision=revision;state.evidence=null;}
  const model=state.cache,cycle=model.cycles.find(c=>c.id===state.cycle),path=cycle?.path||[],pathEdges=new Set(path.slice(1).map((id,i)=>JSON.stringify([path[i],model.owner.get(id)])));
  const rows=treeMatrixRows(model),pageSize=120,width=box.clientWidth||window.innerWidth,columnSize=Math.max(2,Math.min(12,Math.floor((width-(width<=600?150:280)-32)/96))),pages=Math.max(1,Math.ceil(model.columns.length/columnSize));
  if(state.columnSize&&state.columnSize!==columnSize)ui.matrixPage=Math.floor(ui.matrixPage*state.columnSize/columnSize);state.columnSize=columnSize;
  ui.matrixPage=Math.max(0,Math.min(ui.matrixPage,pages-1));state.rowPage=Math.max(0,Math.min(state.rowPage,Math.ceil(rows.length/pageSize)-1));
  const columns=model.columns.slice(ui.matrixPage*columnSize,(ui.matrixPage+1)*columnSize),shown=rows.slice(state.rowPage*pageSize,(state.rowPage+1)*pageSize);
  document.getElementById('matrix-page').textContent=t('Target packages {start}–{end} / {total}','Zielpakete {start}–{end} / {total}',{start:columns.length?ui.matrixPage*columnSize+1:0,end:ui.matrixPage*columnSize+columns.length,total:model.columns.length});
  document.getElementById('matrix-prev').disabled=ui.matrixPage===0;document.getElementById('matrix-next').disabled=ui.matrixPage===pages-1;
  box.textContent='';box.setAttribute('data-tree-matrix-host','true');
  if(!state.bound){box.addEventListener('keydown',event=>{if(event.key==='Escape'&&state.evidence){event.preventDefault();event.stopPropagation();closeTreeMatrixEvidence();}});state.bound=true;}
  if(!state.resizeBound){window.addEventListener('resize',()=>{if(ui.workspace==='dsm'&&work.nodes.some(n=>n.kind==='module'))renderTreeMatrix();});state.resizeBound=true;}
  box.appendChild(element('style',`
    #dsm-tab[data-tree-matrix-host]{position:relative;padding:0 16px 16px}.tree-matrix-toolbar{display:flex;align-items:center;gap:8px;flex-wrap:wrap;margin:14px 0}.tree-matrix-title{margin-right:auto}.tree-matrix-title h2{margin:0 0 8px;font-size:20px}.tree-matrix-title p,.tree-matrix-note{font-size:12px;color:var(--muted);margin:6px 0}
    .tree-matrix-toolbar select{max-width:280px;background:var(--panel);color:var(--text);border:1px solid var(--border);padding:8px;border-radius:6px}.tree-matrix-cycle-path{background:var(--panel-2);padding:14px;border-block:1px solid var(--border);margin:12px 0}.tree-matrix-cycle-path>div{display:flex;align-items:center;gap:8px;flex-wrap:wrap;margin-top:12px}.tree-matrix-cycle-path button{font-size:12px}.tree-matrix-step{color:var(--c-cycle);font-weight:700}
    table.tree-matrix{width:100%;font-size:12px}.tree-matrix th,.tree-matrix td{border-left:0;border-right:0;border-top:0;height:30px}.tree-matrix th:first-child{font-weight:400;min-width:270px;max-width:340px;padding:0 8px}.tree-matrix thead th{min-width:100px;font-size:11px}.tree-matrix .matrix-heading>span{font-weight:400}.tree-matrix td{min-width:90px}.tree-matrix td button{width:auto;min-width:30px;height:22px;border-radius:4px;background:color-mix(in srgb,var(--accent) 12%,transparent);color:var(--accent);padding:2px 7px;font-size:12px}.tree-matrix td button[data-matrix-path-edge]{background:color-mix(in srgb,var(--c-cycle) 12%,transparent);color:var(--c-cycle);border-bottom:2px solid var(--c-cycle)}
    .tree-matrix tr[data-matrix-package] button{font-weight:600}.tree-matrix tr[data-matrix-package]>*{background:var(--panel-2);font-weight:600}.tree-matrix tr[data-matrix-path-row]>th{background:color-mix(in srgb,var(--c-cycle) 12%,var(--panel));border-left:2px solid var(--c-cycle)}.tree-matrix-label{display:flex;align-items:center;gap:7px;min-height:30px}.tree-matrix-label>button{font-weight:400;border:0;background:none;color:var(--text);text-align:left;cursor:pointer;padding:4px 0;overflow-wrap:anywhere}.tree-matrix-label .tree-matrix-toggle{width:18px;flex-shrink:0}.tree-matrix-badge{font-size:10px;color:var(--c-cycle);margin-left:auto;white-space:nowrap}.tree-matrix-evidence{position:fixed;right:16px;bottom:16px;width:min(480px,calc(100vw - 32px));max-height:65vh;overflow:auto;padding:18px;background:var(--panel);border:1px solid var(--border);box-shadow:0 8px 32px #0005;z-index:5;border-radius:12px}.tree-matrix-evidence pre{white-space:pre-wrap;overflow-wrap:anywhere}.tree-matrix-evidence summary{overflow-wrap:anywhere;cursor:pointer}.tree-matrix-evidence h3{overflow-wrap:anywhere}.tree-matrix-evidence header{display:flex;justify-content:space-between;align-items:center}
    #tab-dsm:has([data-tree-matrix]) .matrix-controls>span:first-child{display:none}
    .tree-matrix-toolbar{margin:10px 0;gap:8px}.tree-matrix-title h2{font-size:18px;margin-bottom:4px}.tree-matrix-title p{margin:0}.tree-matrix-search{flex:1;min-width:180px;max-width:360px;padding:9px 12px;background:var(--panel);color:var(--text);border:1px solid var(--border);border-radius:9px}
    .tree-matrix-options{position:relative}.tree-matrix-options>summary{cursor:pointer;padding:8px 12px;border:1px solid var(--border);border-radius:8px}.tree-matrix-options>div{position:absolute;right:0;top:100%;z-index:8;width:280px;max-width:80vw;padding:14px;background:var(--panel);border:1px solid var(--border);border-radius:12px;box-shadow:var(--shadow)}.tree-matrix-options button{margin:0 6px 6px 0}
    .tree-matrix-cycle-path{margin:8px 0;padding:10px 12px;border-radius:9px;font-size:12px}.tree-matrix-cycle-path>div{flex-wrap:nowrap;overflow:auto;margin-top:6px;padding-bottom:3px}.tree-matrix-cycle-path button{white-space:nowrap;flex-shrink:0}
    table.tree-matrix{table-layout:fixed;border-spacing:0}.tree-matrix th:first-child{width:280px;min-width:280px;max-width:280px;z-index:1}.tree-matrix thead th{min-width:0;height:40px;vertical-align:middle}.tree-matrix thead th:first-child{z-index:4}.tree-matrix .matrix-heading{align-items:center}.tree-matrix .matrix-heading>span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.tree-matrix .matrix-copy{opacity:0}.tree-matrix th:hover .matrix-copy,.tree-matrix th:focus-within .matrix-copy{opacity:1}
    table.tree-matrix td,table.tree-matrix th{border-left:0;border-right:0;border-top:0;border-bottom:1px solid color-mix(in srgb,var(--border) 55%,transparent)}.tree-matrix td{min-width:0;color:var(--muted)}.tree-matrix td:empty{background:transparent}.tree-matrix-label>button{white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.tree-matrix-label{min-width:0}.tree-matrix-badge{flex-shrink:0}.tree-matrix .matrix-column-active{background:color-mix(in srgb,var(--accent) 9%,var(--panel))!important}.tree-matrix .matrix-row-active>th{box-shadow:inset 3px 0 var(--accent);background:var(--panel-2)}.tree-matrix td button:focus-visible{outline:2px solid var(--accent);outline-offset:2px}.tree-matrix td button[aria-pressed=true]{outline:2px solid var(--accent)}[data-matrix-focus][aria-pressed=true]{color:var(--c-cycle);border-color:var(--c-cycle)}
    @media(max-width:600px){.tree-matrix th:first-child{min-width:150px;max-width:150px;width:150px}.tree-matrix-label{max-width:134px;padding-left:0!important}.tree-matrix-label>button{min-width:0;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.tree-matrix-badge{max-width:30px;overflow:hidden}.tree-matrix-title h2{font-size:18px}}
  `));
  const toolbar=element('div',undefined,'tree-matrix-toolbar'),title=element('div',undefined,'tree-matrix-title');title.appendChild(element('h2',t('Dependencies in the tree','Abhängigkeiten im Baum')));title.appendChild(element('p',model.files.length+' '+t('files','Dateien')+' · '+(model.totalExact?'':'≥')+model.total+' '+t('dependency evidence records','Abhängigkeitsbelege')+' · '+model.cycles.length+' '+t('file cycle groups','Datei-Zyklusgruppen')));toolbar.appendChild(title);box.appendChild(toolbar);
  const scope=element('select');scope.setAttribute('aria-label',t('Matrix package scope','Matrix-Paketbereich'));scope.setAttribute('data-matrix-scope','true');const overview=element('option',t('Project overview','Projektübersicht'));overview.value='';scope.appendChild(overview);
  for(const n of work.nodes.filter(n=>n.kind==='module').sort((a,b)=>compareNames(a.name,b.name))){const option=element('option',n.name);option.value=String(n.id);scope.appendChild(option);}scope.value=state.scope==null?'':String(state.scope);scope.addEventListener('change',()=>{state.scope=scope.value===''?null:work.nodes.find(n=>String(n.id)===scope.value)?.id;state.expanded.clear();state.rowPage=0;state.cycle=null;state.cycleOnly=false;state.target=null;ui.matrixPage=0;renderTreeMatrix();});toolbar.appendChild(scope);
  const options=element('details',undefined,'tree-matrix-options');options.appendChild(element('summary',t('View','Ansicht')));const optionsBody=element('div');options.appendChild(optionsBody);options.addEventListener('keydown',event=>{if(event.key==='Escape'){options.open=false;options.querySelector('summary').focus();event.stopPropagation();}});toolbar.appendChild(options);
  const expand=button(t('Expand all','Alle öffnen'),()=>{for(const n of work.nodes)if(n.kind==='module')state.expanded.add(n.id);state.rowPage=0;renderTreeMatrix();},'btn btn-secondary');expand.setAttribute('data-matrix-expand-all','true');optionsBody.appendChild(expand);
  optionsBody.appendChild(button(t('Collapse all','Alle schließen'),()=>{state.expanded.clear();state.rowPage=0;renderTreeMatrix();},'btn btn-secondary'));
  const cycleBar=element('div',undefined,'tree-matrix-toolbar');
  const search=element('input',undefined,'tree-matrix-search');search.type='search';search.value=state.query;search.placeholder=t('Find package or file…','Paket oder Datei suchen…');search.setAttribute('aria-label',search.placeholder);search.setAttribute('data-matrix-search','true');search.addEventListener('input',()=>{state.query=search.value;state.rowPage=0;state.resetScroll=true;renderTreeMatrix();});cycleBar.appendChild(search);
  function chooseCycle(id){state.query='';state.cycle=id||null;if(!id)state.cycleOnly=false;state.evidence=null;const c=model.cycles.find(c=>c.id===id);if(c){for(const member of c.members)for(const ancestor of model.chains.get(member))state.expanded.add(ancestor);state.rowPage=Math.floor(treeMatrixRows(model).findIndex(r=>r.node.id===c.path[0])/pageSize);ui.matrixPage=Math.floor(Math.max(0,model.columns.findIndex(col=>col.id===model.owner.get(c.path[1])))/columnSize);}renderTreeMatrix();}
  const cycleSelect=element('select');cycleSelect.setAttribute('data-matrix-cycle-select','true');cycleSelect.setAttribute('aria-label',t('Cycle group','Zyklusgruppe'));const allCycles=element('option',t('All dependencies','Alle Abhängigkeiten'));allCycles.value='';cycleSelect.appendChild(allCycles);for(const c of model.cycles){const option=element('option',c.id+' · '+c.members.length+' '+t('files','Dateien'));option.value=c.id;cycleSelect.appendChild(option);}cycleSelect.value=state.cycle||'';cycleSelect.addEventListener('change',()=>chooseCycle(cycleSelect.value));cycleBar.appendChild(cycleSelect);
  const focus=button(t('Cycle only','Nur dieser Zyklus'),()=>{state.cycleOnly=!state.cycleOnly;state.rowPage=0;state.resetScroll=true;renderTreeMatrix();},'btn btn-secondary');focus.setAttribute('data-matrix-focus','true');focus.setAttribute('aria-pressed',String(state.cycleOnly));focus.disabled=!cycle;cycleBar.appendChild(focus);box.appendChild(cycleBar);
  if(cycle){const strip=element('div',undefined,'tree-matrix-cycle-path');strip.setAttribute('data-matrix-cycle-path',cycle.id);strip.appendChild(element('strong',cycle.id+' · '+t('Example cycle','Beispielzyklus')+' · '+(path.length-1)+' '+t('directed edges · closed','gerichtete Kanten · geschlossen')));const steps=element('div');strip.appendChild(steps);
    path.forEach((id,i)=>{if(i)steps.appendChild(element('span','→','tree-matrix-step'));const n=model.nodes.get(id),b=button((i===path.length-1?1:i+1)+' · '+n.name.split('/').pop(),()=>{for(const ancestor of model.chains.get(id))state.expanded.add(ancestor);state.rowPage=Math.floor(treeMatrixRows(model).findIndex(r=>r.node.id===id)/pageSize);const col=model.columns.findIndex(c=>c.id===model.owner.get(id));ui.matrixPage=Math.floor(Math.max(0,col)/columnSize);renderTreeMatrix();[...box.querySelectorAll('tr[data-matrix-row]')].find(r=>r.getAttribute('data-matrix-row')===String(id))?.scrollIntoView?.({block:'nearest'});},'btn btn-secondary');b.title=n.name;steps.appendChild(b);});box.appendChild(strip);}
  optionsBody.appendChild(element('p',t('Working graph, independent of the graph cut. Source rows → target packages. Parent rows always sum all descendants, including with search or cycle filters; internal dependencies remain visible. Cycle badges refer to file cycles within this scope.','Arbeitsgraph, unabhängig vom Graph-Ausschnitt. Quellzeilen → Zielpakete. Paketzeilen summieren auch bei Suche und Zyklusfilter alle Nachfahren; interne Abhängigkeiten bleiben sichtbar. Zyklusmarkierungen beziehen sich auf Dateizyklen innerhalb dieses Bereichs.'),'tree-matrix-note'));
  const pager=element('div',undefined,'tree-matrix-toolbar');pager.appendChild(element('span',t('Rows {start}–{end} / {total}','Zeilen {start}–{end} / {total}',{start:shown.length?state.rowPage*pageSize+1:0,end:state.rowPage*pageSize+shown.length,total:rows.length})));const prev=button('‹',()=>{state.rowPage--;renderTreeMatrix();},'btn btn-secondary'),next=button('›',()=>{state.rowPage++;renderTreeMatrix();},'btn btn-secondary');prev.disabled=state.rowPage===0;next.disabled=(state.rowPage+1)*pageSize>=rows.length;prev.setAttribute('aria-label',t('Previous rows','Vorherige Zeilen'));next.setAttribute('aria-label',t('Next rows','Nächste Zeilen'));pager.appendChild(prev);pager.appendChild(next);
  const target=element('select');target.setAttribute('data-matrix-target','true');target.setAttribute('aria-label',t('Jump to target package','Zum Zielpaket springen'));const targetPlaceholder=element('option',t('Jump to target…','Zielpaket wählen…'));targetPlaceholder.value='';target.appendChild(targetPlaceholder);for(const col of model.columns){const option=element('option',col.name);option.value=String(col.id);target.appendChild(option);}target.value=state.target||'';target.addEventListener('change',()=>{const index=model.columns.findIndex(c=>String(c.id)===target.value);if(index<0)return;ui.matrixPage=Math.floor(index/columnSize);state.target=target.value;renderTreeMatrix();highlightTreeMatrix(box,state.target,null);});pager.appendChild(target);box.appendChild(pager);
  const table=element('table',undefined,'dsm tree-matrix'+(ui.compact?' compact':''));table.setAttribute('data-tree-matrix','true');const head=element('thead'),header=element('tr');header.appendChild(element('th',t('Tree / source','Baum / Quelle')));
  for(const col of columns){const th=element('th'),heading=element('div',undefined,'matrix-heading'),name=element('span',col.kind==='module'?col.name.split('/').pop():col.name);name.setAttribute('data-matrix-name','true');name.title=col.name;th.scope='col';th.setAttribute('data-matrix-column',col.id);heading.appendChild(name);const copy=button('⧉',()=>copyText(col.name),'btn btn-quiet matrix-copy');copy.setAttribute('data-matrix-copy',col.name);copy.setAttribute('aria-label',t('Copy {name}','{name} kopieren',{name:col.name}));heading.appendChild(copy);th.appendChild(heading);header.appendChild(th);}head.appendChild(header);table.appendChild(head);const body=element('tbody');
  for(const {node,depth}of shown){const tr=element('tr'),th=element('th'),label=element('div',undefined,'tree-matrix-label');tr.setAttribute('data-matrix-row',node.id);if(node.kind==='module')tr.setAttribute('data-matrix-package','true');if(path.includes(node.id))tr.setAttribute('data-matrix-path-row','true');th.scope='row';label.style.paddingLeft=(depth*16)+'px';
    if(node.kind==='module'){const toggle=button(state.expanded.has(node.id)?'⌄':'›',()=>{if(state.expanded.has(node.id))state.expanded.delete(node.id);else state.expanded.add(node.id);renderTreeMatrix();[...box.querySelectorAll('button[data-matrix-toggle]')].find(b=>b.getAttribute('data-matrix-toggle')===String(node.id))?.focus({preventScroll:true});},'tree-matrix-toggle');toggle.setAttribute('data-matrix-toggle',node.id);toggle.setAttribute('aria-expanded',String(!!state.query.trim()||!!(state.cycleOnly&&cycle)||state.expanded.has(node.id)));toggle.disabled=!!state.query.trim()||!!(state.cycleOnly&&cycle);toggle.setAttribute('aria-label',node.name);label.appendChild(toggle);}else label.appendChild(element('span',path.includes(node.id)?String(path.indexOf(node.id)+1):'·',path.includes(node.id)?'tree-matrix-step':'muted'));
    const name=button(node.name.split('/').pop(),()=>{selectNode(node.name);openInspector();},'');name.title=node.name;label.appendChild(name);const badges=[...(model.badges.get(node.id)||[])];if(badges.length)label.appendChild(element('span',badges.slice(0,3).join(' ')+(badges.length>3?' +'+(badges.length-3):''),'tree-matrix-badge'));th.appendChild(label);tr.appendChild(th);
    for(const col of columns){const td=element('td'),cell=model.cells.get(node.id)?.get(col.id);td.setAttribute('data-matrix-column',col.id);if(cell){const b=button((cell.exact?'':'≥')+cell.count,()=>{state.evidence={row:node.id,column:col.id,limit:40};renderTreeMatrix();box.querySelector('section[data-matrix-evidence]')?.focus({preventScroll:true});},'');b.setAttribute('data-matrix-cell',JSON.stringify([node.id,col.id]));b.setAttribute('aria-pressed',String(state.evidence?.row===node.id&&state.evidence?.column===col.id));if(pathEdges.has(JSON.stringify([node.id,col.id])))b.setAttribute('data-matrix-path-edge','true');b.title=node.name+' → '+col.name+' · '+(cell.exact?'':'≥')+cell.count;b.setAttribute('aria-label',b.title);td.appendChild(b);}else{td.textContent='';td.setAttribute('aria-label',t('No recorded dependency','Keine erfasste Abhängigkeit'));}tr.appendChild(td);}body.appendChild(tr);
  }table.appendChild(body);box.appendChild(table);
  for(const tr of body.querySelectorAll('tr[data-matrix-row]'))for(const cell of tr.querySelectorAll('td[data-matrix-column]')){const highlight=()=>highlightTreeMatrix(box,cell.getAttribute('data-matrix-column'),tr.getAttribute('data-matrix-row'));cell.addEventListener('pointerenter',highlight);cell.addEventListener('focusin',highlight);}
  table.addEventListener('pointerleave',()=>highlightTreeMatrix(box,state.target||null,null));
  if(!shown.length)box.appendChild(element('p',t('No matching files or packages. Adjust the search or cycle filter.','Keine passenden Dateien oder Pakete. Suche oder Zyklusfilter anpassen.'),'empty'));
  if(state.evidence){const {row,column,limit}=state.evidence,cell=model.cells.get(row)?.get(column);if(cell){const panel=element('section',undefined,'tree-matrix-evidence');panel.setAttribute('data-matrix-evidence','true');panel.setAttribute('role','dialog');panel.setAttribute('aria-modal','false');panel.setAttribute('tabindex','-1');panel.setAttribute('aria-label',t('Dependency evidence','Abhängigkeitsbelege'));const header=element('header');header.appendChild(element('strong',t('Dependency evidence','Abhängigkeitsbelege')));const close=button('×',closeTreeMatrixEvidence,'btn btn-secondary');close.setAttribute('data-matrix-close','true');close.setAttribute('aria-label',t('Close details','Details schließen'));header.appendChild(close);panel.appendChild(header);panel.appendChild(element('h3',model.nodes.get(row).name+' → '+model.columns.find(c=>c.id===column).name));panel.appendChild(element('p',(cell.exact?'':'≥')+cell.count+' '+t('evidence records','Belege')+' · '+cell.edges.length+' '+t('source connections','Quellverbindungen')));
    for(const edge of cell.edges.slice(0,limit)){const details=element('details');details.appendChild(element('summary',model.nodes.get(edge.from).name+' → '+model.nodes.get(edge.to).name));for(const ref of edge.refs||[])details.appendChild(element('pre',ref.file+':'+ref.line+'\n'+ref.raw));if(!edge.refs?.length)details.appendChild(element('p',t('No source excerpt retained.','Kein Quellausschnitt gespeichert.')));panel.appendChild(details);}if(cell.edges.length>limit)panel.appendChild(button(t('More connections','Weitere Verbindungen'),()=>{state.evidence.limit+=40;renderTreeMatrix();},'btn btn-secondary'));box.appendChild(panel);}}
  box.scrollTop=state.resetScroll?0:top;box.scrollLeft=left;state.resetScroll=false;
  if(focusAttribute){const replacement=[...box.querySelectorAll('['+focusAttribute+']')].find(n=>n.getAttribute(focusAttribute)===focusValue);replacement?.focus({preventScroll:true});if(focusAttribute==='data-matrix-search'&&Number.isInteger(cursor))replacement?.setSelectionRange?.(cursor,cursor);}

}

// DSM tab: the CURRENT view as a matrix table, cyclic cells highlighted.
function matrixOrder() {
  // Same sink-first condensation ordering as CLI dsm_layout; no visual sort.
  const groups=sccs.map(c=>c.members.slice()),seen=new Set(groups.flat());
  for(const n of view.nodes)if(!seen.has(n.id))groups.push([n.id]);
  const groupOf=new Map();groups.forEach((g,i)=>g.forEach(n=>groupOf.set(n,i)));
  const deps=groups.map(()=>new Set());for(const e of view.edges){const a=groupOf.get(e.from),b=groupOf.get(e.to);if(a!==b)deps[a].add(b);}
  const remaining=new Set(groups.map((_,i)=>i)),order=[];
  const key=i=>groups[i].map(id=>nodeById(view,id).name).sort()[0];
  while(remaining.size){const next=[...remaining].filter(i=>![...deps[i]].some(j=>remaining.has(j))).sort((a,b)=>compareNames(key(a),key(b))||a-b)[0];if(next===undefined)break;remaining.delete(next);order.push(...groups[next]);}
  return order.map(id=>nodeById(view,id));
}
function updateMatrixSelection() {
  for(const button of document.querySelectorAll('#dsm-tab button[data-edge]'))button.setAttribute('aria-pressed',String(button.dataset.edge===selectedLinkKey));
}
function renderDsm() {
  if(ui.workspace!=='dsm')return;
  if(work.nodes.some(n=>n.kind==='module')){renderTreeMatrix();return;}
  const revision=viewRevision+':'+ui.matrixPage+':'+ui.compact+':'+viewerLanguage;
  if(matrixRevision===revision){updateMatrixSelection();return;}
  matrixRevision=revision;
  const box=document.getElementById('dsm-tab');box.textContent='';
  const all=matrixOrder(),size=40,tiles=Math.max(1,Math.ceil(all.length/size));
  ui.matrixPage=Math.min(ui.matrixPage,tiles*tiles-1);
  const rowPage=Math.floor(ui.matrixPage/tiles),colPage=ui.matrixPage%tiles;
  const rows=all.slice(rowPage*size,(rowPage+1)*size),columns=all.slice(colPage*size,(colPage+1)*size);
  document.getElementById('matrix-page').textContent=t('Rows {rowStart}–{rowEnd} / columns {columnStart}–{columnEnd} of {count}','Zeilen {rowStart}–{rowEnd} / Spalten {columnStart}–{columnEnd} von {count}',{rowStart:rows.length?rowPage*size+1:0,rowEnd:rowPage*size+rows.length,columnStart:columns.length?colPage*size+1:0,columnEnd:colPage*size+columns.length,count:all.length});
  document.getElementById('matrix-prev').disabled=ui.matrixPage===0;document.getElementById('matrix-next').disabled=ui.matrixPage>=tiles*tiles-1;
  if(!all.length){box.appendChild(element('p',t('No visible nodes. Reset the view to inspect the project.','Keine sichtbaren Knoten. Ansicht zurücksetzen, um das Projekt zu untersuchen.'),'empty'));return;}
  const edges=new Map(view.edges.map(e=>[e.from+' '+e.to,e])),table=element('table',undefined,'dsm'+(ui.compact?' compact':''));
  const caption=element('caption',t('Dependency matrix: source rows → target columns','Abhängigkeitsmatrix: Quellzeilen → Zielspalten'));table.appendChild(caption);
  const head=element('thead'),header=element('tr');header.appendChild(element('th',t('From ↓ / To →','Von ↓ / Nach →')));
  for(const column of columns){
    const th=element('th'),heading=element('div',undefined,'matrix-heading'),name=element('span',column.name);th.scope='col';name.setAttribute('data-matrix-name','true');name.title=column.name;
    const copy=button('⧉',()=>copyText(column.name),'btn btn-quiet matrix-copy');copy.setAttribute('data-matrix-copy',column.name);copy.setAttribute('aria-label',t('Copy {name}','{name} kopieren',{name:column.name}));copy.title=t('Copy full name','Vollständigen Namen kopieren');
    heading.appendChild(name);heading.appendChild(copy);th.appendChild(heading);header.appendChild(th);
  }head.appendChild(header);table.appendChild(head);
  const body=element('tbody');
  for(const row of rows){const tr=element('tr'),th=element('th',row.name);th.scope='row';tr.appendChild(th);
    for(const column of columns){const td=element('td'),key=row.id+' '+column.id,edge=edges.get(key);
      if(row.id===column.id){td.className='diagonal';td.textContent='—';}
      else if(edge){const cyclic=cyclicKeys.has(key);if(cyclic)td.className='dsm-cyclic';
        const cell=button(weightLabel(edge)+(cyclic?' ↻':''),()=>selectEdge(edge),'');cell.dataset.edge=key;
        cell.addEventListener('keydown',event=>{if(event.key==='Enter'){event.preventDefault();event.stopPropagation();selectEdge(edge);openInspector(true);}});
        cell.setAttribute('aria-label',row.name+' → '+column.name+': '+weightLabel(edge)+' '+graphEvidenceUnit()+(cyclic?t(', cyclic',', zyklisch'):''));cell.title=cell.getAttribute('aria-label');
        cell.style.setProperty('--cell-bg','color-mix(in srgb, var(--accent) '+Math.min(28,5+Math.log2(Math.max(1,edgeWeight(edge)))*4)+'%, transparent)');
        cell.setAttribute('aria-pressed',String(selectedLinkKey===key));td.appendChild(cell);
      }else{td.textContent='·';td.setAttribute('aria-label',t('No dependency','Keine Abhängigkeit'));}tr.appendChild(td);
    }body.appendChild(tr);
  }table.appendChild(body);box.appendChild(table);
}

// ── Compiler call-site exploration: separate, immutable source snapshot ─────
// Import aggregation/SimOps do not rewrite compiler-resolved method calls.
let callModel=data.calls||{status:'unavailable',files:0,classes:[],methods:[],calls:[],diagnostics:['Call analysis was not included in this report. Generate a new report or use the live viewer.']};
const callClasses=new Map(),callMethods=new Map();
const callAnalyzers=Array.isArray(callModel.analyzers)?callModel.analyzers:[];
const callUi={language:'all',package:null,query:'',resolution:'project',internal:false,relation:null,element:null,limit:100,siteLimit:100};
const callRelationsCache={key:null,scope:null,rows:[]};
let callClassCycles=[],callModelPrepared=false;
function prepareCallModel() {
  if(callModelPrepared)return;
  try{
    // Decode into a separate outer object so corrupt optional data cannot
    // partially rewrite the retained transport or prevent graph exploration.
    const decoded=decodeCallPayload({...callModel});
    for(const cls of decoded.classes||[])callClasses.set(cls.id,cls);
    for(const method of decoded.methods||[])callMethods.set(method.id,method);
    callClassCycles=findSccs({nodes:[...callClasses.values()].map(c=>({id:c.id,name:c.name})),edges:(decoded.calls||[]).filter(c=>c.resolution==='project'&&c.from_class!==c.to_class).map(c=>({from:c.from_class,to:c.to_class}))});
    // Without a single resolved project target (e.g. Java without a classpath) the
    // project-only default would open the Calls view empty despite recorded sites.
    if((decoded.calls||[]).length&&!(decoded.calls||[]).some(c=>c.resolution==='project'))callUi.resolution='all';
    callModel=decoded;if(data.calls)data.calls=decoded;
  }catch(error){
    callClasses.clear();callMethods.clear();callClassCycles=[];
    callModel={status:'unavailable',files:callModel.files||0,classes:[],methods:[],calls:[],diagnostics:['Call data could not be read: '+String(error.message)]};
  }
  callModelPrepared=true;
}
function callClassLabel(id) {
  const name=callClasses.get(id)?.name;if(name)return name;
  if(!callModelPrepared&&id){
    // Inspector contract labels need two class fields, not every call site.
    // Read those fields without expanding the optional compiler snapshot.
    const compact=callModel.compact,fields=compact?.fields?.classes;
    const pooled=compact?.encoding==='string-pool-v1'&&Array.isArray(compact.strings)&&Array.isArray(fields);
    const cell=(row,field)=>pooled&&fields.includes(field)?compact.strings[row[field]]:row[field];
    const declaration=(callModel.classes||[]).find(cls=>cell(cls,'id')===id);
    const label=declaration&&cell(declaration,'name');if(typeof label==='string')return label;
  }
  if(id?.startsWith('csharp:')){
    const label=id.slice('csharp:'.length);
    return label.startsWith('external:')&&label.includes('::')?label.slice(label.lastIndexOf('::')+2):label;
  }
  return id||t('Unresolved target','Unaufgelöstes Ziel');
}
function callMethodLabel(id) { return callMethods.get(id)?.signature||id||t('initializer','Initialisierer'); }
function callTargetLabel(call,qualified=false) {
  const method=callMethods.get(call.to_method);
  return method?(qualified?callClassLabel(method.class_id)+'#':'')+method.signature:call.target_label||t('Unresolved target','Unaufgelöstes Ziel');
}
function callLanguageLabel(language) { return {go:'Go',java:'Java',csharp:'C#',cs:'C#',ts:'TypeScript',typescript:'TypeScript',js:'JavaScript',javascript:'JavaScript',py:'Python',python:'Python',php:'PHP',rust:'Rust'}[language]||language||t('Unreported language','Sprache nicht angegeben'); }
function callClassLanguage(cls) {
  if(cls?.language)return cls.language==='cs'?'csharp':cls.language;
  if(cls?.file?.endsWith('.java'))return 'java';
  if(cls?.file?.endsWith('.cs'))return 'csharp';
  return callAnalyzers.length===1?callAnalyzers[0].language:Number.isInteger(callModel.java_release)&&callModel.java_release>0?'java':'';
}
function callScopeClasses() { return [...callClasses.values()].filter(cls=>callUi.language==='all'||callClassLanguage(cls)===callUi.language); }
function callResolutionLabel(resolution){return {project:t('project','Projekt'),external:t('external','extern'),unresolved:t('unresolved','unaufgelöst')}[resolution]||resolution;}
function callCoverageLabel(status){return {ready:t('ready','vollständig'),partial:t('partial','teilweise'),unavailable:t('unavailable','nicht verfügbar'),not_applicable:t('not_applicable','nicht anwendbar')}[status]||status;}
function openCallsForNode(name) {
  prepareCallModel();
  const source=nodeByName(work,name),original=source?data.nodes.find(n=>n.id===source.id)?.name:name;
  const declared=[...callClasses.values()].filter(c=>c.graph_node===original);
  const contained=declared.length?declared:[...callClasses.values()].filter(c=>c.graph_node.startsWith(original+'/'));
  const languages=new Set(contained.map(callClassLanguage).filter(Boolean));
  callUi.language=languages.size===1?[...languages][0]:'all';
  const packages=[...new Set(callScopeClasses().map(c=>c.package))].sort(compareNames);
  callUi.package=declared[0]?.package??packages.find(p=>p===original)??packages.find(p=>p.startsWith(original+'/'))??packages[0]??null;
  callUi.query=declared.length===1?declared[0].name:'';callUi.relation=null;callUi.element=declared.length===1?{kind:'class',id:declared[0].id}:null;callUi.limit=100;callUi.siteLimit=100;
  document.getElementById('calls-filter').value=callUi.query;showTab('calls');
}
function callRelations({ignoreElement=false}={}) {
  prepareCallModel();
  const scope=JSON.stringify([callUi.language,callUi.package,callUi.query,callUi.resolution,callUi.internal]),cacheKey=viewerLanguage+scope;
  if(callRelationsCache.key!==cacheKey){
  const q=callUi.query.trim().toLowerCase(),groups=new Map();
  for(const call of callModel.calls||[]){
    const from=callClasses.get(call.from_class),to=callClasses.get(call.to_class);
    if(!from||callUi.package!=='*'&&from.package!==callUi.package&&to?.package!==callUi.package)continue;
    if(callUi.language!=='all'&&callClassLanguage(from)!==callUi.language)continue;
    if(callUi.resolution!=='all'&&call.resolution!==callUi.resolution)continue;
    if(!callUi.internal&&call.from_class===call.to_class)continue;
    if(q&&!([from.name,callClassLabel(call.to_class),callMethodLabel(call.from_method),callMethodLabel(call.to_method),call.target_label].join(' ').toLowerCase().includes(q)))continue;
    const key=JSON.stringify([call.from_class,call.to_class||call.target_label,call.resolution]);
    if(!groups.has(key))groups.set(key,{key,from:call.from_class,to:call.to_class,label:call.target_label,resolution:call.resolution,calls:[]});
    groups.get(key).calls.push(call);
  }
  callRelationsCache.rows=[...groups.values()].sort((a,b)=>compareNames(callClassLabel(a.from),callClassLabel(b.from))||compareNames(callClassLabel(a.to),callClassLabel(b.to))||compareNames(a.key,b.key));
  callRelationsCache.key=cacheKey;
  callRelationsCache.scope=scope;
  }
  if(ignoreElement||!callUi.element)return callRelationsCache.rows;
  return callRelationsCache.rows.map(relation=>({...relation,calls:relation.calls.filter(callMatchesElement)})).filter(relation=>relation.calls.length);
}
function callSuggestions(relation) {
  prepareCallModel();
  if(!relation||relation.resolution!=='project'||relation.from===relation.to)return [];
  const from=callClasses.get(relation.from),to=callClasses.get(relation.to);
  if(!from||!to)return [];
  const sites=(callModel.calls||[]).filter(c=>c.resolution==='project'&&c.from_class===from.id&&c.to_class===to.id);
  const cycle=callClassCycles.find(c=>c.members.includes(from.id)&&c.members.includes(to.id));
  const instance=sites.filter(c=>c.dispatch==='instance'&&!callMethods.get(c.to_method)?.is_static);
  const methods=[...new Set(instance.map(c=>callMethodLabel(c.to_method)))].sort(compareNames);
  const suggestions=[];
  if(cycle&&from.kind!=='module'&&from.graph_node!==to.graph_node&&to.kind==='class'&&methods.length){
    suggestions.push({title:t('Interface candidate: {name}','Interface-Kandidat: {name}',{name:to.name}),summary:t('{from} calls {methods}. Both classes belong to a {count}-class call cycle.','{from} ruft {methods} auf. Beide Klassen gehören zu einem Aufrufzyklus mit {count} Klassen.',{from:from.name,methods:methods.join(', '),count:cycle.members.length}),steps:[
      t('Check whether an existing interface already covers {methods}. Reuse it if appropriate; otherwise consider a narrow contract owned by the consuming responsibility.','Prüfe, ob ein vorhandenes Interface bereits {methods} abdeckt. Verwende es gegebenenfalls wieder; andernfalls erwäge einen schmalen Vertrag im Verantwortungsbereich des Aufrufers.',{methods:methods.join(', ')}),
      t('Let {target} implement that contract and inject it into {source} from the composition root. Keep concrete construction outside the consumer.','Lass {target} diesen Vertrag implementieren und übergib die Implementierung von der zentralen Verdrahtung aus an {source}. Erzeuge konkrete Instanzen außerhalb des Aufrufers.',{target:to.name,source:from.name}),
      t('Review signatures, access modifiers and return/parameter types for dependencies that would preserve the cycle. An interface alone does not remove runtime call cycles; rescan after changing code.','Prüfe Signaturen, Zugriffsmodifikatoren sowie Rückgabe- und Parametertypen auf Abhängigkeiten, die den Zyklus erhalten würden. Ein Interface allein entfernt keine Aufrufzyklen zur Laufzeit; nach der Codeänderung erneut analysieren.')
    ],evidence:instance});
  }else if(cycle&&from.kind!=='module'&&to.kind!=='module'&&from.graph_node!==to.graph_node&&sites.some(c=>c.dispatch==='static'||c.dispatch==='constructor')){
    suggestions.push({title:t('Adapter / factory candidate','Adapter-/Factory-Kandidat'),summary:t('{source} uses static calls or construction of {target} inside a class call cycle.','{source} verwendet statische Aufrufe oder erzeugt {target} innerhalb eines Klassen-Aufrufzyklus.',{source:from.name,target:to.name}),steps:[
      t('Review moving concrete construction or static utility access behind an injected adapter or factory owned by the consumer.','Prüfe, ob die konkrete Objekterzeugung oder statische Hilfsaufrufe hinter einen übergebenen Adapter oder eine Factory im Verantwortungsbereich des Aufrufers verschoben werden können.'),
      t('Choose the contract from the recorded operations; verify lifetime, state and error behavior before changing code. Static methods cannot directly implement an instance interface.','Leite den Vertrag aus den erfassten Operationen ab; prüfe vor der Codeänderung Lebensdauer, Zustand und Fehlerverhalten. Statische Methoden können kein Instanz-Interface direkt implementieren.'),
      t('Rescan after the source change. This proposal does not modify source code or prove that every cycle is removed.','Nach der Quellcodeänderung erneut analysieren. Dieser Vorschlag verändert keinen Quellcode und belegt nicht, dass sämtliche Zyklen entfernt werden.')
    ],evidence:sites});
  }
  const outgoing=(callModel.calls||[]).filter(c=>c.resolution==='project'&&c.from_class===from.id&&c.to_class!==from.id);
  const targetPackageCalls=outgoing.filter(c=>callClasses.get(c.to_class)?.package===to.package);
  const ownPackageCalls=outgoing.filter(c=>callClasses.get(c.to_class)?.package===from.package);
  if(!from.id.includes('@')&&from.package!==to.package&&targetPackageCalls.length>=2&&targetPackageCalls.length>outgoing.length/2&&targetPackageCalls.length>ownPackageCalls.length){
    const sameFile=[...callClasses.values()].filter(c=>c.graph_node===from.graph_node);
    suggestions.push({title:t('Move candidate: {source} → {target}','Verschiebekandidat: {source} → {target}',{source:from.name,target:to.package.replaceAll('/','.')}),summary:t('{target} of {total} recorded outgoing project call sites target this package; {current} target the current package.','{target} von {total} erfassten ausgehenden Projekt-Aufrufstellen zielen auf dieses Paket; {current} auf das aktuelle Paket.',{target:targetPackageCalls.length,total:outgoing.length,current:ownPackageCalls.length}),steps:[
      t('Review whether this type or module belongs to the target package’s responsibility. Call affinity is evidence to inspect, not an architectural decision.','Prüfe, ob dieser Typ oder dieses Modul zum Verantwortungsbereich des Zielpakets gehört. Die Häufung von Aufrufen ist ein Prüfhinweis und keine Architekturentscheidung.'),
      t('Check incoming callers, restricted access, imports and public API compatibility','Prüfe eingehende Aufrufer, eingeschränkte Zugriffsrechte, Imports und die Kompatibilität öffentlicher APIs')+(sameFile.length>1?t('; this file contains {count} type/module elements, so moving the file also affects them.','; diese Datei enthält {count} Typ-/Modulelemente, die beim Verschieben ebenfalls betroffen sind.',{count:sameFile.length}):'.'),
      t('A package move changes grouping, not method calls. Class call cycles remain unless responsibilities or calls also change. Rerun the import and call analyses after a source edit.','Eine Paketverschiebung ändert die Gruppierung, nicht die Methodenaufrufe. Klassen-Aufrufzyklen bleiben erhalten, sofern sich Verantwortlichkeiten oder Aufrufe nicht ebenfalls ändern. Nach einer Quellcodeänderung Import- und Aufrufanalysen erneut ausführen.')
    ],evidence:targetPackageCalls});
  }
  const unresolved=(callModel.calls||[]).filter(c=>c.from_class===from.id&&c.resolution==='unresolved').length;
  if(unresolved)for(const proposal of suggestions)proposal.steps.push(t('{count} unresolved outgoing call sites are excluded from this evidence; resolving them may change the candidate ranking.','{count} unaufgelöste ausgehende Aufrufstellen sind nicht in diesen Belegen enthalten; ihre Auflösung kann die Rangfolge der Kandidaten ändern.',{count:unresolved}));
  else if(callModel.status==='partial')for(const proposal of suggestions)proposal.steps.push(t('Analysis coverage is incomplete. Review compiler diagnostics before acting on this plan.','Die Analyseabdeckung ist unvollständig. Prüfe die Compilerdiagnosen, bevor du diesen Plan umsetzt.'));
  return suggestions;
}
function renderCallInspector() {
  const box=document.getElementById('sidebar');box.textContent='';
  const relation=activeCallRelations().find(r=>r.key===callUi.relation);
  box.appendChild(element('span',t('CALL CONNECTION','AUFRUFVERBINDUNG'),'inspector-eyebrow'));
  if(!relation){if(renderSelectedClassDeclaration(box)||renderSelectedMethodDeclaration(box))return;box.appendChild(element('h2',t('Method calls','Methodenaufrufe')));box.appendChild(element('p',t('Select a type/module connection to inspect its functions and methods and source evidence.','Wähle eine Typ-/Modulverbindung, um ihre Funktionen und Methoden und Quellcode-Belege zu untersuchen.'),'empty'));return;}
  const from=callClasses.get(relation.from),to=callClasses.get(relation.to);
  box.appendChild(element('h2',callClassLabel(relation.from).split('.').pop()+' → '+(relation.to?callClassLabel(relation.to).split('.').pop():relation.label)));
  box.appendChild(element('p',t('{sites} call sites · {methods} target methods','{sites} Aufrufstellen · {methods} Zielmethoden',{sites:relation.calls.length,methods:new Set(relation.calls.map(c=>c.to_method||c.target_label)).size}),'badge'));
  if(relation.resolution==='project'&&from&&to){
    const nodes=[from,...(to.id===from.id?[]:[to])].map(c=>({id:c.id,name:c.name.replaceAll('.','/'),graph_node:c.graph_node}));
    renderDetailGraph(box,{nodes,edges:[{from:from.id,to:to.id}],label:t('Recorded call direction','Erfasste Aufrufrichtung'),onNode:n=>{if(nodeByName(work,n.graph_node)){showTab('graph');revealNode(n.graph_node);}}});
  }
  for(const [isTarget,cls] of [[false,from],[true,to]]){
    const label=isTarget?t('Target','Ziel'):t('Caller','Aufrufer');
    const section=element('section',undefined,'inspector-section');section.appendChild(element('h3',label));
    section.appendChild(element('p',cls?.name||(isTarget?(relation.to?callClassLabel(relation.to):relation.label):callClassLabel(relation.from)),'inspector-full-name'));
    if(callRankingUi.mode==='cycles'&&callCycleUi.kind==='method')section.appendChild(element('p',callCycleLabel('method',isTarget?relation.cycle_to:relation.cycle_from),'path'));
    if(cls){
      section.appendChild(element('p',cls.package?cls.package.replaceAll('/','.'):t('(default package)','(Standardpaket)'),'muted'));
      const language=callClassLanguage(cls);if(language)section.appendChild(element('span',callLanguageLabel(language),'badge'));
      if(cls.kind==='module')section.appendChild(element('p',t('Module owner for top-level functions and initializers; not a declared class.','Modul für freie Funktionen und Initialisierer; keine deklarierte Klasse.'),'muted'));
      if(nodeByName(work,cls.graph_node)){
        const action=button(isTarget?t('Locate target in graph','Ziel im Graphen anzeigen'):t('Locate caller in graph','Aufrufer im Graphen anzeigen'),()=>{showTab('graph');revealNode(cls.graph_node);},'btn btn-secondary');action.setAttribute('data-call-node',cls.graph_node);section.appendChild(action);
      }
    }
    box.appendChild(section);
  }
  if(callRankingUi.mode==='cycles'&&relation.calls.length){
    const first=relation.calls[0],evidence=element('section',undefined,'inspector-section');
    evidence.appendChild(element('h3',t('First call site of this connection','Erste Aufrufstelle dieser Verbindung')));
    evidence.appendChild(element('p',first.file+':'+first.line+':'+first.column,'path'));
    evidence.appendChild(element('pre',first.code));box.appendChild(evidence);
  }
  box.appendChild(element('p',t('Original source snapshot · {resolution} targets. Import simulations do not change these calls.','Ursprünglicher Quellcode-Snapshot · Zielauflösung: {resolution}. Import-Simulationen ändern diese Aufrufe nicht.',{resolution:callResolutionLabel(relation.resolution)}),'muted'));
  if(callModel.status!=='ready')box.appendChild(element('p',t('Call coverage is {status}. Review the analysis details before interpreting missing connections.','Aufrufabdeckung: {status}. Prüfe die Analysedetails, bevor du fehlende Verbindungen interpretierst.',{status:callCoverageLabel(callModel.status)}),'muted'));
}
function callMethodPairKey(call) { return JSON.stringify([call.from_method,call.to_method||call.target_label]); }
function renderCallEvidence(relation) {
  const box=document.getElementById('calls-evidence'),suggestions=document.getElementById('calls-suggestions');
  const openGroups=new Map(box.getAttribute('data-call-relation')===relation?.key?Array.from(box.querySelectorAll('details[data-call-method]'),group=>[group.getAttribute('data-call-method'),group.open]):[]);
  box.textContent='';suggestions.textContent='';box.setAttribute('data-call-relation',relation?.key||'');
  if(!relation){if(renderSelectedClassDeclaration(box)||renderSelectedMethodDeclaration(box))return;box.appendChild(element('p',t("Select a type/module connection to inspect its functions and methods and source call sites.","Wähle eine Typ-/Modulverbindung, um ihre Funktionen und Methoden und Aufrufstellen im Quellcode zu untersuchen."),'empty'));return;}
  box.appendChild(element('h3',callClassLabel(relation.from)+' → '+(relation.to?callClassLabel(relation.to):relation.label)));
  box.appendChild(element('p',relation.calls.length+t(" call sites · "," Aufrufstellen · ")+(relation.resolution==='project'?t("project declaration targets","Ziele mit Deklaration im Projekt"):relation.resolution==='external'?t("external declaration targets","Ziele mit externer Deklaration"):t("unresolved targets; no class or method is assumed","unaufgelöste Ziele; es wird keine Klasse oder Methode angenommen")),'muted'));
  const groups=new Map(),totals=new Map();
  for(const [index,call] of relation.calls.entries()){
    const key=callMethodPairKey(call);totals.set(key,(totals.get(key)||0)+1);
    if(index>=callUi.siteLimit)continue;
    if(!groups.has(key))groups.set(key,[]);groups.get(key).push(call);
  }
  for(const [key,calls] of groups){
    const first=calls[0],group=element('details',undefined,'call-method');group.setAttribute('data-call-method',key);group.open=openGroups.get(key)??(box.querySelectorAll('details[data-call-method]').length===0);
    const summary=element('summary');summary.appendChild(element('strong',callMethodLabel(first.from_method)+' → '+callTargetLabel(first)));
    summary.appendChild(element('span',(calls.length<totals.get(key)?calls.length+' / ':'')+totals.get(key)+t(" call sites"," Aufrufstellen"),'badge'));group.appendChild(summary);
    for(const id of new Set([first.from_method,first.to_method]))appendMethodMetrics(group,id);
    for(const call of calls){
      const row=element('article',undefined,'evidence');row.setAttribute('data-call-site','true');
      row.appendChild(element('div',call.file+':'+call.line+':'+call.column,'path'));
      row.appendChild(element('span',({instance:t('instance','Instanz'),static:t('static','statisch'),constructor:t('constructor','Konstruktor'),unknown:t('unknown','unbekannt')}[call.dispatch]||call.dispatch)+' · '+callResolutionLabel(call.resolution),'badge'));
      if(call.to_method)row.appendChild(element('div',t("Declared target: ","Deklariertes Ziel: ")+callTargetLabel(call,true),'path'));
      row.appendChild(element('pre',call.code));group.appendChild(row);
    }
    box.appendChild(group);
  }
  if(relation.calls.length>callUi.siteLimit)box.appendChild(button(t("Show next 100 call sites","Nächste 100 Aufrufstellen anzeigen"),()=>{
    const next=relation.calls[callUi.siteLimit];callUi.siteLimit+=100;renderCallEvidence(relation);
    const group=Array.from(box.querySelectorAll('details[data-call-method]')).find(node=>node.getAttribute('data-call-method')===callMethodPairKey(next));
    if(group){group.open=true;group.querySelector('summary').focus();}
  }));
  const proposals=callSuggestions(relation);
  suggestions.appendChild(element('h3',t("Refactoring candidates","Refactoring-Kandidaten")));
  suggestions.appendChild(element('p',t("Review plans based on the original source calls. They do not change code or the simulated dependency graph.","Prüfpläne auf Basis der ursprünglichen Quellcode-Aufrufe. Sie verändern weder Code noch den simulierten Abhängigkeitsgraphen."),'muted'));
  if(!proposals.length)suggestions.appendChild(element('p',t("No supported suggestion for this connection. A dependency alone does not establish a misplaced responsibility or justify an interface.","Kein belegter Vorschlag für diese Verbindung. Eine Abhängigkeit allein belegt weder eine falsch zugeordnete Verantwortung noch rechtfertigt sie ein Interface."),'muted'));
  for(const proposal of proposals){
    const card=element('details',undefined,'call-proposal');card.setAttribute('data-call-proposal','true');card.appendChild(element('summary',proposal.title));card.appendChild(element('p',proposal.summary));
    const steps=element('ol');for(const step of proposal.steps)steps.appendChild(element('li',step));card.appendChild(steps);
    const evidence=[...new Set(proposal.evidence.map(c=>c.file+':'+c.line+' → '+callTargetLabel(c,true)))];
    card.appendChild(element('p',t("Evidence: ","Belege: ")+evidence.slice(0,5).join(' · ')+(evidence.length>5?' · '+(evidence.length-5)+t(" more call sites"," weitere Aufrufstellen"):''),'path'));
    card.appendChild(button(t("Copy review plan","Prüfplan kopieren"),()=>copyText([proposal.title,proposal.summary,...proposal.steps,t("Evidence:","Belege:"),...evidence].join('\n'))));suggestions.appendChild(card);
  }
}
function renderCalls() {
  prepareCallModel();
  const status=document.getElementById('calls-status'),list=document.getElementById('calls-relations'),details=document.getElementById('calls-diagnostics');
  list.textContent='';details.textContent='';
  const diagnostics=data.calls?(callModel.diagnostics||[]):[t(callModel.diagnostics[0],'Dieser Bericht enthält keine Aufrufanalyse. Einen neuen Bericht erstellen oder den Live-Viewer verwenden.')];
  const calls=callModel.calls||[],counts={project:0,external:0,unresolved:0};for(const c of calls)counts[c.resolution]=(counts[c.resolution]||0)+1;
  const unavailable=callModel.status==='unavailable',none=callModel.status==='not_applicable';
  const legacyJava=!callAnalyzers.length&&Number.isInteger(callModel.java_release)&&callModel.java_release>0;
  status.textContent=none?t("No applicable source files for call analysis.","Keine geeigneten Quelldateien für die Aufrufanalyse."):unavailable?t("Call analysis unavailable. ","Aufrufanalyse nicht verfügbar. ")+diagnostics.join(' '):(callModel.status==='partial'?(legacyJava?t("Partial Java call analysis; coverage is incomplete. ","Teilweise Java-Aufrufanalyse; die Abdeckung ist unvollständig. "):t("Partial call analysis; coverage is incomplete. ","Teilweise Aufrufanalyse; die Abdeckung ist unvollständig. ")):t("Static call analysis. ","Statische Aufrufanalyse. "))+callModel.files+t(" files · "," Dateien · ")+callClasses.size+t(" types/modules · "," Typen/Module · ")+counts.project+t(" project call sites · "," Projekt-Aufrufstellen · ")+counts.external+t(" external · "," extern · ")+counts.unresolved+t(" unresolved."," unaufgelöst.");
  if(legacyJava)status.textContent+=' Java '+callModel.java_release+t(" API/source baseline."," API-/Quellcode-Basis.");
  else if(!callAnalyzers.length&&!none)status.textContent+=' '+t("Analyzer version unreported.","Analyseversion nicht angegeben.");
  status.textContent+=t(" Original source snapshot"," Ursprünglicher Quellcode-Snapshot")+(ops.length?t("; import simulation is active but does not change these calls.","; die Import-Simulation ist aktiv, ändert jedoch diese Aufrufe nicht."):'.');
  if(!unavailable&&!none)details.appendChild(element('p',status.textContent,'muted'));
  if(!unavailable&&!none)status.textContent=t('Analysis: {status}','Analyse: {status}',{status:callCoverageLabel(callModel.status)})+(ops.length?t(' · original snapshot',' · Original-Snapshot'):'');
  for(const analyzer of callAnalyzers){
    const item=element('article',undefined,'evidence');item.setAttribute('data-call-analyzer',analyzer.language);
    item.appendChild(element('strong',callLanguageLabel(analyzer.language)+' · '+analyzer.backend+' · '+callCoverageLabel(analyzer.status)));
    item.appendChild(element('p',analyzer.files+t(' files',' Dateien')+(analyzer.version?' · '+t('Version ','Version ')+analyzer.version:''),'muted'));
    if(analyzer.project)item.appendChild(element('p',analyzer.project,'path'));
    for(const message of analyzer.diagnostics||[])item.appendChild(element('p',message,'muted'));
    details.appendChild(item);
  }
  for(const message of diagnostics)details.appendChild(element('p',message,'muted'));
  details.hidden=false;
  const languages=[...new Set([...callAnalyzers.map(a=>a.language),...[...callClasses.values()].map(callClassLanguage)].filter(Boolean))].sort(compareNames),languagePicker=document.getElementById('calls-language');
  if(!languages.includes(callUi.language))callUi.language='all';
  languagePicker.textContent='';
  for(const language of ['all',...languages]){const option=element('option',language==='all'?t('All languages','Alle Sprachen'):callLanguageLabel(language));option.value=language;languagePicker.appendChild(option);}
  languagePicker.value=callUi.language;languagePicker.disabled=!languages.length;
  const scopeClasses=callScopeClasses(),packages=[...new Set(scopeClasses.map(c=>c.package))].sort(compareNames),picker=document.getElementById('calls-package');
  if(callUi.package!=='*'&&!packages.includes(callUi.package)){
    const selected=ui.selection?.kind==='node'?nodeByName(work,ui.selection.name):null,original=selected?data.nodes.find(n=>n.id===selected.id)?.name:null;
    callUi.package=scopeClasses.find(c=>c.graph_node===original)?.package??packages.find(p=>p===original)??packages[0]??null;
  }
  picker.textContent='';for(const name of ['*',...packages]){const option=element('option',name==='*'?t('All packages','Alle Pakete'):name.replaceAll('/','.')||t("(default package)","(Standardpaket)"));option.value=name;picker.appendChild(option);}picker.value=callUi.package??'*';picker.disabled=!packages.length;
  document.getElementById('calls-internal').setAttribute('aria-pressed',String(callUi.internal));
  document.getElementById('calls-resolution').value=callUi.resolution;
  const relations=activeCallRelations(),focused=callRankingUi.mode==='cycles'&&callUi.element?relations.filter(r=>r.calls.some(callMatchesElement)):relations;
  const selected=focused.find(r=>r.key===callUi.relation)||focused[0]||relations[0]||null;
  const callSummary=document.getElementById('calls-summary');
  callSummary.textContent=t('{connections} connections · {sites} call sites','{connections} Verbindungen · {sites} Aufrufstellen',{connections:relations.length,sites:relations.reduce((sum,r)=>sum+r.calls.length,0)});
  explainContext(callSummary,relations.length+(callRankingUi.mode==='cycles'?t(' directed connections in the selected cycle group · ',' gerichtete Verbindungen in der gewählten Zyklusgruppe · '):t(" type/module connections · "," Typ-/Modulverbindungen · "))+relations.reduce((sum,r)=>sum+r.calls.length,0)+(callRankingUi.mode==='cycles'?t(' recorded project call sites · complete group across package boundaries',' erfasste Projekt-Aufrufstellen · vollständige Gruppe über Paketgrenzen hinweg'):callUi.package==='*'?t(' matching call sites · all packages',' passende Aufrufstellen · alle Pakete'):t(" matching call sites · includes callers or targets in the selected package"," passende Aufrufstellen · enthält Aufrufer oder Ziele im gewählten Paket")));
  callUi.relation=selected?.key??null;
  if(callRankingUi.mode!=='cycles'){
  if(selected)callUi.limit=Math.max(callUi.limit,Math.ceil((relations.indexOf(selected)+1)/100)*100);
  for(const relation of relations.slice(0,callUi.limit)){
    const from=callClassLabel(relation.from).split('.').pop(),to=relation.to?callClassLabel(relation.to).split('.').pop():relation.label;
    const row=button(from+' → '+to,()=>{callUi.relation=relation.key;callUi.siteLimit=100;renderCalls();document.getElementById('calls-evidence').focus();},'explorer-row call-relation');
    row.setAttribute('data-relation',relation.key);row.setAttribute('aria-pressed',String(relation.key===callUi.relation));
    const language=callClassLanguage(callClasses.get(relation.from));if(language)row.appendChild(element('span',callLanguageLabel(language),'badge'));
    row.appendChild(element('small',relation.calls.length+t(" call sites · "," Aufrufstellen · ")+new Set(relation.calls.map(c=>c.to_method||c.target_label)).size+t(" target methods · "," Zielmethoden · ")+callResolutionLabel(relation.resolution)));
    row.appendChild(element('small',callClassLabel(relation.from)+' → '+(relation.to?callClassLabel(relation.to):relation.label)));list.appendChild(row);
  }
  if(!relations.length)list.appendChild(element('p',unavailable||none?t("No call relations available.","Keine Aufrufbeziehungen verfügbar."):t("No matching call relations. Change the package, search, target filter or internal-call setting.","Keine passenden Aufrufbeziehungen. Ändere das Paket, die Suche, den Zielfilter oder die Einstellung für interne Aufrufe."),'empty'));
  if(relations.length>callUi.limit)list.appendChild(button(t("Show next 100 type/module connections","Nächste 100 Typ-/Modulverbindungen anzeigen"),()=>{callUi.limit+=100;renderCalls();}));
  }
  renderCallTree(activeCallRelations({ignoreElement:true}),selected);
  renderCallRankings();
  renderCallCycles();
  renderCallEvidence(selected);
  if(ui.workspace==='calls')renderSidebar();
}

function renderOpsStrip() {
  if(isRepositoryOverview()){
    for(const id of ['ops-strip','simulation-indicator','repair-result'])document.getElementById(id).hidden=true;
    for(const id of ['ops-list','simulation-summary','simulation-indicator','repair-result'])document.getElementById(id).textContent='';
    for(const id of ['undo-ops','clear-ops','export-ops'])document.getElementById(id).disabled=true;
    document.getElementById('simulation-mode').setAttribute('aria-pressed','false');
    document.getElementById('refactor-summary').setAttribute('data-active','false');
    return;
  }
  const indicator=document.getElementById('simulation-indicator');
  indicator.hidden=!ui.simulation&&!ops.length;
  indicator.textContent=ui.simulation?t('Simulation active','Simulation aktiv'):t('{count} changes','{count} Änderungen',{count:ops.length});
  document.getElementById('refactor-summary').setAttribute('data-active',String(!indicator.hidden));
  const strip=document.getElementById('ops-strip');strip.hidden=!ui.simulation&&!ops.length;
  if(strip.hidden)hideMetricTooltip(strip);
  const ul=document.getElementById('ops-list');ul.textContent='';
  for(const [i,op] of ops.entries())ul.appendChild(element('li',(i+1)+'. '+(op.cut_edge?t('Cut {from} → {to}','Trenne {from} → {to}',op.cut_edge):op.move_node?t('Move {node} under {new_parent}','Verschiebe {node} unter {new_parent}',op.move_node):t('Merge {absorbed} into {survivor}','Führe {absorbed} in {survivor} zusammen',op.merge))));
  document.getElementById('simulation-summary').textContent=t('{count} changes · current scope cycles {before} → {after} · ACD ','{count} Änderungen · Zyklen im aktuellen Umfang {before} → {after} · ACD ',{count:ops.length,before:baselineStats.cycles,after:sccs.length})+baselineStats.acd.toFixed(2)+' → '+currentAcd.toFixed(2)+(ui.simulation?t(' · Simulation mode',' · Simulationsmodus'):t(' · Preserved while browsing',' · Beim Erkunden beibehalten'));
  explainMetric(document.getElementById('simulation-summary'),'simulation',metricHelp('acd'));
  const repairResult=document.getElementById('repair-result');repairResult.hidden=!appliedRepair;
  repairResult.textContent=appliedRepair?t('Repair preview · ','Lösungsvorschau · ')+repairStats(t('Selected scope','Ausgewählter Umfang'),appliedRepair.before,appliedRepair.plan.after)+'. '+repairStats(t('Source dependency graph','Abhängigkeitsgraph der Quellen'),appliedRepair.sourceBefore,appliedRepair.plan.sourceAfter)+t('. Source changes still require implementation and verification.','. Änderungen am Quellcode müssen weiterhin umgesetzt und geprüft werden.'):'';
  for(const id of ['undo-ops','clear-ops','export-ops'])document.getElementById(id).disabled=!ops.length;
}

// "F17 direction/width legend": one sample line per reference count on
// the documented scale (1, 2, 5, 25, 100 — 100 already hits the 8 px cap).
// Each row is ONE directed bundle: thickness = widthFor(count); A→B and
// B→A are separate mirrored curves and only directions present in the
// graph appear. Built with createElement/textContent only (F01 rule: no
// innerHTML with data-derived text; here even the texts are static).
function buildLegend() {
  const box = document.getElementById('edge-legend');
  box.textContent = '';
  box.appendChild(element('div',t('→ arrows point from source to target','→ Pfeile zeigen von der Quelle zum Ziel')));
  const reference=directionReference();
  if(reference){
    const context=document.createElement('div'),fullName=reference.name,name=fullName.split('/').pop();
    context.textContent=(ui.selection?.kind==='node'?t('Selected: ','Ausgewählt: '):ui.selection?.kind==='edge'?t('Reference: ','Bezug: '):t('Inside: ','Innerhalb von: '))+(name.length>28?name.slice(0,26)+'…':name);context.title=fullName;box.appendChild(context);
    const row=document.createElement('div');row.className='leg-row';
    for(const [text,color] of [[t('← Incoming','← Eingehend'),C.incoming],[t('Outgoing →','Ausgehend →'),C.outgoing]]){
      const label=document.createElement('span');label.textContent=text;label.style.color=color;row.appendChild(label);
    }
    box.appendChild(row);
  }
  const stateHead=document.createElement('div');stateHead.textContent=t('Lines and arrowheads:','Linien und Pfeilspitzen:');box.appendChild(stateHead);
  const states=document.createElement('div');states.className='leg-row';
  for(const [text,color,show] of [[t('▲ cycle','▲ Zyklus'),C.cycle,true],[t('▲ resolved','▲ aufgelöst'),C.resolved,resolvedKeys.size],[t('▲ new cycle','▲ neuer Zyklus'),C.introduced,introducedKeys.size]])if(show){
    const label=document.createElement('span');label.textContent=text;label.style.color=color;states.appendChild(label);
  }
  box.appendChild(states);
  box.appendChild(element('div',t('Red: current-scope cycles. Dashed: coarser package-level cycle context; direction colors are retained.','Rot: Zyklen im aktuellen Umfang. Gestrichelt: gröberer Zykluskontext auf Paketebene; Richtungsfarben bleiben erhalten.')));
  if(ui.highlightPriorities){const priority=element('div',t('P1 ≥40 · P2 ≥20 review priority','P1 ≥40 · P2 ≥20 Prüfpriorität'));priority.style.color=C.priority;priority.title=metricHelp('priority');box.appendChild(priority);}
  const head = document.createElement('div');
  head.textContent = graphEvidenceUnit()+t(' per direction',' je Richtung');
  box.appendChild(head);
  for (const n of [1, 2, 5, 25, 100]) {
    const row = document.createElement('div');
    row.className = 'leg-row';
    const line = document.createElement('span');
    line.className = 'leg-line';
    line.style.borderTopWidth = widthFor(n) + 'px';
    line.style.borderTopColor = C.edge;
    row.appendChild(line);
    const label = document.createElement('span');
    label.textContent = String(n) + (n === 100 ? t(' (cap 8px)',' (max. 8px)') : '');
    row.appendChild(label);
    box.appendChild(row);
  }
}

function renderAll(fitGraph=true) {
  enforceRepositoryScope();renderRepositoryNavigation();
  if(isRepositoryOverview()){
    renderBreadcrumb();renderSummary();renderWorkspaceDescription();
    if(ui.workspace==='start')renderStart();else if(ui.workspace==='security')renderSecurity();else renderServices();
    renderSidebar();renderOpsStrip();return;
  }
  if(ui.workspace==='graph')renderGraph(fitGraph);
  renderBreadcrumb();renderSummary();renderSidebar();renderExplorer();renderCyclesPanel();renderReviewPanel();
  if(ui.workspace==='start')renderStart();
  if(ui.workspace==='dsm')renderDsm();
  if(ui.workspace==='calls')renderCalls();
  if(ui.workspace==='services')renderServices();
  if(ui.workspace==='architecture'&&typeof renderArchitecture==='function')renderArchitecture();
  if(ui.workspace==='security')renderSecurity();
  if(ui.workspace==='review-dashboard'&&typeof renderReviewDashboard==='function')renderReviewDashboard();
  if(ui.workspace==='quality')renderEngineeringQuality();
  if(ui.workspace==='assessment')renderAssessment();
  renderOpsStrip();
}

// Navigation and simulation are explicit, keyboard-accessible DOM controls.
function showExplorer(name) {
  if(isRepositoryOverview())return;
  hideMetricTooltip();
  ui.explorer=name;
  document.getElementById('modules-panel').classList.toggle('panel-hidden',name!=='modules');
  document.getElementById('tab-cycles').classList.toggle('panel-hidden',name!=='cycles');
  document.getElementById('review-panel').classList.toggle('panel-hidden',name!=='review');
  for(const b of document.querySelectorAll('#explorer-tabs button'))b.setAttribute('aria-selected',String(b.dataset.explorer===name));
}
function renderWorkspaceDescription() {
  if(isRepositoryOverview()){
    document.getElementById('workspace-description').textContent=ui.workspace==='security'?
      t('Repository-wide dependency scan · source projects are analyzed separately','Repository-weiter Abhängigkeitsscan · Quellprojekte werden getrennt analysiert'):
      t('Repository projects and deployments · configuration evidence only · open a project for source analysis','Repository-Projekte und Deployments · nur Konfigurationsbelege · für Quellanalyse ein Projekt öffnen');
    return;
  }
  const text=ui.workspace==='start'?t('Explore the architecture, inspect quality and verify changes using the same evidence as MCP.','Architektur erkunden, Qualität prüfen und Änderungen anhand derselben Belege wie im MCP bewerten.'):ui.workspace==='assessment'?t('Recorded architecture review · findings, decisions, change impact and evidence gaps','Gespeichertes Architekturreview · Befunde, Entscheidungen, Änderungsreichweite und Beleglücken'):ui.workspace==='quality'?t('Original source measurements · package cohesion, history, test impact and mutation evidence','Ursprüngliche Quellmessungen · Paketkohäsion, Verlauf, Testreichweite und Mutationsbelege'):ui.workspace==='security'?t('Original dependency scan snapshot · graph simulations do not change security findings','Ursprünglicher Abhängigkeitsscan · Graphsimulationen verändern Sicherheitsbefunde nicht'):
    ui.workspace==='review-dashboard'?t('Review KPIs from the analysis pipeline · read-only; decisions persist only through the serve API','Review-Kennzahlen aus der Analyse-Pipeline · schreibgeschützt; Entscheidungen werden nur über die serve-API gespeichert'):
    ui.workspace==='architecture'?t('Declared target architecture · observed full-source dependency boundaries','Deklarierte Sollarchitektur · beobachtete Abhängigkeitsgrenzen des vollständigen Quellgraphen'):
    ui.workspace==='services'?t('Service configuration · top metrics and simulation describe the dependency graph','Dienstkonfiguration · obere Kennzahlen und Simulation beschreiben den Abhängigkeitsgraphen'):
    ui.workspace==='calls'?t('Source call sites · types, modules and functions/methods','Aufrufstellen · Typen, Module und Funktionen/Methoden'):
    ui.workspace==='dsm'?t('Dependencies · source rows → target columns','Abhängigkeiten · Quellzeilen → Zielspalten'):
    ui.diagram==='network'?t('Complete dependency network · use Tree for routed dependency arrows','Vollständiges Abhängigkeitsnetz · der Baum zeigt angeordnete Abhängigkeitspfeile'):
    ui.diagram==='overview'?t('Exploration tree · relations on a separate axis · + / − expands branches','Strukturbaum · Beziehungen auf einer eigenen Achse · + / − öffnet Zweige'):
    t('Incoming → selected package → outgoing · each connection has source evidence','Eingehend → ausgewähltes Paket → ausgehend · jede Verbindung hat Quellbelege');
  document.getElementById('workspace-description').textContent=text;
}
function showTab(name) {
  if(isRepositoryOverview()&&!['start','services','security'].includes(name))name='start';
  enforceRepositoryScope();
  hideMetricTooltip();
  hideContextTooltip();
  hideServiceTooltip();
  if(name==='cycles'){
    showExplorer('cycles');name='graph';
    document.getElementById('main').classList.remove('explorer-closed');
    document.getElementById('toggle-explorer').setAttribute('aria-expanded','true');
    if(window.innerWidth<=1100){document.getElementById('main').classList.add('inspector-closed');document.getElementById('toggle-inspector').setAttribute('aria-expanded','false');}
  }
  // Navigation already renders its final scope. Re-selecting the active Graph
  // tab must not rebuild the tree once more with the preceding scope.
  if(name==='graph'&&ui.workspace==='graph')return;
  ui.workspace=name==='start'?'start':name==='assessment'?'assessment':name==='quality'?'quality':name==='security'?'security':name==='review-dashboard'?'review-dashboard':name==='architecture'?'architecture':name==='services'?'services':name==='calls'?'calls':name==='dsm'?'dsm':'graph';
  document.documentElement.classList.toggle('report-workspace',['start','quality','architecture','assessment','security','review-dashboard'].includes(ui.workspace));
  document.documentElement.classList.toggle('context-workspace',['calls','services','dsm'].includes(ui.workspace));
  if(ui.workspace!=='graph')closeWorkspaceMenu('graph-display-menu');
  renderWorkspaceDescription();
  document.getElementById('graph-options').hidden=ui.workspace!=='graph';
  for(const id of ['simulation-mode','repair-cycle'])document.getElementById(id).disabled=ui.workspace==='assessment'||ui.workspace==='quality'||ui.workspace==='services'||ui.workspace==='architecture'||ui.workspace==='security'||ui.workspace==='review-dashboard';
  for(const id of ['tab-start','tab-graph','tab-dsm','tab-calls','tab-services','tab-architecture','tab-security','tab-quality','tab-assessment','tab-review-dashboard'])document.getElementById(id).classList.toggle('panel-hidden',id!=='tab-'+ui.workspace);
  for(const b of document.querySelectorAll('#tabs button')){b.classList.toggle('active',b.dataset.tab===ui.workspace);b.setAttribute('aria-selected',String(b.dataset.tab===ui.workspace));}
  renderRepositoryNavigation();
  if(ui.workspace==='start')renderStart();else if(ui.workspace==='assessment')renderAssessment();else if(ui.workspace==='quality')renderEngineeringQuality();else if(ui.workspace==='security')renderSecurity();else if(ui.workspace==='review-dashboard'){if(typeof renderReviewDashboard==='function')renderReviewDashboard();}else if(ui.workspace==='architecture'){if(typeof renderArchitecture==='function')renderArchitecture();}else if(ui.workspace==='services')renderServices();else if(ui.workspace==='dsm')renderDsm();else if(ui.workspace==='calls')renderCalls();else {renderGraph(false);setWidth();}
  renderSidebar();
  syncViewerNavigation();
}
document.getElementById('calls-language').addEventListener('change',event=>{callUi.language=event.target.value;callUi.relation=null;callUi.element=null;callUi.limit=100;callUi.siteLimit=100;renderCalls();});
document.getElementById('calls-package').addEventListener('change',event=>{callUi.package=event.target.value;callUi.relation=null;callUi.element=null;callUi.limit=100;callUi.siteLimit=100;renderCalls();});
document.getElementById('calls-tree-expand').addEventListener('click',()=>expandCallTree(true));
document.getElementById('calls-tree-collapse').addEventListener('click',()=>expandCallTree(false));
document.getElementById('calls-tree-clear').addEventListener('click',()=>{callUi.element=null;callUi.limit=100;callUi.siteLimit=100;renderCalls();});
document.getElementById('calls-filter').addEventListener('input',event=>{callUi.query=event.target.value;callUi.limit=100;callUi.siteLimit=100;renderCalls();});
document.getElementById('calls-resolution').addEventListener('change',event=>{callUi.resolution=event.target.value;callUi.relation=null;callUi.limit=100;callUi.siteLimit=100;renderCalls();});
document.getElementById('calls-internal').addEventListener('click',()=>{callUi.internal=!callUi.internal;callUi.limit=100;callUi.siteLimit=100;renderCalls();});
for(const b of document.querySelectorAll('#tabs button'))b.addEventListener('click',()=>showTab(b.dataset.tab));
for(const b of document.querySelectorAll('#explorer-tabs button'))b.addEventListener('click',()=>showExplorer(b.dataset.explorer));
for(const mode of ['overview','dependencies','network'])document.getElementById('diagram-'+mode).addEventListener('click',()=>{closeWorkspaceMenu('graph-display-menu',true);setDiagram(mode);});
for(const b of document.querySelectorAll('#edge-modes button'))b.addEventListener('click',()=>{ui.edgeMode=b.dataset.edges;ui.edgeModeAutomatic=false;refreshSelection();});
document.getElementById('toggle-cycles').addEventListener('click',()=>{
  ui.cyclesOnly=!ui.cyclesOnly;
  if(!edgeReference()&&ui.edgeModeAutomatic)ui.edgeMode='all';
  hideContextTooltip();refreshSelection();
});
document.getElementById('toggle-priorities').addEventListener('click',()=>{ui.highlightPriorities=!ui.highlightPriorities;hideContextTooltip();refreshSelection();renderExplorer();});
document.getElementById('review-open').addEventListener('click',()=>{
  closeWorkspaceMenu('graph-display-menu',true);
  showExplorer('review');renderReviewPanel();document.getElementById('main').classList.remove('explorer-closed');document.getElementById('toggle-explorer').setAttribute('aria-expanded','true');
  if(window.innerWidth<=1100){document.getElementById('main').classList.add('inspector-closed');document.getElementById('toggle-inspector').setAttribute('aria-expanded','false');}
  setWidth();document.getElementById('review-orphans').focus();
});
document.getElementById('review-orphans').addEventListener('click',()=>{ui.reviewOrphans=!ui.reviewOrphans;ui.reviewLimit=20;renderReviewPanel();});
document.getElementById('toggle-files').addEventListener('click',()=>{
  ui.showFiles=!ui.showFiles;if(!ui.showFiles){revealedGraphFiles.clear();pinSelectedGraphFiles();}
  renderGraph();
});
let searchTimer=null;
for(const action of ['expand','collapse'])document.getElementById('explorer-'+action+'-all').addEventListener('click',()=>{
  setAllTreeBranches(action==='expand');
  const next=document.getElementById('explorer-'+(action==='expand'?'collapse':'expand')+'-all');if(!next.disabled)next.focus({preventScroll:true});
});
function applySearch(){ui.searchLimit=100;ui.reviewLimit=20;renderExplorer();renderCyclesPanel();renderReviewPanel();renderGraph();}
document.getElementById('filter').addEventListener('input',()=>{clearTimeout(searchTimer);searchTimer=setTimeout(applySearch,120);});
document.getElementById('clear-search').addEventListener('click',()=>{document.getElementById('filter').value='';applySearch();document.getElementById('filter').focus();});
document.getElementById('cycles-reset').addEventListener('click',clearCycle);
document.getElementById('fit-view').addEventListener('click',fitView);
document.getElementById('reset-view').addEventListener('click',resetView);
document.getElementById('zoom-in').addEventListener('click',()=>{if(fg)fg.zoom(fg.zoom()*1.25,reducedMotion?0:150);});
document.getElementById('zoom-out').addEventListener('click',()=>{if(fg)fg.zoom(fg.zoom()/1.25,reducedMotion?0:150);});
// The browser fullscreen covers the whole workspace, not just one canvas.
function workspaceFullscreenToggle(button){
  if(document.fullscreenElement||document.webkitFullscreenElement){
    const exit=document.exitFullscreen||document.webkitExitFullscreen;
    if(exit)exit.call(document);
    button.setAttribute('aria-pressed','false');return;
  }
  const root=document.documentElement,enter=root.requestFullscreen||root.webkitRequestFullscreen;
  if(!enter){feedback(t('Fullscreen is not available in this browser.','Vollbild ist in diesem Browser nicht verfügbar.'));return;}
  button.setAttribute('aria-pressed','true');
  Promise.resolve(enter.call(root)).catch(()=>{button.setAttribute('aria-pressed','false');feedback(t('The browser blocked fullscreen.','Der Browser hat Vollbild blockiert.'));});
}
// Esc and other browser exits must keep every toggle honest.
function syncFullscreenToggles(){
  const active=String(!!(document.fullscreenElement||document.webkitFullscreenElement));
  for(const id of ['toggle-fullscreen','world-fullscreen'])document.getElementById(id).setAttribute('aria-pressed',active);
}
document.getElementById('toggle-fullscreen').addEventListener('click',event=>workspaceFullscreenToggle(event.currentTarget||event.target));
document.addEventListener('fullscreenchange',syncFullscreenToggles);
for(const panel of ['explorer','inspector'])document.getElementById('toggle-'+panel).addEventListener('click',()=>{
  if(panel==='inspector'){setInspectorOpen(!inspectorIsOpen(),true);return;}
  if(panel==='explorer'&&isRepositoryOverview())return;
  const main=document.getElementById('main'),closed=main.classList.toggle(panel+'-closed');
  document.getElementById('toggle-'+panel).setAttribute('aria-expanded',String(!closed));
  if(!closed&&window.innerWidth<=1100){const other=panel==='explorer'?'inspector':'explorer';main.classList.add(other+'-closed');document.getElementById('toggle-'+other).setAttribute('aria-expanded','false');}
  setWidth();
});
document.getElementById('simulation-mode').addEventListener('click',()=>{closeWorkspaceMenu('refactor-menu',true);setSimulation(!ui.simulation);});
document.getElementById('repair-cycle').addEventListener('click',()=>{
  closeWorkspaceMenu('refactor-menu',true);
  showTab('cycles');
  const cycle=ui.cycle&&cyclesForView().find(c=>cycleKey(c)===ui.cycle);
  if(cycle)compareCycleSolutions(cycle);
  else feedback(t('Choose a cycle in the Explorer, then Compare solutions to review low-change alternatives.','Wähle im Explorer einen Zyklus und dann „Lösungen vergleichen“, um Alternativen mit wenigen Änderungen zu prüfen.'));
});
document.getElementById('cancel-action').addEventListener('click',closeDialog);
document.getElementById('action-dialog').addEventListener('cancel',()=>{ui.pending=null;if(typeof cancelScenarioAction==='function')cancelScenarioAction();});
document.getElementById('confirm-action').addEventListener('click',()=>{if(typeof confirmScenarioAction==='function'&&confirmScenarioAction())return;const op=ui.pending,repair=pendingRepair;closeDialog();if(op)recordOp(op,repair);});
document.getElementById('undo-ops').addEventListener('click',()=>{ops.splice(Math.max(0,ops.length-(opBatches.pop()||1)));appliedRepair=null;ui.cycle=null;recompute();renderAll();feedback(t('Last simulated plan or change undone.','Letzter simulierter Plan oder Änderung rückgängig gemacht.'));});
document.getElementById('clear-ops').addEventListener('click',()=>{ops=[];opBatches=[];appliedRepair=null;ui.cycle=null;recompute();renderAll();feedback(t('All simulated changes cleared.','Alle simulierten Änderungen gelöscht.'));});
document.getElementById('export-ops').addEventListener('click',()=>{
  const blob=new Blob([JSON.stringify(ops)],{type:'application/json'}),a=document.createElement('a');a.href=URL.createObjectURL(blob);a.download='archguard-simops.json';a.click();URL.revokeObjectURL(a.href);feedback(t('Exported simulated changes. Source files were not edited.','Simulierte Änderungen exportiert. Quelldateien wurden nicht bearbeitet.'));
});
document.getElementById('matrix-density').addEventListener('click',()=>{ui.compact=!ui.compact;document.getElementById('matrix-density').setAttribute('aria-pressed',String(ui.compact));renderDsm();});
document.getElementById('matrix-prev').addEventListener('click',()=>{ui.matrixPage=Math.max(0,ui.matrixPage-1);renderDsm();});
document.getElementById('matrix-next').addEventListener('click',()=>{ui.matrixPage+=1;renderDsm();});
document.addEventListener('keydown',event=>{
  if(event.defaultPrevented)return;
  const modal=document.getElementById('action-dialog').open||document.getElementById('scenario-dialog').open||kpiHelpUi.dialog?.open;
  if(event.key==='Escape'&&metricTooltipTarget){hideMetricTooltip();event.preventDefault();return;}
  if(event.key==='Enter'&&!modal&&ui.selection&&['graph','dsm'].includes(ui.workspace)){
    const target=event.target,graph=document.getElementById('graph'),tree=document.getElementById('package-tree');
    if(!target||target===document.body||target===tree||target===graph||target.tagName==='CANVAS'&&graph.contains(target)){openInspector(true);event.preventDefault?.();return;}
  }
  if(!modal&&ui.workspace==='start'){
    if(event.key==='Escape'&&worldUi.detailsOpen){setWorldDetailsOpen(false,true);event.preventDefault?.();return;}
    if(event.key==='Enter'&&worldUi.selection&&(!event.target||event.target===document.body||event.target===document.getElementById('world-canvas'))){setWorldDetailsOpen(true,true);event.preventDefault?.();return;}
  }
  if(event.key==='Escape'&&!modal&&inspectorIsOpen()&&['graph','dsm','calls','services'].includes(ui.workspace)){setInspectorOpen(false,true);event.preventDefault?.();return;}
  if(event.key==='Escape'&&!document.getElementById('action-dialog').open&&!document.getElementById('scenario-dialog').open){
    const openMenu=Object.keys(WORKSPACE_MENUS).find(id=>document.getElementById(id).open);
    if(openMenu){closeWorkspaceMenu(openMenu,true);event.preventDefault();return;}
  }
  if(event.key==='Escape')hideContextTooltip();
  if(event.key==='Escape'&&!document.getElementById('action-dialog').open&&!document.getElementById('scenario-dialog').open){ui.selection=null;reconcileSelection();renderSidebar();refreshSelection();}
});
for(const id of Object.keys(WORKSPACE_MENUS))document.getElementById(id).addEventListener('toggle',()=>{
  if(document.getElementById(id).open)for(const other of Object.keys(WORKSPACE_MENUS))if(other!==id)closeWorkspaceMenu(other);
});
document.addEventListener('pointerdown',event=>{
  for(const id of Object.keys(WORKSPACE_MENUS))if(!document.getElementById(id).contains(event.target))closeWorkspaceMenu(id);
});
function setWidth() {
  syncPanelSizes();
  hideMetricTooltip();
  hideContextTooltip();
  if(ui.workspace==='graph'&&ui.diagram==='overview'&&treeViewportWidth!==treeAvailableWidth(document.getElementById('package-tree')))renderPackageTree();
  if(ui.workspace==='services'&&serviceUi.viewportWidth!==treeAvailableWidth(document.getElementById('services-tree')))renderServices();
  if(!fg||ui.workspace!=='graph')return;
  const box=document.getElementById('tab-graph').getBoundingClientRect();fg.width(Math.max(1,box.width)).height(Math.max(1,box.height));
  fitWhenReady();
}
window.addEventListener('resize',setWidth);
if(typeof ResizeObserver==='function'){
  const workspaceResizeObserver=new ResizeObserver(setWidth);
  for(const id of ['viewport','package-tree','services-tree'])workspaceResizeObserver.observe(document.getElementById(id));
}
for(const b of document.querySelectorAll('#theme-switch button'))b.addEventListener('click',()=>applyTheme(b.dataset.mode));
if(mediaDark.addEventListener)mediaDark.addEventListener('change',()=>{if(themePref==='system')applyTheme('system');});
// Navigation URLs contain report-relative names, never ephemeral graph IDs.
const viewerNavigation={ready:false,applying:false,last:'',button:null};
function parseViewerRoute(hash){
  if(!hash||hash.length>12000)return null;
  const params=new URL('https://viewer.invalid/?'+hash.replace(/^#/, '')).searchParams;
  const tab=params.get('tab')||(params.has('world')?'start':null);
  if(!['start','graph','matrix','dsm','calls','services','architecture','quality','security'].includes(tab))return null;
  const route={tab:tab==='matrix'?'dsm':tab};
  for(const key of ['scope','node','to','chapter','lens','kpi','entity','diagram','direction','cycle','service','section','query','severity','language','resolution'])route[key]=params.get(key)||'';
  if(params.has('world'))route.scope=params.get('world');
  return route;
}
function viewerRouteHash(){
  const route={tab:ui.workspace==='dsm'?'matrix':ui.workspace};
  const name=id=>data.nodes.find(n=>String(n.id)===String(id))?.name;
  if(ui.workspace==='start')Object.assign(route,{scope:worldUi.within,lens:worldUi.lens,kpi:worldUi.kpi,node:worldUi.selection?.area,entity:worldUi.selection?.entity?.id});
  if(ui.workspace==='graph')Object.assign(route,{scope:name(treeLocation),node:ui.selection?.kind==='node'?ui.selection.name:ui.selection?.from,to:ui.selection?.to,diagram:ui.diagram,direction:ui.edgeMode});
  if(ui.workspace==='architecture'){
    const kind=architectureUi.chapter==='6'?'calls':architectureUi.chapter==='11'?'cycles':'dependencies';
    Object.assign(route,{chapter:architectureUi.chapter,scope:name(architectureUi.diagramStates?.[kind]?.scope)});
  }
  if(ui.workspace==='dsm')Object.assign(route,{scope:name(treeMatrixUi.scope),cycle:name(treeMatrixUi.cache?.cycles.find(c=>c.id===treeMatrixUi.cycle)?.members?.[0])});
  if(ui.workspace==='services')Object.assign(route,{service:serviceUi.selected,scope:serviceProjects().find(p=>p.id===serviceUi.project)?.path});
  if(ui.workspace==='calls')Object.assign(route,{scope:callUi.package,language:callUi.language,resolution:callUi.resolution,query:callUi.query});
  if(ui.workspace==='quality')Object.assign(route,{section:engineeringUi.section,query:engineeringUi.query});
  if(ui.workspace==='security')Object.assign(route,{severity:securityUi.severity,query:securityUi.query});
  return '#'+Object.entries(route).filter(([,value])=>value!=null&&value!=='').map(([key,value])=>key+'='+encodeURIComponent(value)).join('&');
}
function applyViewerRoute(route){
  if(!route)return;
  viewerNavigation.applying=true;
  try{
    const source=data.nodes.find(n=>n.name===route.scope&&n.kind==='module');
    if(route.tab==='start'){
      worldUi.within=source?.name||null;worldUi.selection=null;worldUi.history=[];
      worldUi.camera={x:0,y:0,zoom:1,rotation:0};
      worldUi.lens=['problems','complexity','coverage'].includes(route.lens)?route.lens:'problems';
      worldUi.kpi=['complexity',...WORLD_CLASS_KPIS].includes(route.kpi)?route.kpi:'complexity';
    }
    if(route.tab==='architecture'){
      architectureUi.chapter=/^(?:[1-9]|1[0-2])$/.test(route.chapter)?route.chapter:'5';
      const kind=architectureUi.chapter==='6'?'calls':architectureUi.chapter==='11'?'cycles':'dependencies';
      architectureUi.diagramStates??={};architectureUi.diagramStates[kind]={scope:source?String(source.id):null,selected:null};
    }
    if(route.tab==='graph'){
      ui.selection=null;trail=[];treeLocation=null;isolated=null;ui.cycle=null;treeBaseCollapses.clear();
      ui.diagram=['overview','dependencies','network'].includes(route.diagram)?route.diagram:'overview';
      const target=nodeByName(work,route.node)||source;
      if(target)navigateExplorerNode(target.id,target.kind==='module');else {recompute();renderAll(false);}
      if(source)treeLocation=source.id;
      if(route.to&&nodeByName(work,route.node)&&nodeByName(work,route.to))ui.selection={kind:'edge',from:route.node,to:route.to};
      if(['all','incoming','outgoing','both'].includes(route.direction)){ui.edgeMode=route.direction;ui.edgeModeAutomatic=false;}
    }
    if(route.tab==='dsm'){
      treeMatrixUi.scope=source?.id??null;treeMatrixUi.expanded.clear();treeMatrixUi.rowPage=0;treeMatrixUi.revision=null;treeMatrixUi.cycle=null;
      const model=buildTreeMatrix(work,treeMatrixUi.scope),member=nodeByName(work,route.cycle);
      const cycle=member&&model.cycles.find(c=>c.members.includes(member.id));
      if(cycle){treeMatrixUi.cycle=cycle.id;treeMatrixUi.sourceRevision=viewRevision;for(const id of cycle.members)for(const parent of ancestorsOf(work,id))treeMatrixUi.expanded.add(parent);}
    }
    if(route.tab==='services'){const selected=serviceModel()?.services.find(s=>s.id===route.service);serviceUi.selected=selected?.id||null;serviceUi.project=selected?.project||serviceProjects().find(p=>p.path===route.scope)?.id||null;serviceUi.edge=null;serviceUi.cycle=null;}
    if(route.tab==='calls'){callUi.package=source?.name||null;callUi.query=route.query;callUi.language=route.language||'all';callUi.resolution=['all','project','external','unresolved'].includes(route.resolution)?route.resolution:'project';}
    if(route.tab==='quality'){engineeringUi.section=route.section||null;engineeringUi.query=route.query;}
    if(route.tab==='security'){securityUi.severity=['critical','high','medium','low','unknown'].includes(route.severity)?route.severity:'all';securityUi.query=route.query;}
    showTab(route.tab);
    if(route.tab==='graph')renderAll(false);
    if(route.tab==='start'&&worldUi.model){
      const area=worldUi.model.areas.find(a=>a.name===route.node);
      if(area){const entity=area.entities.find(e=>e.id===route.entity);worldUi.selection={area:area.name,...(entity?{entity}:{})};worldDetails();worldDraw();}
    }
  }finally{viewerNavigation.applying=false;}
}
function syncViewerNavigation(replace=false){
  if(!viewerNavigation.ready||viewerNavigation.applying)return;
  const hash=viewerRouteHash();
  if(viewerNavigation.button)viewerNavigation.button.textContent=t('Copy link','Link kopieren');
  if(hash===viewerNavigation.last)return;
  const previous=parseViewerRoute(viewerNavigation.last),next=parseViewerRoute(hash);
  const major=!previous||['tab','scope','chapter'].some(key=>previous[key]!==next?.[key]);
  window.history[replace||!major?'replaceState':'pushState'](null,'',window.location.pathname+window.location.search+hash);
  viewerNavigation.last=hash;
}
function initViewerNavigation(){
  if(!window.location?.href||!window.history?.replaceState)return;
  applyViewerRoute(parseViewerRoute(window.location.hash));
  viewerNavigation.ready=true;syncViewerNavigation(true);
  const host=document.getElementById('share-actions');
  if(host){const link=button(t('Copy link','Link kopieren'),()=>{closeWorkspaceMenu('export-menu');syncViewerNavigation();copyText(window.location.protocol==='file:'?window.location.hash:window.location.href);},'btn btn-secondary');link.setAttribute('data-view-link','true');host.appendChild(link);viewerNavigation.button=link;}
  const restore=()=>{if(window.location.hash===viewerNavigation.last)return;const route=parseViewerRoute(window.location.hash);if(route)applyViewerRoute(route);viewerNavigation.last='';syncViewerNavigation(true);};
  window.addEventListener('popstate',restore);window.addEventListener('hashchange',restore);
  // Capture before bubbling, synchronize after the renderer has handled the event.
  for(const event of ['click','change','input','keydown','pointerup'])document.addEventListener(event,()=>setTimeout(()=>syncViewerNavigation(),0),true);
}

const rootName=(data.root||'Project').replace(/\\/g,'/').split('/').filter(Boolean).pop()||'Project';
document.getElementById('project-name').textContent=rootName;document.getElementById('project-name').title=data.root||rootName;document.getElementById('project-path').textContent=data.root||rootName;
if(window.innerWidth<=1100){document.getElementById('main').classList.add('explorer-closed','inspector-closed');for(const id of ['toggle-explorer','toggle-inspector'])document.getElementById(id).setAttribute('aria-expanded','false');}
initPanelResize();
initRepository();
initServices();
initMetricTooltips();
initDevelopment();
initKpiHelp();
refreshActionHelp();
applyTheme(storedTheme());recompute();renderAll();setWidth();
if(isRepositoryOverview())showTab(data.initialTab==='security'?'security':'start');
else if(data.initialTab === 'assessment' || data.initialTab === 'cycles' || data.initialTab === 'dsm' || data.initialTab === 'calls' || data.initialTab === 'services' || data.initialTab === 'architecture' || data.initialTab === 'security' || data.initialTab === 'review-dashboard')showTab(data.initialTab);
else if(!data.nodes.length&&serviceModel()?.services.length)showTab('services');
else if(developmentPaths().length&&!isRepositoryOverview())showTab('start');
// The live server can refresh a report once no simulated work would be lost.
window.archguardCanReload = () => !ops.length && !ui.pending && !kpiHelpUi.dialog?.open && (typeof scenarioCanReload!=='function'||scenarioCanReload()) && assessmentCanReload();

initViewerNavigation();
