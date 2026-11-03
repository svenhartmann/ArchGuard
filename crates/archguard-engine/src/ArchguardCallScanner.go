// ArchGuard's Go evidence helper reads supplied source only. It never invokes
// project commands, downloads modules, or opens project files. go/types proves
// direct targets; interface and function-value dispatch remain unresolved.
package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"go/ast"
	"go/build"
	"go/importer"
	"go/parser"
	"go/printer"
	"go/scanner"
	"go/token"
	"go/types"
	"io"
	"os"
	"path"
	"path/filepath"
	"runtime"
	"sort"
	"strings"
)

type object = map[string]any
type sourceFile struct {
	Path        string `json:"path"`
	Content     string `json:"content"`
	PackagePath string `json:"package_path"`
}
type input struct {
	Files []sourceFile `json:"files"`
}

const maxInput = 128 << 20
const maxFiles = 20000
const maxEvidence = 100000

type unit struct {
	sourceFile
	tree     *ast.File
	group    *pkgGroup
	syntaxOK bool
}
type pkgGroup struct {
	name, importPath, logicalPath string
	units                         []*unit
	pkg                           *types.Package
	info                          *types.Info
	checking, checked, failed     bool
}
type method struct {
	decl      *ast.FuncDecl
	owner, id string
	unit      *unit
}
type analysis struct {
	ambiguous                                                          map[string]bool
	dynamic                                                            bool
	standard                                                           types.Importer
	fieldCount, accessCount                                            int
	fset                                                               *token.FileSet
	groups                                                             map[string]*pkgGroup
	byImport                                                           map[string]*pkgGroup
	units                                                              []*unit
	methods                                                            []method
	classes, methodRows, calls, typeUses, states, metrics, inheritance []object
	classByID                                                          map[string]object
	stateByID                                                          map[string]object
	functions                                                          map[*types.Func]method
	fields                                                             map[*types.Var]string
	diagnostics                                                        []string
	partial, complete, budgetHit                                       bool
}

