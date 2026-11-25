//! The web front-end readers take foreign source: broken input must never panic.
#[path = "../../archguard-flow-common/tests/support/dirs.rs"]
mod dirs;
#[path = "../../archguard-flow-common/tests/support/variants.rs"]
mod variants;

use archguard_flow_web::{ui_flow_angular, ui_flow_react, ui_flow_svelte, ui_flow_vue};

const ROUTES: &str = r#"
import { Routes } from '@angular/router';
import { createBrowserRouter, Route, Link, useNavigate } from 'react-router-dom';
import { createRouter } from 'vue-router';
export const routes: Routes = [
  { path: 'a/:id', component: AComponent, canActivate: [AuthGuard], data: { roles: ['admin'] } },
  { path: 'b', loadChildren: () => import('./b/b.routes').then(m => m.routes) },
];
const router = createBrowserRouter([{ path: '/r', element: <R />, children: [{ path: 'c', element: <C /> }] }]);
function R() { const nav = useNavigate(); return <div onClick={() => nav('/c')}><Link to="/d">é</Link></div>; }
const vr = createRouter({ routes: [{ path: '/v', component: V, meta: { requiresAuth: true } }] });
"#;
const VUE: &str = r#"<template><div @click="go()"><router-link to="/x">x</router-link><button v-if="a">b</button></div></template>
<script setup lang="ts">
import { useRouter } from 'vue-router'
const router = useRouter()
function go() { router.push('/y') }
</script>"#;
const SVELTE: &str = r#"<script>
  import { goto } from '$app/navigation';
  let n = 0;
</script>
<a href="/s">s</a>
<button on:click={() => goto('/t')}>t</button>
{#if n}<p>{n}</p>{/if}"#;
const COMPONENT: &str = r#"
@Component({ selector: 'app-a', templateUrl: './a.component.html' })
export class AComponent { constructor(private router: Router) {} go() { this.router.navigate(['/a', 1]); } }
"#;

#[test]
fn web_extractors_survive_broken_files() {
    let files = [
        (
            "ng/package.json",
            r#"{"dependencies":{"@angular/router":"17"}}"#,
        ),
        ("ng/src/app/app.routes.ts", ROUTES),
        ("ng/src/app/a.component.ts", COMPONENT),
        (
            "ng/src/app/a.component.html",
            "<a routerLink=\"/a\" (click)=\"go()\">a</a><ng-container *ngIf=\"x\"></ng-container>",
        ),
        (
            "react/package.json",
            r#"{"dependencies":{"react-router-dom":"6"}}"#,
        ),
        ("react/src/App.tsx", ROUTES),
        ("vue/package.json", r#"{"dependencies":{"vue-router":"4"}}"#),
        ("vue/src/router.ts", ROUTES),
        ("vue/src/Page.vue", VUE),
        (
            "svelte/package.json",
            r#"{"devDependencies":{"@sveltejs/kit":"2"}}"#,
        ),
        ("svelte/src/routes/+page.svelte", SVELTE),
    ];
    dirs::intact_tree(&files, |root| {
        dirs::assert_flow("ui_flow_angular::extract", &ui_flow_angular::extract(root));
        dirs::assert_flow("ui_flow_react::extract", &ui_flow_react::extract(root));
        dirs::assert_flow("ui_flow_vue::extract", &ui_flow_vue::extract(root));
        dirs::assert_flow("ui_flow_svelte::extract", &ui_flow_svelte::extract(root));
    });
    dirs::each_broken_tree(&files, |root| {
        ui_flow_angular::extract(root);
        ui_flow_react::extract(root);
        ui_flow_vue::extract(root);
        ui_flow_svelte::extract(root);
    });
}
