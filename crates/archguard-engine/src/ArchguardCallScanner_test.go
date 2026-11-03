package main

import (
	"encoding/json"
	"go/types"
	"strings"
	"testing"
)

func scan(t *testing.T, files ...sourceFile) object {
	t.Helper()
	r := analyze(input{Files: files})
	b, err := json.Marshal(r)
	if err != nil {
		t.Fatal(err)
	}
	var result object
	if err = json.Unmarshal(b, &result); err != nil {
		t.Fatal(err)
	}
	return result
}
func rows(r object, key string) []any { return r[key].([]any) }
func named(r object, key, name string) object {
	for _, v := range rows(r, key) {
		o := v.(map[string]any)
		if o["name"] == name {
			return o
		}
	}
	return nil
}
func TestDeclarationsMetricsCallsAndComposition(t *testing.T) {
	r := scan(t, sourceFile{"api/main.go", `package api
 type Base struct { Value int }
 type Handler struct { Base; count int }
 type Service interface { Run() }
 func (h *Handler) Add(v int) int { // if for && are not decisions
 s := "if && for"; _ = s
 if v > 0 && h.count > 1 { h.count += v }; return h.count
 }
 func (h Handler) Run() { h.Add(1) }
 func Main() { h := Handler{}; h.Run() }
 `, "example/api"})
	if r["status"] != "ready" {
		t.Fatal(r["diagnostics"])
	}
	if len(rows(r, "classes")) != 4 {
		t.Fatal(r["classes"])
	}
	m := named(r, "methods", "Add")
	if m == nil || m["complexity"].(map[string]any)["cyclomatic"] != float64(3) {
		t.Fatal(m)
	}
	if len(rows(r, "calls")) != 2 {
		t.Fatal(r["calls"])
	}
	for _, v := range rows(r, "calls") {
		if v.(map[string]any)["resolution"] != "project" {
			t.Fatal(v)
		}
	}
	d := r["details"].(map[string]any)
	for _, v := range d["inheritance"].([]any) {
		o := v.(map[string]any)
		if o["status"] != "not_applicable" || o["depth"] != nil {
			t.Fatal(o)
		}
	}
	found := false
	for _, v := range d["class_states"].([]any) {
		o := v.(map[string]any)
		if o["class_id"] == "example/api/Handler" {
			found = true
			if len(o["accesses"].([]any)) != 3 {
				t.Fatal(o)
			}
		}
	}
	if !found {
		t.Fatal(d)
	}
}
func TestImportsGenericsAndDynamicCalls(t *testing.T) {
	r := scan(t, sourceFile{"a/a.go", `package a
 type Box[T any] struct { Value T }
 func (b *Box[T]) Get() T { return b.Value }
 func F() {} 
 `, "example/a"}, sourceFile{"b/b.go", `package b
 import "example/a"
 type Box struct {}
 type Reader interface { Read() }
 func F(r Reader, f func()) { a.F(); b:=a.Box[int]{}; b.Get(); r.Read(); f() }
 `, "example/b"})
	if r["status"] != "partial" {
		t.Fatal(r)
	}
	if named(r, "methods", "Get") == nil {
		t.Fatal(r["methods"])
	}
	project, unresolved := 0, 0
	for _, v := range rows(r, "calls") {
		o := v.(map[string]any)
		if o["resolution"] == "project" {
			project++
		}
		if o["resolution"] == "unresolved" {
			unresolved++
			if o["to_method"] != nil {
				t.Fatal(o)
			}
		}
	}
	if project != 2 || unresolved != 2 {
		t.Fatal(r["calls"])
	}
}
func TestMissingDependenciesPreserveMetricsAndShadowedFields(t *testing.T) {
	r := scan(t, sourceFile{"a.go", `package p
 import missing "invalid.example/missing"
 type S struct { x int }
 func (s *S) F() { x:=1; _=x; missing.Call(); for i:=0;i<2;i++ { s.x++ } }
 `, "example/p"})
	if r["status"] != "partial" {
		t.Fatal(r)
	}
	if named(r, "methods", "F")["complexity"].(map[string]any)["cyclomatic"] != float64(2) {
		t.Fatal(r)
	}
	for _, v := range r["details"].(map[string]any)["class_states"].([]any) {
		o := v.(map[string]any)
		if o["class_id"] == "example/p/S" && len(o["accesses"].([]any)) != 1 {
			t.Fatal(o)
		}
	}
}
func TestBuildConstraintsAndBadSyntaxAreExplicit(t *testing.T) {
	r := scan(t, sourceFile{"a.go", "//go:build special\n\npackage p\nfunc F() {}", "example/p"})
	if r["status"] != "partial" || !strings.Contains(strings.Join(toStrings(r["diagnostics"].([]any)), " "), "build") {
		t.Fatal(r)
	}
	r = scan(t, sourceFile{"bad.go", "package p\nfunc F( {", "example/p"})
	if r["status"] != "partial" {
		t.Fatal(r)
	}
}
func toStrings(v []any) []string {
	r := []string{}
	for _, x := range v {
		r = append(r, x.(string))
	}
	return r
}