func main() {
	data, err := io.ReadAll(io.LimitReader(os.Stdin, maxInput+1))
	if err != nil || len(data) > maxInput {
		emit(failed("Go input exceeds byte budget or cannot be read"))
		return
	}
	var in input
	if err = json.Unmarshal(data, &in); err != nil {
		emit(failed("Invalid Go source manifest: " + err.Error()))
		return
	}
	emit(analyze(in))
}
func emit(r object) {
	if err := json.NewEncoder(os.Stdout).Encode(r); err != nil {
		fmt.Fprintln(os.Stderr, "Cannot encode Go evidence:", err)
		os.Exit(1)
	}
}
func failed(message string) object {
	a := newAnalysis()
	a.complete = false
	a.note(message)
	return a.report()
}
func newAnalysis() *analysis {
	return &analysis{fset: token.NewFileSet(), standard: importer.Default(), ambiguous: map[string]bool{}, groups: map[string]*pkgGroup{}, byImport: map[string]*pkgGroup{}, classByID: map[string]object{}, stateByID: map[string]object{}, functions: map[*types.Func]method{}, fields: map[*types.Var]string{}, complete: true, diagnostics: []string{}, classes: []object{}, methodRows: []object{}, calls: []object{}, typeUses: []object{}, states: []object{}, metrics: []object{}, inheritance: []object{}}
}
func (a *analysis) note(s string) {
	a.partial = true
	if len(a.diagnostics) < 80 {
		if len(s) > 1000 {
			s = s[:1000]
		}
		a.diagnostics = append(a.diagnostics, s)
	}
}
func (a *analysis) allow() bool {
	if len(a.calls)+len(a.typeUses)+len(a.methodRows)+len(a.classes) >= maxEvidence {
		if !a.budgetHit {
			a.note("Go evidence budget reached; enumeration is incomplete")
			a.budgetHit = true
			a.complete = false
		}
		return false
	}
	return true
}
func analyze(in input) object {
	a := newAnalysis()
	sort.Slice(in.Files, func(i, j int) bool { return in.Files[i].Path < in.Files[j].Path })
	if len(in.Files) > maxFiles {
		a.note("Go file budget reached; enumeration is incomplete")
		a.complete = false
		in.Files = in.Files[:maxFiles]
	}
	seen := map[string]bool{}
	for _, s := range in.Files {
		if seen[s.Path] {
			a.note("Duplicate source path: " + s.Path)
			a.complete = false
			continue
		}
		seen[s.Path] = true
		if s.PackagePath == "" {
			s.PackagePath = path.Dir(s.Path)
		}
		tree, err := parser.ParseFile(a.fset, s.Path, s.Content, parser.AllErrors|parser.ParseComments)
		if err != nil {
			a.note(err.Error())
			a.complete = false
		}
		if tree == nil {
			continue
		}
		if strings.HasSuffix(s.Path, "_test.go") && strings.HasSuffix(tree.Name.Name, "_test") {
			s.PackagePath += "_test"
		}
		key := s.PackagePath + "\x00" + tree.Name.Name + "\x00" + path.Dir(s.Path)
		g := a.groups[key]
		if g == nil {
			g = &pkgGroup{name: tree.Name.Name, importPath: s.PackagePath, logicalPath: s.PackagePath}
			a.groups[key] = g
			if old := a.byImport[s.PackagePath]; old != nil || a.ambiguous[s.PackagePath] {
				a.ambiguous[s.PackagePath] = true
				a.note("Multiple source directories or package clauses claim " + s.PackagePath + "; imports are ambiguous")
				delete(a.byImport, s.PackagePath)
			} else {
				a.byImport[s.PackagePath] = g
			}
		}
		u := &unit{sourceFile: s, tree: tree, group: g, syntaxOK: err == nil}
		g.units = append(g.units, u)
		a.units = append(a.units, u)
		// All supplied variants remain visible. Their union is NOT a build target.
		// Explicitly report conditional source instead of silently choosing the host.
		if conditional(s.Path, s.Content) {
			g.failed = true
			a.note("Conditional build source " + s.Path + ": combined source view; build tags/GOOS/GOARCH were not selected")
		}
	}
	// A monorepo can contain separate modules with the same declared module
	// path. Keep their compiler universes and declaration identities separate.
	// The directory suffix is only an evidence identity, never an import alias.
	for _, u := range a.units {
		g := u.group
		if a.ambiguous[g.logicalPath] {
			g.importPath = g.logicalPath + "@source/" + path.Dir(u.Path) + "@package/" + g.name
			u.PackagePath = g.importPath
		}
	}
	for _, u := range a.units {
		a.check(u.group)
	}
	for _, u := range a.units {
		a.declarations(u)
	}
	for _, u := range a.units {
		a.registerMethods(u)
	}
	for _, u := range a.units {
		a.declarationTypes(u)
		a.globals(u)
	}
	for _, m := range a.methods {
		a.body(m)
	}
	return a.report()
}
func conditional(p, s string) bool {
	for _, line := range strings.Split(s, "\n") {
		line = strings.TrimSpace(line)
		if strings.HasPrefix(line, "//go:build ") || strings.HasPrefix(line, "// +build ") {
			return true
		}
		if strings.HasPrefix(line, "package ") {
			break
		}
	}
	base := strings.TrimSuffix(path.Base(p), ".go")
	base = strings.TrimSuffix(base, "_test")
	for _, part := range strings.Split(base, "_") {
		if strings.Contains(" aix android darwin dragonfly freebsd hurd illumos ios js linux nacl netbsd openbsd plan9 solaris wasip1 windows zos 386 amd64 amd64p32 arm arm64 loong64 mips mipsle mips64 mips64le ppc ppc64 ppc64le riscv riscv64 s390 s390x sparc sparc64 wasm ", " "+part+" ") {
			return true
		}
	}
	return false
}
func (a *analysis) Import(p string) (*types.Package, error) {
	if a.ambiguous[p] {
		return nil, fmt.Errorf("Go import %q is ambiguous across selected source directories or package clauses", p)
	}
	if g := a.byImport[p]; g != nil {
		if g.checking {
			return nil, fmt.Errorf("local import cycle involving %s", p)
		}
		a.check(g)
		if g.pkg != nil {
			return g.pkg, nil
		}
	}
	// Only installed SDK source directories can reach the compiler importer.
	// ImportDir with FindOnly takes go/build's local-directory path and never
	// resolves a module or runs a project command. Do not use Import here:
	// its nonlocal-path lookup may consult modules or the user's GOPATH.
	if p == "" || p == "." || path.IsAbs(p) || path.Clean(p) != p || strings.Contains(p, "\\") || strings.HasPrefix(p, "../") {
		return nil, fmt.Errorf("Go dependency %q is not supplied source or installed standard library", p)
	}
	ctx := build.Default
	ctx.GOPATH = ""
	directory := filepath.Join(ctx.GOROOT, "src", filepath.FromSlash(p))
	pkg, err := ctx.ImportDir(directory, build.FindOnly)
	if err != nil || !pkg.Goroot || pkg.ImportPath != p {
		return nil, fmt.Errorf("Go dependency %q is not supplied source or installed standard library", p)
	}
	// The default importer reads compiler export data and may invoke the
	// installed SDK's `go list -export` for this standard-library directory.
	// The runner supplies a controlled, offline Go environment. External
	// project packages never reach this fallback; missing data stays partial.
	return a.standard.Import(p)
}
func (a *analysis) check(g *pkgGroup) {
	if g.checked || g.checking {
		return
	}
	g.checking = true
	g.info = &types.Info{Types: map[ast.Expr]types.TypeAndValue{}, Defs: map[*ast.Ident]types.Object{}, Uses: map[*ast.Ident]types.Object{}, Selections: map[*ast.SelectorExpr]*types.Selection{}, Instances: map[*ast.Ident]types.Instance{}}
	files := []*ast.File{}
	for _, u := range g.units {
		files = append(files, u.tree)
	}
	conf := types.Config{Importer: a, Error: func(err error) { g.failed = true; a.note(err.Error()) }}
	g.pkg, _ = conf.Check(g.importPath, a.fset, files, g.info)
	g.checking = false
	g.checked = true
}
func (a *analysis) pos(n ast.Node) (string, int, int) {
	p := a.fset.Position(n.Pos())
	return p.Filename, p.Line, p.Column
}
func (a *analysis) code(n ast.Node) string {
	var b bytes.Buffer
	_ = printer.Fprint(&b, a.fset, n)
	s := b.String()
	if len(s) > 500 {
		s = s[:500]
	}
	return s
}
func typeID(t types.Type) string {
	if t == nil {
		return ""
	}
	t = types.Unalias(t)
	if p, ok := t.(*types.Pointer); ok {
		t = types.Unalias(p.Elem())
	}
	if n, ok := t.(*types.Named); ok {
		obj := n.Origin().Obj()
		if obj.Pkg() != nil {
			return obj.Pkg().Path() + "/" + obj.Name()
		}
	}
	return ""
}
func (a *analysis) addClass(u *unit, name, id, kind string, n ast.Node) {
	if !a.allow() {
		return
	}
	if _, exists := a.classByID[id]; exists {
		a.note("Duplicate Go declaration " + id)
		return
	}
	_, line, _ := a.pos(n)
	vis := "package"
	if ast.IsExported(name) {
		vis = "public"
	}
	c := object{"language": "go", "id": id, "name": name, "package": u.group.logicalPath, "file": u.Path, "graph_node": u.Path, "line": line, "kind": kind, "visibility": vis, "is_abstract": kind == "interface", "supertypes": []string{}}
	a.classes = append(a.classes, c)
	a.classByID[id] = c
	status := "ready"
	if u.group.failed || !u.syntaxOK {
		status = "partial"
	}
	state := object{"class_id": id, "status": status, "fields": []object{}, "accesses": []object{}}
	a.states = append(a.states, state)
	a.stateByID[id] = state
	a.inheritance = append(a.inheritance, object{"class_id": id, "base_class": nil, "depth": nil, "status": "not_applicable"})
}
func packageID(u *unit) string { return u.PackagePath + "/@file/" + path.Base(u.Path) }
func (a *analysis) declarations(u *unit) {
	for _, d := range u.tree.Decls {
		switch d := d.(type) {
		case *ast.GenDecl:
			for _, spec := range d.Specs {
				t, ok := spec.(*ast.TypeSpec)
				if !ok {
					continue
				}
				kind := ""
				switch t.Type.(type) {
				case *ast.StructType:
					kind = "struct"
				case *ast.InterfaceType:
					kind = "interface"
				}
				if kind == "" {
					continue
				}
				id := u.PackagePath + "/" + t.Name.Name
				a.addClass(u, t.Name.Name, id, kind, t)
				st, ok := t.Type.(*ast.StructType)
				if !ok {
					continue
				}
				state := a.stateByID[id]
				if state == nil {
					continue
				}
				for _, field := range st.Fields.List {
					if a.fieldCount >= maxEvidence {
						a.note("Go field budget reached")
						a.complete = false
						break
					}
					names := field.Names
					if len(names) == 0 { // Embedded fields are state, never superclass edges.
						ident := embeddedFieldName(field.Type)
						if ident != nil {
							names = []*ast.Ident{ident}
						}
					}
					for _, name := range names {
						obj, _ := u.group.info.Defs[name].(*types.Var)
						if obj == nil {
							if named, ok := types.Unalias(u.group.info.TypeOf(t.Type)).(*types.Struct); ok {
								for i := 0; i < named.NumFields(); i++ {
									f := named.Field(i)
									if f.Pos() == name.Pos() {
										obj = f
										break
									}
								}
							}
						}
						fid := id + "." + name.Name
						_, line, _ := a.pos(name)
						a.fieldCount++
						state["fields"] = append(state["fields"].([]object), object{"id": fid, "name": name.Name, "is_static": false, "file": u.Path, "line": line})
						if obj != nil {
							a.fields[obj.Origin()] = fid
						}
					}
				}
			}
		case *ast.FuncDecl:
			if d.Recv == nil {
				a.addPackage(u, d)
			}
		}
	}
}
func (a *analysis) addPackage(u *unit, n ast.Node) {
	id := packageID(u)
	if a.classByID[id] == nil {
		a.addClass(u, path.Base(u.Path), id, "package", n)
	}
}

