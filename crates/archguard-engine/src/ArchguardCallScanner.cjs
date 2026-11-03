'use strict';
// Compiler-only source analysis: no emit, project code, plugins, scripts or network.
const fs = require('node:fs'), path = require('node:path'), crypto = require('node:crypto');
const input = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
const root = fs.realpathSync(input.root), compiler = fs.realpathSync(input.compiler);
const compilerRoot = path.dirname(path.dirname(compiler));
const ts = require(compiler);
if (!ts.createProgram || !ts.getPreEmitDiagnostics || !ts.version) throw Error('An installed TypeScript compiler is required');
const MAX_FILES = 20000, MAX_BYTES = 64 * 1024 * 1024, MAX_RECORDS = 100000;
const dependencies = new Map(), contents = new Map(), diagnostics = [], problems = new Set();
let readBytes = 0, partial = false;
const inside = (base, file) => file === base || file.startsWith(base + path.sep);
const allowed = file => inside(root, file) || inside(compilerRoot, file);
function note(text) { partial = true; if (diagnostics.length < 100 && !problems.has(text)) { problems.add(text); diagnostics.push(text); } }
function track(file, bytes) {
  if (dependencies.has(file)) return;
  if (dependencies.size >= MAX_FILES) throw Error('TypeScript input file budget exceeded');
  readBytes += bytes ? bytes.length : 0;
  if (readBytes > MAX_BYTES) throw Error('TypeScript input byte budget exceeded');
  dependencies.set(file, bytes === null ? null : crypto.createHash('sha256').update(bytes).digest('hex'));
}
function safePath(file) {
  const absolute = path.resolve(file);
  // TypeScript probes ancestor directories during ordinary module resolution.
  // Denied probes are not themselves missing source evidence; compiler errors
  // and explicit project-reference checks report actual unavailable inputs.
  if (!allowed(absolute)) return null;
  try { const real = fs.realpathSync(absolute); if (!allowed(real)) { note('A compiler input symlink leaves the analysis/compiler roots.'); return null; } return real; }
  catch { track(absolute, null); return null; }
}
function readFile(file) {
  const real = safePath(file); if (!real) return undefined;
  if (contents.has(real)) return contents.get(real);
  try {
    const stat = fs.statSync(real); if (!stat.isFile()) return undefined;
    if (stat.size > 8 * 1024 * 1024) throw Error('TypeScript source exceeds 8 MiB');
    const bytes = fs.readFileSync(real); track(real, bytes); const text = bytes.toString('utf8'); contents.set(real, text); return text;
  } catch (error) { if (String(error).includes('budget') || String(error).includes('8 MiB')) throw error; return undefined; }
}
const fileExists = file => readFile(file) !== undefined;
function directoryExists(file) { const real = safePath(file); return !!real && fs.statSync(real).isDirectory(); }
function realpath(file) { return safePath(file) || path.resolve(file); }
const sources = new Map(input.sources.map(source => [fs.realpathSync(source.path), source]));
if (sources.size !== input.sources.length || sources.size > MAX_FILES) throw Error('Invalid source manifest');
for (const file of sources.keys()) if (!inside(root, file)) throw Error('Source escapes analysis root');
const configHost = {useCaseSensitiveFileNames: ts.sys.useCaseSensitiveFileNames, getCurrentDirectory: () => root, readFile, fileExists, directoryExists, realpath,
  readDirectory(dir, extensions, exclude, include, depth) {
    const real = safePath(dir); if (!real) return [];
    return ts.sys.readDirectory(real, extensions, exclude, include, depth).filter(file => !!safePath(file));
  }, onUnRecoverableConfigFileDiagnostic: diagnostic => compilerDiagnostic(diagnostic)};