func TestInterfaceDeclarationsAliasesAndGlobalInitializers(t *testing.T) {
	r := scan(t, sourceFile{"p.go", `package p
 type S struct { Value int }
 type Alias = S
 type Reader interface { Read() int }
 func (s *Alias) Read() int { return s.Value }
 func New() *S { return &S{} }
 var instance = New()
 `, "example/p"})
	if r["status"] != "ready" {
		t.Fatal(r["diagnostics"])
	}
	decls := 0
	for _, v := range rows(r, "methods") {
		o := v.(map[string]any)
		if o["name"] == "Read" {
			decls++
			if o["class_id"] == "example/p/Alias" {
				t.Fatal("alias received a separate identity", o)
			}
		}
	}
	if decls != 2 {
		t.Fatal(r["methods"])
	}
	if len(rows(r, "calls")) != 1 {
		t.Fatal(r["calls"])
	}
	call := rows(r, "calls")[0].(map[string]any)
	if call["resolution"] != "project" || call["from_method"] != nil {
		t.Fatal(call)
	}
}
func TestPromotedFieldsUseEmbeddingStateAndTypeParametersStayDynamic(t *testing.T) {
	r := scan(t, sourceFile{"p.go", `package p
 type Base struct { Value int }
 type S struct { Base }
 func (s *S) F() { s.Value++ }
 type Reader interface { Read() }
 func Generic[T Reader](v T) { v.Read() }
 `, "example/p"})
	for _, v := range r["details"].(map[string]any)["class_states"].([]any) {
		o := v.(map[string]any)
		if o["class_id"] == "example/p/S" {
			if len(o["accesses"].([]any)) != 1 {
				t.Fatal(o)
			}
			if o["accesses"].([]any)[0].(map[string]any)["field"] != "example/p/S.Base" {
				t.Fatal(o)
			}
		}
	}
	if len(rows(r, "calls")) != 1 || rows(r, "calls")[0].(map[string]any)["resolution"] != "unresolved" {
		t.Fatal(r["calls"])
	}
}

func TestRawStringsDoNotCreateDecisionsAndExternalTestsStaySeparate(t *testing.T) {
	r := scan(t, sourceFile{"p.go", "package p\ntype S struct{}\nfunc F() { s := `if x && y { for range z {} }`; _ = s }", "example/p"}, sourceFile{"p_test.go", `package p_test
 import "example/p"
 type S struct{}
 func Check() { p.F() }
 `, "example/p"})
	if r["status"] != "ready" {
		t.Fatal(r["diagnostics"])
	}
	if named(r, "methods", "F")["complexity"].(map[string]any)["cyclomatic"] != float64(1) {
		t.Fatal(r["methods"])
	}
	if len(rows(r, "calls")) != 1 || rows(r, "calls")[0].(map[string]any)["resolution"] != "project" {
		t.Fatal(r["calls"])
	}
}
func TestStandardLibraryCallsUseExternalCompilerEvidence(t *testing.T) {
	r := scan(t, sourceFile{"p.go", `package p
 import "strings"
 func F() int { a:=strings.NewReader("a"); return a.Len() }
 `, "example/p"})
	if r["status"] != "ready" {
		t.Fatal(r["diagnostics"])
	}
	if len(rows(r, "calls")) != 2 {
		t.Fatal(r["calls"])
	}
	for _, v := range rows(r, "calls") {
		if v.(map[string]any)["resolution"] != "external" {
			t.Fatal(v)
		}
	}
}

type recordingImporter struct{ called bool }

func (r *recordingImporter) Import(p string) (*types.Package, error) {
	r.called = true
	return types.NewPackage(p, "external"), nil
}
func TestImporterRejectsExternalAndNoncanonicalPathsBeforeFallback(t *testing.T) {
	for _, p := range []string{"invalid.example/external", "thirdparty", "../strings", "/strings", "strings/../strings", `..\strings`} {
		a := newAnalysis()
		fallback := &recordingImporter{}
		a.standard = fallback
		if _, err := a.Import(p); err == nil || fallback.called {
			t.Fatalf("external import reached SDK fallback: %q, called=%v, error=%v", p, fallback.called, err)
		}
	}
	a := newAnalysis()
	fallback := &recordingImporter{}
	a.standard = fallback
	if _, err := a.Import("strings"); err != nil || !fallback.called {
		t.Fatalf("standard library import denied: %v", err)
	}
}