// The field name comes from the embedded base type, never its type arguments.
// Preserve its identifier position for go/types' implicit field definition.
func embeddedFieldName(expr ast.Expr) *ast.Ident {
	switch expr := expr.(type) {
	case *ast.Ident:
		return expr
	case *ast.SelectorExpr:
		return expr.Sel
	case *ast.StarExpr:
		return embeddedFieldName(expr.X)
	case *ast.IndexExpr:
		return embeddedFieldName(expr.X)
	case *ast.IndexListExpr:
		return embeddedFieldName(expr.X)
	}
	return nil
}

func receiverName(e ast.Expr) string {
	switch e := e.(type) {
	case *ast.Ident:
		return e.Name
	case *ast.StarExpr:
		return receiverName(e.X)
	case *ast.IndexExpr:
		return receiverName(e.X)
	case *ast.IndexListExpr:
		return receiverName(e.X)
	}
	return ""
}
func (a *analysis) registerMethods(u *unit) {
	for _, decl := range u.tree.Decls {
		d, ok := decl.(*ast.FuncDecl)
		if !ok || !a.allow() {
			continue
		}
		owner := packageID(u)
		if d.Recv != nil && len(d.Recv.List) > 0 {
			owner = u.PackagePath + "/" + receiverName(d.Recv.List[0].Type)
			if fn, ok := u.group.info.Defs[d.Name].(*types.Func); ok {
				if sig, ok := fn.Type().(*types.Signature); ok && sig.Recv() != nil {
					if id := typeID(sig.Recv().Type()); id != "" {
						owner = id
					}
				}
			}
		}
		if a.classByID[owner] == nil { // Named scalar types can also own Go methods.
			a.addClass(u, strings.TrimPrefix(owner, u.PackagePath+"/"), owner, "type", d)
		}
		id := owner + "#" + d.Name.Name
		if d.Name.Name == "init" {
			id += fmt.Sprintf("@%d", a.fset.Position(d.Pos()).Line)
		}
		m := method{d, owner, id, u}
		a.methods = append(a.methods, m)
		if fn, ok := u.group.info.Defs[d.Name].(*types.Func); ok {
			a.functions[fn.Origin()] = m
		}
		_, line, _ := a.pos(d)
		row := object{"id": id, "class_id": owner, "name": d.Name.Name, "signature": a.code(d.Type), "file": u.Path, "line": line, "end_line": a.fset.Position(d.End()).Line, "is_static": d.Recv == nil}
		if d.Body != nil && u.syntaxOK {
			complexity, metrics := a.measure(m)
			row["complexity"] = complexity
			a.metrics = append(a.metrics, metrics)
		}
		a.methodRows = append(a.methodRows, row)
	}
}
func (a *analysis) use(u *unit, owner, mid, role string, expr ast.Expr) {
	// Inner composite literals may omit their type (for example []T{{...}}).
	// A missing syntax expression is not an unresolved type reference.
	if expr == nil {
		return
	}
	seen := map[string]bool{}
	ast.Inspect(expr, func(n ast.Node) bool {
		ident, ok := n.(*ast.Ident)
		if !ok {
			return true
		}
		obj := u.group.info.Uses[ident]
		tn, ok := obj.(*types.TypeName)
		if !ok {
			return true
		}
		if tn.Pkg() == nil {
			return true
		} // Builtins and type parameters are not coupling.
		if _, ok := types.Unalias(tn.Type()).(*types.TypeParam); ok {
			return true
		}
		target := typeID(tn.Type())
		if target == "" {
			target = tn.Pkg().Path() + "/" + tn.Name()
		}
		if target == owner || seen[target] {
			return true
		}
		seen[target] = true
		resolution := "external"
		if a.classByID[target] != nil {
			resolution = "project"
		}
		_, line, _ := a.pos(ident)
		var methodID any
		if mid != "" {
			methodID = mid
		}
		if a.allow() {
			a.typeUses = append(a.typeUses, object{"from_class": owner, "from_method": methodID, "to_class": target, "target_label": target, "role": role, "resolution": resolution, "file": u.Path, "line": line, "code": a.code(expr)})
		}
		return true
	})
}
func (a *analysis) useFields(u *unit, owner, mid, role string, list *ast.FieldList) {
	if list == nil {
		return
	}
	for _, f := range list.List {
		a.use(u, owner, mid, role, f.Type)
	}
}
func (a *analysis) declarationTypes(u *unit) {
	for _, d := range u.tree.Decls {
		gd, ok := d.(*ast.GenDecl)
		if !ok {
			continue
		}
		for _, spec := range gd.Specs {
			t, ok := spec.(*ast.TypeSpec)
			if !ok {
				continue
			}
			owner := u.PackagePath + "/" + t.Name.Name
			if a.classByID[owner] == nil {
				continue
			}
			switch x := t.Type.(type) {
			case *ast.StructType:
				a.useFields(u, owner, "", "field", x.Fields)
			case *ast.InterfaceType:
				for _, field := range x.Methods.List {
					if ft, ok := field.Type.(*ast.FuncType); ok {
						for _, name := range field.Names {
							if !a.allow() {
								break
							}
							id := owner + "#" + name.Name
							_, line, _ := a.pos(field)
							a.methodRows = append(a.methodRows, object{"id": id, "class_id": owner, "name": name.Name, "signature": a.code(ft), "file": u.Path, "line": line, "end_line": a.fset.Position(field.End()).Line, "is_static": false})
							a.useFields(u, owner, id, "parameter", ft.Params)
							a.useFields(u, owner, id, "return", ft.Results)
						}
					} else {
						a.use(u, owner, "", "field", field.Type)
					}
				}
			}
		}
	}
}
func unindex(e ast.Expr) ast.Expr {
	switch e := e.(type) {
	case *ast.IndexExpr:
		return unindex(e.X)
	case *ast.IndexListExpr:
		return unindex(e.X)
	case *ast.ParenExpr:
		return unindex(e.X)
	}
	return e
}
func (a *analysis) body(m method) {
	u := m.unit
	a.useFields(u, m.owner, m.id, "parameter", m.decl.Type.Params)
	a.useFields(u, m.owner, m.id, "return", m.decl.Type.Results)
	if m.decl.Body == nil {
		return
	}
	a.walkBody(m, m.decl.Body)
}
func (a *analysis) walkBody(m method, node ast.Node) {
	if node == nil {
		return
	}
	u := m.unit
	info := u.group.info
	ast.Inspect(node, func(n ast.Node) bool {
		switch n := n.(type) {
		case *ast.SelectorExpr:
			if sel := info.Selections[n]; sel != nil && sel.Kind() == types.FieldVal {
				if field, ok := sel.Obj().(*types.Var); ok {
					// Promoted access also touches the explicit embedding field of the receiver.
					if len(sel.Index()) > 1 {
						t := types.Unalias(sel.Recv())
						if p, ok := t.(*types.Pointer); ok {
							t = types.Unalias(p.Elem())
						}
						if st, ok := t.Underlying().(*types.Struct); ok {
							field = st.Field(sel.Index()[0])
						}
					}
					if fid := a.fields[field.Origin()]; fid != "" {
						state := a.stateByID[m.owner]
						if state != nil && strings.HasPrefix(fid, m.owner+".") {
							_, line, _ := a.pos(n)
							accesses := state["accesses"].([]object)
							if a.accessCount < maxEvidence {
								a.accessCount++
								state["accesses"] = append(accesses, object{"method": m.id, "field": fid, "file": u.Path, "line": line})
							} else {
								a.complete = false
								a.note("Go field-access budget reached")
							}
						}
					}
				}
			}
		case *ast.ValueSpec:
			if n.Type != nil {
				a.use(u, m.owner, m.id, "local", n.Type)
			}
		case *ast.CompositeLit:
			a.use(u, m.owner, m.id, "local", n.Type)
		case *ast.CallExpr:
			a.call(m, n)
		}
		return true
	})
}
func (a *analysis) call(m method, c *ast.CallExpr) {
	if !a.allow() {
		return
	}
	u := m.unit
	info := u.group.info
	if tv, ok := info.Types[c.Fun]; ok && tv.IsType() {
		a.use(u, m.owner, m.id, "local", c.Fun)
		return
	} // Conversion, not invocation.
	fun := unindex(c.Fun)
	var obj types.Object
	dispatch := "static"
	dynamic := false
	switch f := fun.(type) {
	case *ast.Ident:
		obj = info.Uses[f]
	case *ast.SelectorExpr:
		obj = info.Uses[f.Sel]
		if sel := info.Selections[f]; sel != nil {
			obj = sel.Obj()
			dispatch = "instance"
			t := types.Unalias(sel.Recv())
			if p, ok := t.(*types.Pointer); ok {
				t = types.Unalias(p.Elem())
			}
			if _, ok := t.Underlying().(*types.Interface); ok {
				dispatch = "interface"
				dynamic = true
			}
		}
	default:
		dynamic = true
		dispatch = "unknown"
	}
	if _, ok := obj.(*types.Builtin); ok {
		return
	} // Language primitives are not method edges.
	resolution := "unresolved"
	var targetClass, targetMethod any
	if fn, ok := obj.(*types.Func); ok && !dynamic {
		fn = fn.Origin()
		if target, ok := a.functions[fn]; ok {
			resolution = "project"
			targetClass = target.owner
			targetMethod = target.id
		} else if fn.Pkg() != nil {
			resolution = "external"
			owner := fn.Pkg().Path() + "/@package"
			if sig, ok := fn.Type().(*types.Signature); ok && sig.Recv() != nil {
				if id := typeID(sig.Recv().Type()); id != "" {
					owner = id
				}
				dispatch = "instance"
			}
			targetClass = owner
			targetMethod = owner + "#" + fn.Name()
		}
	} else if obj != nil {
		dynamic = true
	}
	if resolution == "unresolved" {
		a.dynamic = true
		a.partial = true
		if dispatch == "static" {
			dispatch = "unknown"
		}
	}
	_, line, col := a.pos(c)
	var fromMethod any
	if m.id != "" {
		fromMethod = m.id
	}
	a.calls = append(a.calls, object{"from_class": m.owner, "from_method": fromMethod, "to_class": targetClass, "to_method": targetMethod, "target_label": a.code(c.Fun), "dispatch": dispatch, "resolution": resolution, "file": u.Path, "line": line, "column": col, "code": a.code(c)})
}