function compilerDiagnostic(diagnostic) {
  if (diagnostic.category !== ts.DiagnosticCategory.Error) return;
  const position = diagnostic.file && diagnostic.start !== undefined ? diagnostic.file.getLineAndCharacterOfPosition(diagnostic.start) : null;
  const file = diagnostic.file ? path.relative(root, diagnostic.file.fileName).split(path.sep).join('/') : '';
  note(`TS${diagnostic.code}${file ? ' ' + file + (position ? ':' + (position.line + 1) : '') : ''}: ${ts.flattenDiagnosticMessageText(diagnostic.messageText, ' ').slice(0, 600)}`);
}
const projects = [], visited = new Set();
function project(configPath) {
  const canonical = safePath(configPath); if (!canonical) { note('Configured TypeScript project is missing or outside the analysis root.'); return; }
  if (!inside(root, canonical)) { note('TypeScript project references outside the analysis root are not analyzed.'); return; }
  if (visited.has(canonical)) return;
  if (visited.size >= 64) throw Error('TypeScript project-reference budget exceeded');
  visited.add(canonical);
  const parsed = ts.getParsedCommandLineOfConfigFile(canonical, {noEmit: true}, configHost);
  if (!parsed) return;
  parsed.errors.forEach(compilerDiagnostic);
  if (parsed.options.plugins?.length) diagnostics.push('TypeScript language-service plugins are not executed.');
  for (const reference of parsed.projectReferences || []) {
    const referenced = path.extname(reference.path) === '.json' ? reference.path : path.join(reference.path, 'tsconfig.json');
    project(referenced);
  }
  projects.push({path: canonical, parsed});
}
if (input.project) project(path.resolve(root, input.project));
else if (fileExists(path.join(root, 'tsconfig.json'))) project(path.join(root, 'tsconfig.json'));
else {
  projects.push({path: null, parsed: {fileNames: [...sources].filter(([, s]) => s.language === 'typescript').map(([file]) => file),
    options: {noEmit: true, target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, moduleResolution: ts.ModuleResolutionKind.Node10}, errors: []}});
  diagnostics.push('No root tsconfig.json: TypeScript sources use an explicit ES2020/CommonJS source-only context.');
}
const typeUses = new Map(), classStates = new Map();
const sourceMetrics = new Map(), inheritance = new Map();
const classes = new Map(), methods = new Map(), calls = new Map(), covered = new Set();
const methodSites = new Map();
function bounded(map, key, value) { if (!map.has(key) && map.size >= MAX_RECORDS) throw Error('TypeScript declaration/call budget exceeded'); map.set(key, value); }
function location(node) {
  const sf = node.getSourceFile(), start = node.getStart(sf), end = Math.max(start, node.getEnd() - 1);
  const point = sf.getLineAndCharacterOfPosition(start);
  return {line: point.line + 1, column: point.character + 1, end_line: sf.getLineAndCharacterOfPosition(end).line + 1};
}
const functionNode = node => ts.isFunctionDeclaration(node) || ts.isMethodDeclaration(node) || ts.isMethodSignature(node)
  || ts.isConstructorDeclaration(node) || ts.isGetAccessorDeclaration(node) || ts.isSetAccessorDeclaration(node)
  || ts.isArrowFunction(node) || ts.isFunctionExpression(node);