func TestDuplicateModulePathsKeepDirectoriesSeparateAndImportsAmbiguous(t *testing.T) {
	r := scan(t,
		sourceFile{"one/a.go", `package shared
 type S struct{}
 func (s S) F() { s.G() }
 `, "example/shared"},
		sourceFile{"one/b.go", `package shared
 func (s S) G() {}
 `, "example/shared"},
		sourceFile{"two/a.go", `package shared
 type S struct{}
 func (s S) F() { s.G() }
 func (s S) G() {}
 `, "example/shared"},
		sourceFile{"consumer/c.go", `package consumer
 import "example/shared"
 func F() { s:=shared.S{}; s.F() }
 `, "example/consumer"})
	if r["status"] != "partial" {
		t.Fatal(r)
	}
	classes := map[string]string{}
	for _, v := range rows(r, "classes") {
		o := v.(map[string]any)
		if o["name"] == "S" {
			classes[o["id"].(string)] = o["file"].(string)
		}
	}
	if len(classes) != 2 {
		t.Fatalf("duplicate source declarations collapsed: %v", r["classes"])
	}
	resolved, unresolved := 0, 0
	for _, v := range rows(r, "calls") {
		o := v.(map[string]any)
		if o["file"] == "consumer/c.go" {
			if o["resolution"] != "unresolved" || o["to_class"] != nil {
				t.Fatal("ambiguous import resolved", o)
			}
			unresolved++
		} else {
			if o["resolution"] != "project" || o["from_class"] != o["to_class"] {
				t.Fatal("cross-directory call", o)
			}
			resolved++
		}
	}
	if resolved != 2 || unresolved != 1 {
		t.Fatal(r["calls"])
	}
	for _, v := range rows(r, "methods") {
		o := v.(map[string]any)
		if o["complexity"] == nil {
			t.Fatal("source metrics lost", o)
		}
	}
}
func TestOneDirectoryMultipleFilesRemainOnePackage(t *testing.T) {
	r := scan(t, sourceFile{"p/a.go", `package p
 type S struct{}
 func F() { G() }
 `, "example/p"}, sourceFile{"p/b.go", `package p
 func G() {}
 `, "example/p"})
	if r["status"] != "ready" {
		t.Fatal(r["diagnostics"])
	}
	if len(rows(r, "calls")) != 1 || rows(r, "calls")[0].(map[string]any)["resolution"] != "project" {
		t.Fatal(r["calls"])
	}
	if named(r, "classes", "S")["id"] != "example/p/S" {
		t.Fatal(r["classes"])
	}
}

func TestImplicitGeneratedCompositeLiteralsAndMissingTypesDoNotPanic(t *testing.T) {
	r := scan(t, sourceFile{"generated.go", `package p
 type Descriptor struct { Name string; Values []int }
 var descriptors = []Descriptor{{Name:"first",Values:[]int{1,2}}, {Name:"second"}}
 var byName = map[string]*Descriptor{"first":{Name:"first"}}
 var matrix = [2][2]int{{1,2},{3,4}}
 func F() { _ = []Descriptor{{Name:"local"}} }
 `, "example/p"})
	if r["status"] != "ready" {
		t.Fatal(r["diagnostics"])
	}
	if named(r, "methods", "F")["complexity"] == nil {
		t.Fatal(r["methods"])
	}
	r = scan(t, sourceFile{"missing.go", `package p
 import missing "invalid.example/missing"
 var unresolved=[]missing.Descriptor{{Name:"first"}}
 func F() { _ = map[string]missing.Descriptor{"first":{Name:"first"}} }
 `, "example/p"})
	if r["status"] != "partial" || named(r, "methods", "F")["complexity"] == nil {
		t.Fatal(r)
	}
}

func TestGenericEmbeddingUsesBaseFieldIdentityAndPromotedAccess(t *testing.T) {
	r := scan(t,
		sourceFile{"base/types.go", `package base
 type Box[T any] struct { Value T }
 type Pair[A, B any] struct { First A; Second B }
 `, "example/base"},
		sourceFile{"owner/main.go", `package owner
 import "example/base"
 type LocalBox[T any] struct { Value T }
 type Local struct { LocalBox[int] }
 type Qualified struct { base.Box[int] }
 type Pointer struct { *base.Box[string] }
 type Multiple struct { base.Pair[int, string] }
 func (o Local) Read() int { return o.Value }
 func (o Qualified) Read() int { return o.Value }
 func (o Pointer) Read() string { return o.Value }
 func (o Multiple) Read() int { return o.First }
 `, "example/owner"})
	if r["status"] != "ready" {
		t.Fatal(r["diagnostics"])
	}
	states := r["details"].(map[string]any)["class_states"].([]any)
	for owner, field := range map[string]string{"Local": "LocalBox", "Qualified": "Box", "Pointer": "Box", "Multiple": "Pair"} {
		id := "example/owner/" + owner
		var state map[string]any
		for _, row := range states {
			candidate := row.(map[string]any)
			if candidate["class_id"] == id {
				state = candidate
				break
			}
		}
		if state == nil {
			t.Fatalf("missing %s", id)
		}
		fields := state["fields"].([]any)
		if len(fields) != 1 || fields[0].(map[string]any)["id"] != id+"."+field {
			t.Fatalf("%s: %#v", id, fields)
		}
		accesses := state["accesses"].([]any)
		if len(accesses) != 1 || accesses[0].(map[string]any)["field"] != id+"."+field || accesses[0].(map[string]any)["method"] != id+"#Read" {
			t.Fatalf("%s: promoted access lost: %#v", id, accesses)
		}
	}
}