// ast-decisions-v1: baseline one plus if/for/range, non-default switch/select
// cases, && and ||. max_nesting counts control statements, not lexical blocks.
// Cognitive: each control statement adds one plus current control nesting;
// each logical operator and non-default case adds one. Nested literals are
// included in their enclosing source method (no invented runtime call edge).
// source-maintainability/v1: lexical identifiers/literals are operands; all
// other non-comment tokens except delimiters, dots and semicolons are operators.
// Source lines count lines touched by non-comment tokens within the body.
func (a *analysis) measure(m method) (object, object) {
	cyclomatic, depth, maxDepth, cognitive := 1, 0, 0, 0
	stack := []bool{}
	ast.Inspect(m.decl.Body, func(n ast.Node) bool {
		if n == nil {
			last := stack[len(stack)-1]
			stack = stack[:len(stack)-1]
			if last {
				depth--
			}
			return true
		}
		control := false
		switch n := n.(type) {
		case *ast.IfStmt, *ast.ForStmt, *ast.RangeStmt:
			cyclomatic++
			control = true
		case *ast.SwitchStmt, *ast.TypeSwitchStmt, *ast.SelectStmt:
			control = true
		case *ast.CaseClause:
			if n.List != nil {
				cyclomatic++
				cognitive++
			}
		case *ast.CommClause:
			if n.Comm != nil {
				cyclomatic++
				cognitive++
			}
		case *ast.BinaryExpr:
			if n.Op == token.LAND || n.Op == token.LOR {
				cyclomatic++
				cognitive++
			}
		}
		if control {
			cognitive += 1 + depth
			depth++
			if depth > maxDepth {
				maxDepth = depth
			}
		}
		stack = append(stack, control)
		return true
	})
	start := a.fset.Position(m.decl.Body.Pos()).Offset
	end := a.fset.Position(m.decl.Body.End()).Offset
	content := []byte(m.unit.Content[start:end])
	fs := token.NewFileSet()
	file := fs.AddFile("body", -1, len(content))
	var s scanner.Scanner
	s.Init(file, content, nil, 0)
	operators, operands := 0, 0
	ops, vals, lines := map[string]bool{}, map[string]bool{}, map[int]bool{}
	for {
		p, t, lit := s.Scan()
		if t == token.EOF {
			break
		}
		if t == token.SEMICOLON && lit == "\n" {
			continue
		}
		line := fs.Position(p).Line
		for i := 0; i <= strings.Count(lit, "\n"); i++ {
			lines[line+i] = true
		}
		if t == token.IDENT || t.IsLiteral() {
			operands++
			vals[t.String()+":"+lit] = true
			continue
		}
		switch t {
		case token.LPAREN, token.RPAREN, token.LBRACK, token.RBRACK, token.LBRACE, token.RBRACE, token.COMMA, token.PERIOD, token.SEMICOLON, token.COLON:
			continue
		}
		operators++
		ops[t.String()] = true
	}
	return object{"cyclomatic": cyclomatic, "max_nesting": maxDepth, "definition": "ast-decisions-v1"}, object{"method_id": m.id, "definition": "source-maintainability/v1", "cognitive": cognitive, "distinct_operators": len(ops), "distinct_operands": len(vals), "operators": operators, "operands": operands, "source_lines": len(lines)}
}
func (a *analysis) report() object {
	if a.dynamic {
		a.note("Dynamic or unresolved call sites remain incomplete; runtime targets are not inferred")
	}
	status := "ready"
	if a.partial {
		status = "partial"
	}
	if len(a.units) == 0 && !a.partial {
		status = "not_applicable"
	}
	if a.partial && len(a.diagnostics) == 0 {
		a.diagnostics = append(a.diagnostics, "Dynamic interface or function-value calls remain unresolved; runtime targets are not inferred")
	}
	details := object{"enumeration_complete": a.complete, "definition": "source-types-state/v1", "type_uses": a.typeUses, "class_states": a.states, "cohesion": []object{}, "method_metrics": a.metrics, "inheritance": a.inheritance}
	analyzer := object{"language": "go", "backend": "go-ast-types", "status": status, "files": len(a.units), "version": runtime.Version(), "project": nil, "diagnostics": a.diagnostics}
	return object{"status": status, "java_release": 0, "analyzers": []object{analyzer}, "files": len(a.units), "classes": a.classes, "methods": a.methodRows, "calls": a.calls, "diagnostics": a.diagnostics, "details": details}
}

// Package initialization is real source evidence, but is not fabricated as a
// declared method and therefore has no invented method complexity.
func (a *analysis) globals(u *unit) {
	for _, d := range u.tree.Decls {
		gd, ok := d.(*ast.GenDecl)
		if !ok {
			continue
		}
		for _, spec := range gd.Specs {
			v, ok := spec.(*ast.ValueSpec)
			if !ok {
				continue
			}
			a.addPackage(u, v)
			owner := packageID(u)
			if v.Type != nil {
				a.use(u, owner, "", "field", v.Type)
			}
			for _, expr := range v.Values {
				a.walkBody(method{owner: owner, unit: u}, expr)
			}
		}
	}
}