const typeNode = node => ts.isClassDeclaration(node) || ts.isClassExpression(node) || ts.isInterfaceDeclaration(node);
const modifiers = node => ts.canHaveModifiers(node) ? ts.getModifiers(node) || [] : [];
const has = (node, kind) => modifiers(node).some(m => m.kind === kind);
function simpleName(node) {
  if (ts.isConstructorDeclaration(node)) return 'constructor';
  if (node.name) return node.name.getText();
  if (node.parent && (ts.isVariableDeclaration(node.parent) || ts.isPropertyDeclaration(node.parent) || ts.isPropertyAssignment(node.parent))) return node.parent.name.getText();
  if (node.parent && ts.isBinaryExpression(node.parent) && node.parent.right === node) return node.parent.left.getText();
  const at = location(node); return `anonymous@${at.line}:${at.column}`;
}
function metrics(node, errors) {
  if (!node.body || errors.length) return undefined;
  let cyclomatic = 1, max_nesting = 0;
  function visit(current, depth) {
    if (current !== node.body && (functionNode(current) || typeNode(current))) return;
    const control = ts.isIfStatement(current) || ts.isForStatement(current) || ts.isForOfStatement(current) || ts.isForInStatement(current)
      || ts.isWhileStatement(current) || ts.isDoStatement(current) || ts.isCatchClause(current) || ts.isConditionalExpression(current);
    if (control || ts.isCaseClause(current)) cyclomatic++;
    if (ts.isBinaryExpression(current) && [ts.SyntaxKind.AmpersandAmpersandToken, ts.SyntaxKind.BarBarToken, ts.SyntaxKind.QuestionQuestionToken,
      ts.SyntaxKind.AmpersandAmpersandEqualsToken, ts.SyntaxKind.BarBarEqualsToken, ts.SyntaxKind.QuestionQuestionEqualsToken].includes(current.operatorToken.kind)) cyclomatic++;
    if (current.questionDotToken) cyclomatic++;
    const next = depth + (control || ts.isSwitchStatement(current) ? 1 : 0);
    max_nesting = Math.max(max_nesting, next);
    if (ts.isIfStatement(current)) {
      visit(current.expression, next); visit(current.thenStatement, next);
      if (current.elseStatement) visit(current.elseStatement, ts.isIfStatement(current.elseStatement) ? depth : next);
    } else ts.forEachChild(current, child => visit(child, next));
  }
  visit(node.body, 0);
  return {cyclomatic, max_nesting, definition: 'ast-decisions-v1'};
}
function sourceMeasurement(node, id) {
  const operators=new Set(),operands=new Set(),lines=new Set();let n1=0,n2=0,cognitive=0;
  const logical=n=>ts.isBinaryExpression(n)&&[ts.SyntaxKind.AmpersandAmpersandToken,ts.SyntaxKind.BarBarToken].includes(n.operatorToken.kind);
  function tokens(n){
    if(ts.isIdentifier(n)||ts.isPrivateIdentifier(n)||ts.isLiteralExpression(n)||[ts.SyntaxKind.TrueKeyword,ts.SyntaxKind.FalseKeyword,ts.SyntaxKind.NullKeyword].includes(n.kind)){operands.add(n.getText());n2++;}
    else if((ts.isExpressionNode(n)||ts.isStatement(n)||ts.isCatchClause(n))&&!ts.isBlock(n)&&!ts.isParenthesizedExpression(n)){
      operators.add(ts.SyntaxKind[n.kind]+(n.operatorToken?':'+ts.SyntaxKind[n.operatorToken.kind]:n.operator?':'+ts.SyntaxKind[n.operator]:''));n1++;
    }else return;
    lines.add(location(n).line);
  }
  function sequence(n,depth,list){
    while(ts.isParenthesizedExpression(n))n=n.expression;
    if(logical(n)){tokens(n);sequence(n.left,depth,list);list.push(n.operatorToken.kind);sequence(n.right,depth,list);}else visit(n,depth);
  }
  function branch(n,depth,ladder){tokens(n);cognitive+=ladder?1:1+depth;visit(n.expression,depth);visit(n.thenStatement,depth+1);
    if(n.elseStatement){if(ts.isIfStatement(n.elseStatement))branch(n.elseStatement,depth,true);else{cognitive++;visit(n.elseStatement,depth+1);}}}
  function visit(n,depth){
    if(!n || n!==node.body&&(functionNode(n)||typeNode(n)))return;
    if(ts.isIfStatement(n)){branch(n,depth,false);return;}
    if(logical(n)){const list=[];sequence(n,depth,list);let last;for(const op of list){if(op!==last)cognitive++;last=op;}return;}
    tokens(n);
    const control=ts.isForStatement(n)||ts.isForInStatement(n)||ts.isForOfStatement(n)||ts.isWhileStatement(n)||ts.isDoStatement(n)||ts.isCatchClause(n)||ts.isConditionalExpression(n)||ts.isSwitchStatement(n);
    if(control)cognitive+=1+depth;
    if((ts.isBreakStatement(n)||ts.isContinueStatement(n))&&n.label)cognitive++;
    ts.forEachChild(n,c=>visit(c,depth+(control?1:0)));
  }
  visit(node.body,0);
  return {method_id:id,definition:'source-maintainability/v1',cognitive,distinct_operators:operators.size,distinct_operands:operands.size,operators:n1,operands:n2,source_lines:lines.size};
}
for (const projectContext of projects) {
  const options = {...projectContext.parsed.options, noEmit: true, incremental: false, composite: false, plugins: undefined};
  const host = ts.createCompilerHost(options, true);
  host.readFile = readFile; host.fileExists = fileExists; host.directoryExists = directoryExists; host.realpath = realpath;
  host.getCurrentDirectory = () => root;
  host.writeFile = () => { throw Error('Compiler emit is disabled'); };
  host.getSourceFile = (file, languageVersion) => { const text = readFile(file); return text === undefined ? undefined : ts.createSourceFile(file, text, languageVersion, true); };
  const roots = projectContext.parsed.fileNames.filter(file => {
    const source = sources.get(realpath(file)); return source && (source.language !== 'javascript' || options.allowJs);
  });
  if (!roots.length) continue;
  const program = ts.createProgram({rootNames: roots, options, host});
  const checker = program.getTypeChecker();
  ts.getPreEmitDiagnostics(program).forEach(compilerDiagnostic);
  const included = program.getSourceFiles().filter(sf => sources.has(realpath(sf.fileName)) && (!/\.jsx?$/.test(sf.fileName) || options.allowJs));
  const semanticErrors = new Map(included.map(sf => [sf, program.getSemanticDiagnostics(sf).filter(d => d.category === ts.DiagnosticCategory.Error)]));
  const syntaxErrors = new Map(included.map(sf => [sf, program.getSyntacticDiagnostics(sf)]));
  const badNode=node=>(semanticErrors.get(node.getSourceFile())||[]).some(d=>d.start===undefined||d.start>=node.getStart()&&d.start<node.getEnd()) || (syntaxErrors.get(node.getSourceFile())||[]).length>0;
  const nodeClasses = new Map(), nodeMethods = new Map(), symbolClasses = new Map(), symbolMethods = new Map(), modules = new Map();
  const sourceOf = node => sources.get(realpath(node.getSourceFile().fileName));
  const unalias = symbol => symbol && symbol.flags & ts.SymbolFlags.Alias ? checker.getAliasedSymbol(symbol) : symbol;
  function moduleOwner(sf) {
    if (modules.has(sf)) return modules.get(sf);
    const source = sourceOf(sf), id = source.file + '::<module>';
    bounded(classes, id, {id, name: source.file, package: path.posix.dirname(source.file) === '.' ? '' : path.posix.dirname(source.file),
      file: source.file, graph_node: source.graph_node, line: 1, kind: 'module', language: source.language, is_abstract: null,
      visibility: 'module', supertypes: [], supertype_sites: [], declaration_files: [source.file]});
    modules.set(sf, id); return id;
  }
  function declarations(node, owner, parentMethod) {
    const source = sourceOf(node); if (!source) return;
    if (typeNode(node)) {
      const namespaces = [];
      for (let parent = node.parent; parent; parent = parent.parent) if (ts.isModuleDeclaration(parent)) namespaces.unshift(parent.name.getText());
      const name = [...namespaces, simpleName(node)].join('.'), id = source.file + '::' + name + (parentMethod ? '@' + parentMethod : '');
      const kind = ts.isInterfaceDeclaration(node) ? 'interface' : 'class';
      const entry = {id, name, package: path.posix.dirname(source.file) === '.' ? '' : path.posix.dirname(source.file), file: source.file,
        graph_node: source.graph_node, line: location(node).line, kind, language: source.language,
        is_abstract: kind === 'interface' || has(node, ts.SyntaxKind.AbstractKeyword), visibility: has(node, ts.SyntaxKind.ExportKeyword) ? 'public' : 'module',
        supertypes: [], supertype_sites: [], declaration_files: [source.file]};
      bounded(classes, id, entry); nodeClasses.set(node, id);
      const symbol = node.name && unalias(checker.getSymbolAtLocation(node.name)); if (symbol) symbolClasses.set(symbol, id);
      owner = id; parentMethod = null;
    }
    if (functionNode(node)) {
      owner ||= moduleOwner(node.getSourceFile());
      const name = simpleName(node);
      const signature = name + '(' + node.parameters.map(parameter => {
        try { return checker.typeToString(checker.getTypeAtLocation(parameter), undefined, ts.TypeFormatFlags.NoTruncation); }
        catch { return parameter.type?.getText() || 'unknown'; }
      }).join(',') + ')';
      const namespaces = [];
      if (classes.get(owner).kind === 'module') for (let parent = node.parent; parent; parent = parent.parent) if (ts.isModuleDeclaration(parent)) namespaces.unshift(parent.name.getText());
      let id = owner + '#' + (namespaces.length ? namespaces.join('.') + '.' : '') + signature + (parentMethod ? '@' + parentMethod : '');
      // Distinct block-local declarations can share both a signature and a line.
      // A stable source offset also prevents duplicate project contexts from splitting one declaration.
      const site = source.file + ':' + node.getStart(node.getSourceFile());
      if (methods.has(id) && methodSites.get(id) !== site) id += '@' + location(node).line + ':' + location(node).column;
      methodSites.set(id, site);
      const entry = {id, class_id: owner, name, signature, file: source.file, line: location(node).line, end_line: location(node).end_line,
        is_static: classes.get(owner).kind === 'module' || has(node, ts.SyntaxKind.StaticKeyword)};
      const complexity = metrics(node, syntaxErrors.get(node.getSourceFile()) || []); if (complexity) { entry.complexity = complexity; bounded(sourceMetrics,id,sourceMeasurement(node,id)); }
      bounded(methods, id, entry); nodeMethods.set(node, id);
      const symbol = node.name && unalias(checker.getSymbolAtLocation(node.name));
      if (symbol) { if (!symbolMethods.has(symbol)) symbolMethods.set(symbol, []); symbolMethods.get(symbol).push(node); }
      parentMethod = id;
    }
    ts.forEachChild(node, child => declarations(child, owner, parentMethod));
  }
  for (const sf of included) { covered.add(realpath(sf.fileName)); declarations(sf, null, null); }
  function classForSymbol(symbol) {
    symbol = unalias(symbol);
    if (symbolClasses.has(symbol)) return symbolClasses.get(symbol);
    for (const declaration of symbol?.declarations || []) if (nodeClasses.has(declaration)) return nodeClasses.get(declaration);
    return null;
  }
  function declarationTarget(symbol) {
    const target = classForSymbol(symbol);
    if (target) return {to_class: target, target_label: target, resolution: 'project'};
    const files = new Map();
    for (const declaration of symbol.declarations || []) {
      const source = sourceOf(declaration);
      if (source) files.set(source.file, declaration.getSourceFile());
    }
    if (files.size === 1) {
      const [file, sf] = files.entries().next().value;
      return {to_class: moduleOwner(sf), target_label: file + '::' + symbol.getName(), resolution: 'project'};
    }
    return {to_class: null, target_label: checker.getFullyQualifiedName(symbol), resolution: files.size ? 'unresolved' : 'external'};
  }
  for (const [node, id] of nodeClasses) {
    const entry = classes.get(id);
    for (const clause of node.heritageClauses || []) for (const type of clause.types) {
      const target = classForSymbol(checker.getSymbolAtLocation(type.expression));
      if (target) { entry.supertypes.push(target); entry.supertype_sites.push({target, file: entry.file, line: location(type).line}); }
    }
    entry.supertypes = [...new Set(entry.supertypes)].sort();
    if(entry.kind==='class') {
      let current=checker.getTypeAtLocation(node),depth=0,base_class=null,known=true;const seen=new Set();
      while(current){
        if(seen.has(current)||depth>=512){known=false;break;}seen.add(current);
        const declarations=current.symbol?.declarations||[];
        const declaration=declarations.find(d=>ts.isClassDeclaration(d)||ts.isClassExpression(d));
        const clause=declaration?.heritageClauses?.find(c=>c.token===ts.SyntaxKind.ExtendsKeyword);
        if(clause&&clause.types.some(t=>badNode(t))){known=false;break;}
        const bases=checker.getBaseTypes(current)?.filter(b=>b.symbol?.flags&ts.SymbolFlags.Class)||[];
        if(!bases.length){if(clause)known=false;break;}
        if(bases.length!==1){known=false;break;}
        if(depth===0)base_class=classForSymbol(bases[0].symbol)||checker.getFullyQualifiedName(bases[0].symbol);
        depth++;current=bases[0];
      }
      bounded(inheritance,id,{class_id:id,base_class,depth:known?depth:null,status:known?'ready':'partial'});
    }
  }
  // A compiler signature is a type declaration, not proof that a reassigned
  // callable still contains its original function. Conservatively invalidate
  // written symbols across the source program (including writes through aliases).
  const writtenSymbols = new Set();
  function bindingSymbol(node) {
    return unalias(checker.getSymbolAtLocation(node));
  }
  function writeTarget(node) {
    if (ts.isParenthesizedExpression(node) || ts.isAsExpression(node) || ts.isNonNullExpression(node)) {
      writeTarget(node.expression); return;
    }
    if (ts.isArrayLiteralExpression(node)) {
      for (const element of node.elements) writeTarget(ts.isSpreadElement(element) ? element.expression : element);
      return;
    }
    if (ts.isObjectLiteralExpression(node)) {
      for (const property of node.properties) {
        if (ts.isPropertyAssignment(property)) writeTarget(property.initializer);
        else if (ts.isShorthandPropertyAssignment(property)) {
          const symbol = unalias(checker.getShorthandAssignmentValueSymbol(property));
          if (symbol) writtenSymbols.add(symbol);
        } else if (ts.isSpreadAssignment(property)) writeTarget(property.expression);
      }
      return;
    }
    // Defaults inside a destructuring assignment still assign their left binding.
    if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.EqualsToken) {
      writeTarget(node.left); return;
    }
    const symbol = bindingSymbol(node);
    if (symbol) writtenSymbols.add(symbol);
  }
  function writes(node) {
    if (ts.isBinaryExpression(node) && node.operatorToken.kind >= ts.SyntaxKind.FirstAssignment
      && node.operatorToken.kind <= ts.SyntaxKind.LastAssignment) writeTarget(node.left);
    else if ((ts.isPrefixUnaryExpression(node) || ts.isPostfixUnaryExpression(node))
      && [ts.SyntaxKind.PlusPlusToken, ts.SyntaxKind.MinusMinusToken].includes(node.operator)) writeTarget(node.operand);
    else if (ts.isDeleteExpression(node)) writeTarget(node.expression);
    else if ((ts.isForOfStatement(node) || ts.isForInStatement(node)) && !ts.isVariableDeclarationList(node.initializer)) writeTarget(node.initializer);
    ts.forEachChild(node, writes);
  }
  for (const sf of program.getSourceFiles()) if (!sf.isDeclarationFile && inside(root, realpath(sf.fileName))) writes(sf);
  function writtenTarget(node) {
    if (writtenSymbols.has(bindingSymbol(node))) return true;
    if (ts.isPropertyAccessExpression(node) || ts.isElementAccessExpression(node)
      || ts.isParenthesizedExpression(node) || ts.isAsExpression(node) || ts.isNonNullExpression(node)) return writtenTarget(node.expression);
    return false;
  }
  function targetOf(node) {
    const expression = node.expression;
    if (writtenTarget(expression)) return null;
    const binding = unalias(checker.getSymbolAtLocation(expression));
    if (!binding && !nodeMethods.has(expression)) return null;
    if (binding?.declarations?.some(declaration => ts.isVariableDeclaration(declaration)
      && (!declaration.initializer || !functionNode(declaration.initializer)))) return null;
    const targetType = checker.getTypeAtLocation(expression);
    if (targetType.flags & (ts.TypeFlags.Any | ts.TypeFlags.Unknown | ts.TypeFlags.Union | ts.TypeFlags.Intersection)) return null;
    const signature = checker.getResolvedSignature(node), declaration = signature?.declaration;
    if (nodeMethods.has(declaration)) {
      const method = methods.get(nodeMethods.get(declaration));
      return {to_class: method.class_id, to_method: method.id, target_label: method.signature,
        dispatch: ts.isNewExpression(node) ? 'constructor' : classes.get(method.class_id).kind === 'interface' ? 'interface' : method.is_static ? 'static' : 'instance', resolution: 'project'};
    }
    if (ts.isNewExpression(node)) {
      const classId = classForSymbol(checker.getSymbolAtLocation(expression));
      if (classId) return {to_class: classId, to_method: null, target_label: classes.get(classId).name, dispatch: 'constructor', resolution: 'project'};
    }
    if (!declaration) return null;
    const declFile = realpath(declaration.getSourceFile().fileName);
    if (!sources.has(declFile)) {
      return {to_class: null, to_method: null, target_label: checker.signatureToString(signature).slice(0, 500), dispatch: ts.isNewExpression(node) ? 'constructor' : 'unknown', resolution: 'external'};
    }
    return null;
  }
  const fieldSymbols=new Map(), fieldDeclarations=new Map();
  function details(node,owner,method) {
    if(nodeClasses.has(node)) {
      owner=nodeClasses.get(node);method=null;
      bounded(classStates,owner,{class_id:owner,status:badNode(node)?'partial':'ready',fields:[],accesses:[]});
    }
    if(nodeMethods.has(node)){method=nodeMethods.get(node);owner=methods.get(method).class_id;}
    const state=classStates.get(owner), source=sourceOf(node);
    function typeUse(type,role) {
      if(!type||!owner||!source)return;
      function visit(part) {
        if(ts.isTypeReferenceNode(part) || ts.isTypeQueryNode(part)) {
          const expression=part.typeName||part.exprName;
          const symbol=unalias(checker.getSymbolAtLocation(expression));
          if(symbol&&!badNode(part)) {
            bounded(typeUses,source.file+':'+part.getStart()+':'+role,{from_class:owner,from_method:method,...declarationTarget(symbol),role,file:source.file,line:location(part).line,code:part.getText().slice(0,1000)});
          }
        }
        ts.forEachChild(part,visit);
      }visit(type);
    }
    const parameterProperty=ts.isParameter(node)&&ts.isConstructorDeclaration(node.parent)&&modifiers(node).some(m=>[ts.SyntaxKind.PublicKeyword,ts.SyntaxKind.PrivateKeyword,ts.SyntaxKind.ProtectedKeyword,ts.SyntaxKind.ReadonlyKeyword].includes(m.kind));
    if(state && (ts.isPropertyDeclaration(node)||ts.isPropertySignature(node)||ts.isGetAccessorDeclaration(node)||ts.isSetAccessorDeclaration(node)||parameterProperty)) {
      const id=owner+'#field:'+node.name.getText(), symbol=unalias(checker.getSymbolAtLocation(node.name));
      if(symbol)fieldSymbols.set(symbol,id);fieldDeclarations.set(node,id);
      if(!state.fields.some(f=>f.id===id))state.fields.push({id,name:node.name.getText(),is_static:has(node,ts.SyntaxKind.StaticKeyword),file:source.file,line:location(node).line});
      typeUse(node.type,'field');
      if(method&&(ts.isGetAccessorDeclaration(node)||ts.isSetAccessorDeclaration(node)))state.accesses.push({method,field:id,file:source.file,line:location(node).line});
    }
    if(ts.isParameter(node))typeUse(node.type,'parameter');
    if(functionNode(node))typeUse(node.type,'return');
    if(ts.canHaveDecorators(node))for(const decorator of ts.getDecorators(node)||[]) {
      const expression=ts.isCallExpression(decorator.expression)?decorator.expression.expression:decorator.expression;
      const symbol=unalias(checker.getSymbolAtLocation(expression));
      if(owner&&symbol&&!badNode(decorator))bounded(typeUses,source.file+':'+decorator.getStart()+':annotation',{from_class:owner,from_method:method,...declarationTarget(symbol),role:typeNode(node)?'class_annotation':'annotation',file:source.file,line:location(decorator).line,code:decorator.getText().slice(0,1000)});
    }
    ts.forEachChild(node,child=>details(child,owner,method));
  }
  for(const sf of included)details(sf,null,null);
  function access(node,owner,method) {
    if(!method||!classStates.has(owner)||(!ts.isIdentifier(node)&&!ts.isPrivateIdentifier(node)))return;
    if(node.parent?.name===node&&!ts.isPropertyAccessExpression(node.parent))return;
    const symbol=unalias(checker.getSymbolAtLocation(node));
    const field=fieldSymbols.get(symbol)||(symbol?.declarations||[]).map(d=>fieldDeclarations.get(d)).find(Boolean);
    if(!field||badNode(node))return;
    const state=classStates.get(owner);
    if(state.accesses.length>=MAX_RECORDS){state.status='partial';return;}
    state.accesses.push({method,field,file:sourceOf(node).file,line:location(node).line});
  }
  function collect(node, owner, method) {
    if (nodeClasses.has(node)) { owner = nodeClasses.get(node); method = null; }
    if (nodeMethods.has(node)) { method = nodeMethods.get(node); owner = methods.get(method).class_id; }
    access(node, owner, method);
    if (ts.isCallExpression(node) || ts.isNewExpression(node)) {
      const source = sourceOf(node); owner ||= moduleOwner(node.getSourceFile());
      let target = targetOf(node);
      const sf = node.getSourceFile(), errors = (semanticErrors.get(sf) || []).filter(d =>
        d.start === undefined || d.start >= node.getStart(sf) && d.start < node.getEnd());
      if (errors.length) target = null;
      if (!target) { note('Some calls have no unique compiler-resolved declaration; dynamic/ambiguous targets remain unresolved.');
        target = {to_class: null, to_method: null, target_label: node.expression.getText().slice(0, 500), dispatch: 'unknown', resolution: 'unresolved'}; }
      const at = location(node), entry = {from_class: owner, from_method: method, ...target, file: source.file, line: at.line, column: at.column,
        code: node.getText().slice(0, 1000)};
      bounded(calls, source.file + ':' + node.getStart(sf), entry);
    }
    ts.forEachChild(node, child => collect(child, owner, method));
  }
  for (const sf of included) collect(sf, null, null);
}
for (const [file, source] of sources) if (!covered.has(file)) note(`Source not analyzed by the selected TypeScript project/allowJs settings: ${source.file}`);
const status = !covered.size ? 'unavailable' : partial ? 'partial' : 'ready';
const analyzers = [...new Set(input.sources.map(source => source.language))].sort().map(language => ({language, backend: 'typescript', status,
  files: input.sources.filter(source => source.language === language).length, version: ts.version,
  project: input.project || (visited.size ? [...visited].map(file => path.relative(root, file)).join(', ') : null), diagnostics: [...diagnostics]}));
const analysis = {details:{enumeration_complete:true,definition:"source-types-state/v1",type_uses:[...typeUses.values()],class_states:[...classStates.values()],method_metrics:[...sourceMetrics.values()],inheritance:[...inheritance.values()]},status, java_release: 0, files: sources.size, classes: [...classes.values()], methods: [...methods.values()],
  calls: [...calls.values()].sort((a, b) => a.file.localeCompare(b.file) || a.line - b.line || a.column - b.column), analyzers, diagnostics};
process.stdout.write(JSON.stringify({analysis, inputs: [...dependencies].map(([file, sha256]) => ({file, sha256}))}));
